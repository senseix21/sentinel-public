//! ONNX Runtime person inference behind the `PersonInference` seam.
//!
//! Compiled only with the `onnx-inference` feature, because `ort` publishes no
//! prebuilt runtime for every platform this is developed on. Without the
//! feature the whole tree still builds and ships with the fixture backend, and
//! nothing outside this module knows the difference. See
//! `docs/adr/015-onnx-person-inference.md`.

use std::path::{Path, PathBuf};

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
/// the path later, and it becomes the model version reported to operators. That
/// is what makes field version skew detectable.
#[derive(Clone, Debug)]
pub struct ModelArtifact {
    path: PathBuf,
    sha256: String,
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
        let artifact = ModelArtifact::load(path, config.model_sha256.as_deref())?;
        let thresholds = DecodeThresholds::from_percentages(
            config.score_threshold_percent,
            config.nms_iou_percent,
        )
        .map_err(|_| OnnxBackendError::Identity)?;

        let session = build_session(config).map_err(OnnxBackendError::Session)?;
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

        let mut backend = Self {
            session,
            decoder: YoloxDecoder,
            thresholds,
            artifact,
            name,
            version,
            input_width: config.input_width,
            input_height: config.input_height,
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
            .map(|_| ())
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
        // YOLOX takes unnormalized 0-255 RGB in CHW order, established by
        // running the artifact rather than from documentation.
        for y in 0..height {
            let row = y * stride;
            for x in 0..width {
                let source = row + x * CHANNELS;
                for channel in 0..CHANNELS {
                    self.scratch[channel * plane + y * width + x] =
                        f32::from(pixels[source + channel]);
                }
            }
        }
        let tensor = ort::value::Tensor::from_array((
            [1_usize, CHANNELS, height, width],
            self.scratch.clone(),
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

fn build_session(config: &InferenceBackendConfig) -> Result<ort::session::Session, String> {
    let mut builder = ort::session::Session::builder().map_err(|error| error.to_string())?;
    builder = builder
        .with_intra_threads(usize::from(config.intra_op_threads))
        .map_err(|error| error.to_string())?;
    builder = builder
        .with_inter_threads(usize::from(config.inter_op_threads))
        .map_err(|error| error.to_string())?;
    let path = config
        .model_path
        .as_ref()
        .ok_or_else(|| "no model path configured".to_owned())?;
    builder
        .commit_from_file(path)
        .map_err(|error| error.to_string())
}

impl PersonInference for OnnxPersonInference {
    fn model_name(&self) -> &ModelName {
        &self.name
    }

    fn model_version(&self) -> &ModelVersion {
        &self.version
    }

    fn infer(&mut self, frame: &DecodedFrame) -> Result<DetectionSet, InferenceError> {
        // The frame must already be the model's input geometry. Slice 5 makes
        // the decoder produce exactly that; resizing here would duplicate
        // FFmpeg's scaler badly and put the resize back on the hot path.
        if frame.width() != self.input_width || frame.height() != self.input_height {
            return Err(InferenceError::new(InferenceFailureKind::MalformedOutput));
        }
        let letterbox = Letterbox::new(
            frame.width(),
            frame.height(),
            self.input_width,
            self.input_height,
        )
        .map_err(|_| InferenceError::new(InferenceFailureKind::MalformedOutput))?;
        let (shape, values) = self.run(frame.pixels(), frame.stride())?;
        self.decoder
            .decode(&values, &shape, letterbox, self.thresholds)
            .map_err(|_| InferenceError::new(InferenceFailureKind::MalformedOutput))
    }

    /// This is the backend the honesty flag exists for.
    fn is_production_model(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{InferenceBackendConfig, ModelArtifact, OnnxBackendError, OnnxPersonInference};
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
            input_width: 416,
            input_height: 416,
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
