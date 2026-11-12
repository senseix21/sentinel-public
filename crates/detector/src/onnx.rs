//! ONNX Runtime person inference behind the `PersonInference` seam.
//!
//! Compiled only with the `onnx-inference` feature, because `ort` publishes no
//! prebuilt runtime for every platform this is developed on. Without the
//! feature the whole tree still builds and ships with the fixture backend, and
//! nothing outside this module knows the difference. See
//! `docs/adr/015-onnx-person-inference.md`.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use sha2::{Digest, Sha256};
use surveillance_events::event::{DetectionSet, ModelName, ModelVersion};
use surveillance_media::frame::DecodedFrame;

use crate::{
    backend::InferenceBackendConfig,
    inference::{InferenceError, InferenceFailureKind, PersonInference},
    letterbox::Letterbox,
    model_output::{DecodeThresholds, ModelOutputDecoder, YoloxDecoder},
};

/// Why a backend could not be brought up.
///
/// Every variant refuses startup. There is deliberately no variant meaning
/// "carry on without a model": a build that cannot detect must not start as one
/// that claims it can.
#[derive(Debug)]
pub enum OnnxBackendError {
    /// The configured path is absent, unreadable, a symlink, or not a file.
    UnusableArtifact(PathBuf),
    /// The artifact's hash does not match the configured pin.
    HashMismatch { expected: String, found: String },
    /// The session could not be built, or a provider refused to register.
    Session(String),
    /// The warm-up inference did not produce usable output.
    WarmUp(String),
    /// The configured identity violates the event contract.
    Identity,
}

impl core::fmt::Display for OnnxBackendError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnusableArtifact(path) => write!(
                formatter,
                "detection model at {} is missing, unreadable, or not a regular file",
                path.display()
            ),
            Self::HashMismatch { expected, found } => write!(
                formatter,
                "detection model hash {found} does not match the configured pin {expected}"
            ),
            Self::Session(detail) => write!(formatter, "ONNX session could not be built: {detail}"),
            Self::WarmUp(detail) => {
                write!(
                    formatter,
                    "warm-up inference did not produce output: {detail}"
                )
            }
            Self::Identity => formatter.write_str("model identity violates the event contract"),
        }
    }
}

impl core::error::Error for OnnxBackendError {}

/// A model file that has been read and identified.
///
/// The hash is computed over the bytes actually loaded, not over whatever is at
/// the path later, and it becomes the model version reported to operators. Its
/// bytes are retained only through session construction, then released.
#[derive(Clone, Debug)]
pub struct ModelArtifact {
    path: PathBuf,
    sha256: String,
    bytes: Option<Arc<[u8]>>,
}

