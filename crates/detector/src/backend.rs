//! Which inference backend a camera runs, and why.
//!
//! Selection lives here rather than at the call site so that one rule holds
//! everywhere: a build reports production detection only when it is actually
//! doing production detection. The configuration that describes a model is
//! feature-independent, so a binary compiled without the ONNX backend can still
//! recognise that a model was configured — and refuse, rather than quietly
//! running the fixture while an operator believes otherwise.

use std::path::PathBuf;

use surveillance_events::event::{DetectionSet, ModelName, ModelVersion};
use surveillance_media::frame::DecodedFrame;

use crate::inference::{DeterministicFakeInference, InferenceError, PersonInference};
use crate::letterbox::LetterboxAlignment;

/// Everything needed to decide and build a backend.
///
/// Present in every build. `model_path` absent means no production model was
/// configured, which is a legitimate deployment state and the shipped one.
#[derive(Clone, Debug)]
pub struct InferenceBackendConfig {
    pub model_path: Option<PathBuf>,
    pub model_sha256: Option<String>,
    /// The camera's own geometry, which is not the frame's any more.
    ///
    /// The decoder now emits frames already scaled and padded to the model's
    /// input, so a letterbox derived from the frame would map detections into
    /// padded space and report coordinates relative to the padding. The mapping
    /// back to what the camera saw has to come from the camera's dimensions.
    pub source_width: u32,
    pub source_height: u32,
    pub input_width: u32,
    pub input_height: u32,
    /// Where the frame producer put the padding, which this side must undo
    /// exactly. Supplied rather than assumed: the two have to agree, and a
    /// default here would let them disagree silently.
    pub letterbox_alignment: LetterboxAlignment,
    pub execution_providers: Vec<String>,
    pub intra_op_threads: u16,
    pub inter_op_threads: u16,
    pub score_threshold_percent: u8,
    pub nms_iou_percent: u8,
}

/// Why a backend could not be selected or built.
#[derive(Debug)]
pub enum BackendError {
    /// A model was configured, but this binary has no ONNX backend compiled in.
    ///
    /// Deliberately fatal. Falling back to the fixture here would let a
    /// deployment that asked for detection get a stream of fabricated results,
    /// which is the single failure this whole design is arranged to prevent.
    OnnxUnavailable(PathBuf),
    /// The fixture backend's own identity is invalid, which is a build defect.
    Fixture,
    /// The ONNX backend refused to come up.
    #[cfg(feature = "onnx-inference")]
    Onnx(crate::onnx::OnnxBackendError),
}

impl core::fmt::Display for BackendError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::OnnxUnavailable(path) => write!(
                formatter,
                "camera.detection.model_path is set to {} but this build has no ONNX backend; \
                 rebuild with the onnx-inference feature or clear the setting",
                path.display()
            ),
            Self::Fixture => formatter.write_str("the fixture backend's identity is invalid"),
            #[cfg(feature = "onnx-inference")]
            Self::Onnx(error) => write!(formatter, "{error}"),
        }
    }
}

impl core::error::Error for BackendError {}

impl BackendError {
    /// Bounded, redacted category suitable for operator logs.
    #[must_use]
    pub const fn category(&self) -> &'static str {
        match self {
            Self::OnnxUnavailable(_) => "onnx_feature_unavailable",
            Self::Fixture => "fixture_identity",
            #[cfg(feature = "onnx-inference")]
            Self::Onnx(error) => match error {
                crate::onnx::OnnxBackendError::UnusableArtifact(_) => "artifact_unreadable",
                crate::onnx::OnnxBackendError::HashMismatch { .. } => "artifact_hash_mismatch",
                crate::onnx::OnnxBackendError::Session(_) => "onnx_session",
                crate::onnx::OnnxBackendError::WarmUp(_) => "model_contract",
                crate::onnx::OnnxBackendError::Identity => "model_identity",
            },
        }
    }
}

/// The inference backend one camera runs.
pub enum PersonInferenceBackend {
    /// Correctly shaped results that say nothing about what the camera saw.
    Fixture(DeterministicFakeInference),
    #[cfg(feature = "onnx-inference")]
    Onnx(Box<crate::onnx::OnnxPersonInference>),
}

impl PersonInferenceBackend {
    /// Chooses and builds the backend this configuration describes.
    ///
    /// # Errors
    /// Returns the reason startup must be refused. There is no success path
    /// that silently downgrades a configured model to the fixture.
    pub fn select(config: &InferenceBackendConfig) -> Result<Self, BackendError> {
        let Some(path) = config.model_path.as_ref() else {
            return DeterministicFakeInference::new()
                .map(Self::Fixture)
                .map_err(|_| BackendError::Fixture);
        };
        #[cfg(feature = "onnx-inference")]
        {
            let _ = path;
            crate::onnx::OnnxPersonInference::load(config)
                .map(|backend| Self::Onnx(Box::new(backend)))
                .map_err(BackendError::Onnx)
        }
        #[cfg(not(feature = "onnx-inference"))]
        {
            Err(BackendError::OnnxUnavailable(path.clone()))
        }
    }
}

impl PersonInference for PersonInferenceBackend {
    fn model_name(&self) -> &ModelName {
        match self {
            Self::Fixture(backend) => backend.model_name(),
            #[cfg(feature = "onnx-inference")]
            Self::Onnx(backend) => backend.model_name(),
        }
    }

    fn model_version(&self) -> &ModelVersion {
        match self {
            Self::Fixture(backend) => backend.model_version(),
            #[cfg(feature = "onnx-inference")]
            Self::Onnx(backend) => backend.model_version(),
        }
    }

