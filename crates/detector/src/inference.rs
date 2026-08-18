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

    /// Whether this implementation performs real inference on the frame.
    ///
    /// `false` means the detections it emits are a fixture: correctly shaped,
    /// correctly timestamped, camera-scoped, and *meaningless* as a statement
    /// about what was in front of the camera. Nothing downstream can tell the
    /// difference by inspecting a `DetectionSet`, which is precisely why this
    /// has to be asked rather than inferred.
    ///
    /// Defaulted to `false` deliberately. An implementation that does not
    /// answer is presumed not to be production, because over-claiming here
    /// produces a UI that reports people who were never there, and that failure
    /// is far worse than under-claiming.
    fn is_production_model(&self) -> bool {
        false
    }
}

/// What an inference implementation is, for reporting to operators.
///
/// Exists so that "detection is a fixture" is a value the API can carry rather
/// than a sentence in a document nobody reads before shipping a person count.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InferenceDescriptor {
    model_name: String,
    model_version: String,
    is_production: bool,
}

impl InferenceDescriptor {
    /// Describes a live implementation by asking it.
    #[must_use]
    pub fn of(inference: &dyn PersonInference) -> Self {
        Self {
            model_name: inference.model_name().as_str().to_owned(),
            model_version: inference.model_version().as_str().to_owned(),
            is_production: inference.is_production_model(),
        }
    }

    #[must_use]
    pub fn model_name(&self) -> &str {
        &self.model_name
    }

    #[must_use]
    pub fn model_version(&self) -> &str {
        &self.model_version
    }

    /// Whether the described implementation performs real inference.
    #[must_use]
    pub const fn is_production(&self) -> bool {
        self.is_production
    }
}

/// Describes the inference implementation this build wires into the pipeline.
///
/// A build fact, not a runtime one: there is exactly one implementation of
/// [`PersonInference`] in the workspace. When a real model lands this must
/// change with it, and `the_compiled_backend_matches_what_the_pipeline_builds`
/// in `apps/server` fails if it does not.
///
/// # Errors
/// Returns an error only if the built-in model identity violates event
/// invariants, which is the same condition
/// [`DeterministicFakeInference::new`] reports.
pub fn compiled_backend(
) -> Result<InferenceDescriptor, surveillance_events::event::EventValidationError> {
    Ok(InferenceDescriptor::of(&DeterministicFakeInference::new()?))
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