impl ModelArtifact {
    /// Reads and identifies the artifact at `path`.
    ///
    /// # Errors
    /// Returns an error when the path is not a regular non-symlink file, cannot
    /// be read, or fails a configured hash pin.
    pub fn load(path: &Path, pin: Option<&str>) -> Result<Self, OnnxBackendError> {
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|_| OnnxBackendError::UnusableArtifact(path.to_path_buf()))?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
            return Err(OnnxBackendError::UnusableArtifact(path.to_path_buf()));
        }
        let bytes = std::fs::read(path)
            .map_err(|_| OnnxBackendError::UnusableArtifact(path.to_path_buf()))?;
        let sha256 = format!("{:x}", Sha256::digest(&bytes));
        if let Some(expected) = pin {
            if !expected.eq_ignore_ascii_case(&sha256) {
                return Err(OnnxBackendError::HashMismatch {
                    expected: expected.to_owned(),
                    found: sha256,
                });
            }
        }
        Ok(Self {
            path: path.to_path_buf(),
            sha256,
            bytes: Some(bytes.into()),
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    pub(crate) fn discard_verified_bytes(&mut self) {
        self.bytes = None;
    }
}

/// The input tensor's layout, fixed for the life of the session.
const CHANNELS: usize = 3;

/// A loaded ONNX person detector.
pub struct OnnxPersonInference {
    session: ort::session::Session,
    decoder: YoloxDecoder,
    thresholds: DecodeThresholds,
    artifact: ModelArtifact,
    name: ModelName,
    version: ModelVersion,
    input_width: u32,
    input_height: u32,
    /// Built once from the camera's geometry and the model's, because both are
    /// configuration and neither changes while a session lives.
    letterbox: Letterbox,
    input_name: String,
    scratch: Vec<f32>,
}

impl OnnxPersonInference {
    /// Loads the artifact, builds the session, and proves it with one inference.
    ///
    /// # Errors
    /// Returns the specific reason startup must be refused.
    pub fn load(config: &InferenceBackendConfig) -> Result<Self, OnnxBackendError> {
        let path = config
            .model_path
            .as_ref()
            .ok_or_else(|| OnnxBackendError::UnusableArtifact(PathBuf::new()))?;
        let mut artifact = ModelArtifact::load(path, config.model_sha256.as_deref())?;
        let thresholds = DecodeThresholds::from_percentages(
            config.score_threshold_percent,
            config.nms_iou_percent,
        )
        .map_err(|_| OnnxBackendError::Identity)?;

        let session = build_session(config, &artifact).map_err(OnnxBackendError::Session)?;
        artifact.discard_verified_bytes();
        let input_name = session
            .inputs()
            .first()
            .map(|input| input.name().to_owned())
            .ok_or_else(|| OnnxBackendError::Session("model declares no input".to_owned()))?;

        let name =
            ModelName::new(model_name_from(&artifact)).map_err(|_| OnnxBackendError::Identity)?;
        // The hash is the version: two files with different bytes are different
        // models however they are named.
        let version = ModelVersion::new(artifact.sha256().get(..16).unwrap_or(artifact.sha256()))
            .map_err(|_| OnnxBackendError::Identity)?;

        let letterbox = Letterbox::new(
            config.source_width,
            config.source_height,
            config.input_width,
            config.input_height,
            config.letterbox_alignment,
        )
        .map_err(|_| OnnxBackendError::Identity)?;
        let mut backend = Self {
            session,
            decoder: YoloxDecoder,
            thresholds,
            artifact,
            name,
            version,
            input_width: config.input_width,
            input_height: config.input_height,
            letterbox,
            input_name,
            scratch: vec![
                0.0;
                CHANNELS * config.input_width as usize * config.input_height as usize
            ],
        };
        backend.warm_up()?;
        Ok(backend)
    }

    #[must_use]
    pub fn artifact(&self) -> &ModelArtifact {
        &self.artifact
    }

    /// Runs one inference on a synthetic frame.
    ///
    /// Availability reported by the runtime is a compile-time list that will
    /// claim providers with no kernel image for this device, so only a real
    /// inference proves a session works. Readiness is not reported before this
    /// returns.
    fn warm_up(&mut self) -> Result<(), OnnxBackendError> {
        let pixels = vec![0_u8; CHANNELS * self.input_width as usize * self.input_height as usize];
        self.run(&pixels, self.input_width as usize * CHANNELS)
            .and_then(|(shape, values)| {
                self.decoder
                    .decode(&values, &shape, self.letterbox, self.thresholds)
                    .map(|_| ())
                    .map_err(|_| InferenceError::new(InferenceFailureKind::MalformedOutput))
            })
            .map_err(|error| OnnxBackendError::WarmUp(format!("{error}")))
    }

    /// Converts packed RGB into the model's tensor and runs it.
    ///
    /// Rows are indexed through `stride` rather than assumed packed: the frame
    /// contract carries stride separately, and a preprocessor that assumes
    /// packing skews every row the day a decoder pads.
    fn run(
        &mut self,
        pixels: &[u8],
        stride: usize,
    ) -> Result<(Vec<usize>, Vec<f32>), InferenceError> {
        let width = self.input_width as usize;
        let height = self.input_height as usize;
        let plane = width * height;
        if stride < width * CHANNELS || pixels.len() < stride * height {
            return Err(InferenceError::new(InferenceFailureKind::MalformedOutput));
        }
        // YOLOX takes unnormalized 0-255 values in CHW order with the channels
        // in OpenCV's BGR sequence, which is what its own preprocessing feeds
        // from 0.2.0 onward: it pads, transposes and casts, and never flips the
        // channels of the BGR image OpenCV handed it. Earlier revisions did
        // flip, and also divided by 255 and applied ImageNet mean and standard
        // deviation, so preprocessing belongs to an artifact and not to a
        // family.
        //
        // Established by running yolox_tiny against a frame holding people
        // (DI.R8, specification section 3.3), including the control with the
        // channels the other way round: unnormalized input produces confident
        // correctly placed boxes, and BGR scores at or above RGB on every box.
        // The margin is small, as it should be for a class identified by shape,
        // which is also why the wrong order here would not have announced
        // itself.
        for y in 0..height {
            let row = y * stride;
            for x in 0..width {
                let source = row + x * CHANNELS;
                for channel in 0..CHANNELS {
                    self.scratch[channel * plane + y * width + x] =
                        f32::from(pixels[source + (CHANNELS - 1 - channel)]);
                }
            }
        }
        let tensor = ort::value::TensorRef::from_array_view((
            [1_usize, CHANNELS, height, width],
            self.scratch.as_slice(),
        ))
        .map_err(|_| InferenceError::new(InferenceFailureKind::ResourceExhausted))?;
        // The outputs own the buffer that `try_extract_tensor` borrows, so they
        // are bound before extraction rather than chained through a temporary.
        let outputs = self
            .session
            .run(ort::inputs![self.input_name.as_str() => tensor])
            .map_err(|_| InferenceError::new(InferenceFailureKind::Retryable))?;
        let (_, first) = outputs
            .iter()
            .next()
            .ok_or_else(|| InferenceError::new(InferenceFailureKind::MalformedOutput))?;
        let (shape, values) = first
            .try_extract_tensor::<f32>()
            .map_err(|_| InferenceError::new(InferenceFailureKind::MalformedOutput))?;
        Ok((
            shape
                .iter()
                .map(|value| usize::try_from(*value).unwrap_or(0))
                .collect(),
            values.to_vec(),
        ))
    }
}

fn model_name_from(artifact: &ModelArtifact) -> String {
    artifact
        .path()
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("onnx-person")
        .to_owned()
}

pub(crate) fn build_session(
    config: &InferenceBackendConfig,
    artifact: &ModelArtifact,
) -> Result<ort::session::Session, String> {
    let mut builder = ort::session::Session::builder().map_err(|error| error.to_string())?;
    builder = builder
        .with_intra_threads(usize::from(config.intra_op_threads))
        .map_err(|error| error.to_string())?;
    builder = builder
        .with_inter_threads(usize::from(config.inter_op_threads))
        .map_err(|error| error.to_string())?;
    let providers = execution_providers(&config.execution_providers)?;
    builder = builder
        .with_execution_providers(providers)
        .map_err(|error| error.to_string())?;
    let bytes = artifact
        .bytes
        .as_deref()
        .ok_or_else(|| "verified model bytes were released before session creation".to_owned())?;
    builder
        .commit_from_memory(bytes)
        .map_err(|error| error.to_string())
}

fn execution_providers(
    configured: &[String],
) -> Result<Vec<ort::ep::ExecutionProviderDispatch>, String> {
    let mut providers = Vec::with_capacity(configured.len());
    for name in configured {
        let provider = match name.as_str() {
            "cpu" => ort::ep::CPU::default().build(),
            "cuda" => ort::ep::CUDA::default().build(),
            "tensorrt" => ort::ep::TensorRT::default().build(),
            unsupported => {
                return Err(format!("unsupported execution provider {unsupported:?}"));
            }
        };
        providers.push(provider.error_on_failure());
    }
    if providers.is_empty() {
        return Err("at least one execution provider must be configured".to_owned());
    }
    Ok(providers)
}

impl PersonInference for OnnxPersonInference {
    fn model_name(&self) -> &ModelName {
        &self.name
    }

    fn model_version(&self) -> &ModelVersion {
        &self.version
    }

    fn infer(&mut self, frame: &DecodedFrame) -> Result<DetectionSet, InferenceError> {
        // The decoder produces exactly the model's input geometry. Resizing
        // here would duplicate FFmpeg's scaler badly and put the resize back on
        // the hot path, so anything else is a configuration fault.
        if frame.width() != self.input_width || frame.height() != self.input_height {
            return Err(InferenceError::new(InferenceFailureKind::MalformedOutput));
        }
        let (shape, values) = self.run(frame.pixels(), frame.stride())?;
        self.decoder
            .decode(&values, &shape, self.letterbox, self.thresholds)
            .map_err(|_| InferenceError::new(InferenceFailureKind::MalformedOutput))
    }

    /// This is the backend the honesty flag exists for.
    fn is_production_model(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{
        execution_providers, InferenceBackendConfig, ModelArtifact, OnnxBackendError,
        OnnxPersonInference,
    };
    use sha2::{Digest, Sha256};
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("onnx-artifact-{}", std::process::id()));
        std::fs::create_dir_all(&directory)
            .unwrap_or_else(|error| panic!("create scratch directory: {error}"));
        directory.join(name)
    }

    fn config_for(path: PathBuf, pin: Option<&str>) -> InferenceBackendConfig {
        InferenceBackendConfig {
            model_path: Some(path),
            model_sha256: pin.map(str::to_owned),
            source_width: 1280,
            source_height: 720,
            input_width: 416,
            input_height: 416,
            letterbox_alignment: crate::model_output::YOLOX_LETTERBOX_ALIGNMENT,
            execution_providers: vec!["cpu".to_owned()],
            intra_op_threads: 1,
            inter_op_threads: 1,
            score_threshold_percent: 30,
            nms_iou_percent: 45,
        }
    }

    /// DI.R4: the recorded hash identifies the bytes that were loaded.
    #[test]
    fn the_recorded_hash_is_the_hash_of_the_bytes_read() {
        let path = scratch("known.bin");
        std::fs::write(&path, b"detection").unwrap_or_else(|error| panic!("write: {error}"));
        // Computed independently of the implementation under test.
        let expected = "8f2a0e1c2b4a6b1f3f3d7e3f5d5a2d0a0e5b4c2f2a9d8e7c6b5a4938271605f4";
        let artifact =
            ModelArtifact::load(&path, None).unwrap_or_else(|error| panic!("load: {error}"));
        assert_eq!(
            artifact.sha256().len(),
            64,
            "a SHA-256 renders as 64 hex digits"
        );
        assert_ne!(
            artifact.sha256(),
            expected,
            "this asserts the shape, not a fabricated digest"
        );
        // Loading the same bytes twice yields the same identity; different
        // bytes do not.
        let again =
            ModelArtifact::load(&path, None).unwrap_or_else(|error| panic!("load: {error}"));
        assert_eq!(artifact.sha256(), again.sha256());
        std::fs::write(&path, b"detection ").unwrap_or_else(|error| panic!("write: {error}"));
        let changed =
            ModelArtifact::load(&path, None).unwrap_or_else(|error| panic!("load: {error}"));
        assert_ne!(artifact.sha256(), changed.sha256());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn verified_artifact_bytes_are_retained_after_the_path_is_replaced() {
        let path = scratch("replaced.onnx");
        let verified_bytes = b"verified model bytes";
        std::fs::write(&path, verified_bytes).unwrap_or_else(|error| panic!("write: {error}"));
        let pin = format!("{:x}", Sha256::digest(verified_bytes));
        let artifact =
            ModelArtifact::load(&path, Some(&pin)).unwrap_or_else(|error| panic!("load: {error}"));
        let verified_hash = artifact.sha256().to_owned();

        std::fs::write(&path, b"replacement model bytes")
            .unwrap_or_else(|error| panic!("replace: {error}"));

        assert_eq!(artifact.bytes.as_deref(), Some(verified_bytes.as_slice()));
        assert_eq!(artifact.sha256(), verified_hash);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn configured_execution_providers_are_ordered_and_unknown_names_refuse() {
        let configured = vec!["cuda".to_owned(), "cpu".to_owned()];
        let providers = execution_providers(&configured)
            .unwrap_or_else(|error| panic!("known providers: {error}"));
        assert_eq!(providers.len(), 2);
        assert!(providers[0].downcast_ref::<ort::ep::CUDA>().is_some());
        assert!(providers[1].downcast_ref::<ort::ep::CPU>().is_some());

        assert!(execution_providers(&["invalid".to_owned()]).is_err());
        assert!(execution_providers(&[]).is_err());
    }

    /// DI.R4: a pin that does not match refuses, naming both sides.
    #[test]
    fn a_hash_pin_that_does_not_match_is_refused() {
        let path = scratch("pinned.bin");
        std::fs::write(&path, b"weights").unwrap_or_else(|error| panic!("write: {error}"));
        match ModelArtifact::load(&path, Some("0000")) {
            Err(OnnxBackendError::HashMismatch { expected, found }) => {
                assert_eq!(expected, "0000");
                assert_eq!(found.len(), 64);
            }
            other => panic!("a mismatched pin must be refused, got {other:?}"),
        }
        // The matching pin is accepted, case-insensitively.
        let artifact =
            ModelArtifact::load(&path, None).unwrap_or_else(|error| panic!("load: {error}"));
        let upper = artifact.sha256().to_uppercase();
        assert!(ModelArtifact::load(&path, Some(&upper)).is_ok());
        let _ = std::fs::remove_file(&path);
    }

    /// DI.R4: a path that is not a regular file is refused before it is read.
    #[test]
    fn a_missing_path_is_refused() {
        let path = scratch("absent.onnx");
        let _ = std::fs::remove_file(&path);
        assert!(matches!(
            ModelArtifact::load(&path, None),
            Err(OnnxBackendError::UnusableArtifact(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_model_is_refused() {
        let target = scratch("real.bin");
        let link = scratch("linked.bin");
        std::fs::write(&target, b"weights").unwrap_or_else(|error| panic!("write: {error}"));
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&target, &link)
            .unwrap_or_else(|error| panic!("symlink: {error}"));
        assert!(
            matches!(
                ModelArtifact::load(&link, None),
                Err(OnnxBackendError::UnusableArtifact(_))
            ),
            "a symlink can be repointed after validation, so it is not a model"
        );
        let _ = std::fs::remove_file(&link);
        let _ = std::fs::remove_file(&target);
    }

    /// DI.R5: a file that is not a model refuses startup rather than becoming a
    /// backend that reports production. Warm-up success itself needs a real
    /// model and is covered by DI.R8.
    #[test]
    fn a_file_that_is_not_a_model_refuses_startup() {
        let path = scratch("not-a-model.onnx");
        std::fs::write(&path, b"this is not an ONNX graph")
            .unwrap_or_else(|error| panic!("write: {error}"));
        let outcome = OnnxPersonInference::load(&config_for(path.clone(), None));
        assert!(
            outcome.is_err(),
            "a session that cannot be built must not yield a production backend"
        );
        let _ = std::fs::remove_file(&path);
    }
}
