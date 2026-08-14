//! Narrow inference substitution boundary and deterministic fixture adapter.

use std::{error::Error, fmt};
use surveillance_events::event::{
    Confidence, DetectionSet, ModelName, ModelVersion, NormalizedBoundingBox, PersonDetection,
};
use surveillance_media::frame::DecodedFrame;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InferenceFailureKind {
    Retryable,
    Terminal,
    MalformedOutput,
    Unavailable,
    ResourceExhausted,
    Timeout,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InferenceError {
    kind: InferenceFailureKind,
}
impl InferenceError {
    #[must_use]
    pub const fn new(kind: InferenceFailureKind) -> Self {
        Self { kind }
    }
    #[must_use]
    pub const fn kind(&self) -> InferenceFailureKind {
        self.kind
    }
}
impl fmt::Display for InferenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "inference failed: {:?}", self.kind)
    }
}
impl Error for InferenceError {}

pub trait PersonInference: Send + 'static {
    fn model_name(&self) -> &ModelName;
    fn model_version(&self) -> &ModelVersion;
    /// # Errors
    /// Returns a typed model/device/output failure.
    fn infer(&mut self, frame: &DecodedFrame) -> Result<DetectionSet, InferenceError>;
}

/// Test/local qualification adapter. It emits one deterministic person when the
/// first RGB byte is odd and none when it is even.
pub struct DeterministicFakeInference {
    name: ModelName,
    version: ModelVersion,
}
impl DeterministicFakeInference {
    /// # Errors
    /// Returns an error only if the built-in model identity violates event invariants.
    pub fn new() -> Result<Self, surveillance_events::event::EventValidationError> {
        Ok(Self {
            name: ModelName::new("deterministic-fake-person")?,
            version: ModelVersion::new("1.0.0")?,
        })
    }
}
impl PersonInference for DeterministicFakeInference {
    fn model_name(&self) -> &ModelName {
        &self.name
    }
    fn model_version(&self) -> &ModelVersion {
        &self.version
    }
    fn infer(&mut self, frame: &DecodedFrame) -> Result<DetectionSet, InferenceError> {
        let detected = frame.pixels().first().is_some_and(|value| value % 2 == 1);
        if !detected {
            return DetectionSet::new(Vec::new())
                .map_err(|_| InferenceError::new(InferenceFailureKind::MalformedOutput));
        }
        let confidence = Confidence::new(0.9)
            .map_err(|_| InferenceError::new(InferenceFailureKind::MalformedOutput))?;
        let bounding_box = NormalizedBoundingBox::new(0.25, 0.2, 0.5, 0.7)
            .map_err(|_| InferenceError::new(InferenceFailureKind::MalformedOutput))?;
        DetectionSet::new(vec![PersonDetection {
            confidence,
            bounding_box,
        }])
        .map_err(|_| InferenceError::new(InferenceFailureKind::MalformedOutput))
    }
}