    fn infer(&mut self, frame: &DecodedFrame) -> Result<DetectionSet, InferenceError> {
        match self {
            Self::Fixture(backend) => backend.infer(frame),
            #[cfg(feature = "onnx-inference")]
            Self::Onnx(backend) => backend.infer(frame),
        }
    }

    fn is_production_model(&self) -> bool {
        match self {
            Self::Fixture(backend) => backend.is_production_model(),
            #[cfg(feature = "onnx-inference")]
            Self::Onnx(backend) => backend.is_production_model(),
        }
    }
}

/// Explicit mode dispatch; faces never implement the person contract.
pub enum ModeInferenceBackend {
    Person(PersonInferenceBackend),
    #[cfg(feature = "onnx-inference")]
    Face(Box<crate::face::OnnxFaceInference>),
}
impl ModeInferenceBackend {
    /// # Errors
    /// Refuses unavailable or invalid models without silently switching mode.
    pub fn select(
        config: &InferenceBackendConfig,
        mode: crate::inference::DetectionMode,
    ) -> Result<Self, BackendError> {
        match mode {
            crate::inference::DetectionMode::Person => {
                PersonInferenceBackend::select(config).map(Self::Person)
            }
            crate::inference::DetectionMode::PersonFace => Err(BackendError::Fixture),
            crate::inference::DetectionMode::Face => {
                #[cfg(feature = "onnx-inference")]
                {
                    crate::face::OnnxFaceInference::load(config)
                        .map(|model| Self::Face(Box::new(model)))
                        .map_err(BackendError::Onnx)
                }
                #[cfg(not(feature = "onnx-inference"))]
                {
                    Err(BackendError::OnnxUnavailable(
                        config.model_path.clone().unwrap_or_default(),
                    ))
                }
            }
        }
    }
}
impl crate::inference::DetectionInference for ModeInferenceBackend {
    fn output_capacity_dropped(&self) -> u64 {
        match self {
            Self::Person(_) => 0,
            #[cfg(feature = "onnx-inference")]
            Self::Face(face) => face.output_capacity_dropped(),
        }
    }

    fn model_name(&self) -> &ModelName {
        match self {
            Self::Person(p) => PersonInference::model_name(p),
            #[cfg(feature = "onnx-inference")]
            Self::Face(f) => f.model_name(),
        }
    }
    fn model_version(&self) -> &ModelVersion {
        match self {
            Self::Person(p) => PersonInference::model_version(p),
            #[cfg(feature = "onnx-inference")]
            Self::Face(f) => f.model_version(),
        }
    }
    fn mode(&self) -> crate::inference::DetectionMode {
        match self {
            Self::Person(_) => crate::inference::DetectionMode::Person,
            #[cfg(feature = "onnx-inference")]
            Self::Face(_) => crate::inference::DetectionMode::Face,
        }
    }
    fn is_production_model(&self) -> bool {
        match self {
            Self::Person(p) => PersonInference::is_production_model(p),
            #[cfg(feature = "onnx-inference")]
            Self::Face(f) => f.is_production_model(),
        }
    }
    fn infer_detections(
        &mut self,
        frame: &DecodedFrame,
    ) -> Result<crate::inference::InferenceResults, InferenceError> {
        match self {
            Self::Person(p) => p
                .infer(frame)
                .map(crate::inference::InferenceResults::Persons),
            #[cfg(feature = "onnx-inference")]
            Self::Face(f) => f.infer_detections(frame),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{InferenceBackendConfig, PersonInferenceBackend};
    use crate::inference::PersonInference as _;

    fn config(model: Option<&str>) -> InferenceBackendConfig {
        InferenceBackendConfig {
            model_path: model.map(std::path::PathBuf::from),
            model_sha256: None,
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

    /// DI.R6: no configured model is a legitimate state, and it reports itself.
    #[test]
    fn no_configured_model_selects_the_fixture_and_says_so() {
        let backend = PersonInferenceBackend::select(&config(None))
            .unwrap_or_else(|error| panic!("the fixture must always be available: {error}"));
        assert!(
            !backend.is_production_model(),
            "a build with no model must never report production detection"
        );
    }

    /// DI.R6: a configured model that this build cannot run refuses, rather
    /// than quietly producing fixture detections an operator would believe.
    #[cfg(not(feature = "onnx-inference"))]
    #[test]
    fn a_configured_model_without_the_backend_is_refused() {
        let outcome = PersonInferenceBackend::select(&config(Some("/srv/models/yolox.onnx")));
        match outcome {
            Err(super::BackendError::OnnxUnavailable(path)) => {
                assert_eq!(path, std::path::PathBuf::from("/srv/models/yolox.onnx"));
            }
            Err(other) => panic!("wrong refusal: {other}"),
            Ok(_) => panic!("a build with no ONNX backend must not accept a configured model"),
        }
    }

    /// DI.R6: with the backend compiled in, a model that cannot be loaded
    /// refuses startup instead of degrading to the fixture.
    #[cfg(feature = "onnx-inference")]
    #[test]
    fn a_configured_model_that_cannot_be_loaded_is_refused() {
        let missing =
            std::env::temp_dir().join(format!("absent-model-{}.onnx", std::process::id()));
        let _ = std::fs::remove_file(&missing);
        let outcome = PersonInferenceBackend::select(&config(Some(&missing.to_string_lossy())));
        assert!(
            outcome.is_err(),
            "a configured model that is absent must refuse startup"
        );
    }
}
