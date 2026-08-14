//! Immutable, versioned domain-event contracts.

use serde::{Deserialize, Serialize};
use std::{error::Error, fmt, num::NonZeroU64};
use surveillance_common::types::{CameraId, RecordingId};

pub const CURRENT_SCHEMA_VERSION: u16 = 1;
pub const MAX_DETECTIONS_PER_EVENT: usize = 64;

macro_rules! validated_text {
    ($name:ident, $label:literal, $max:expr) => {
        #[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);
        impl $name {
            /// Validates bounded stable text.
            ///
            /// # Errors
            /// Returns a validation error for empty, oversized, or unsafe text.
            pub fn new(value: impl Into<String>) -> Result<Self, EventValidationError> {
                let value = value.into();
                if value.is_empty() {
                    return Err(EventValidationError::Empty($label));
                }
                if value.len() > $max
                    || !value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
                {
                    return Err(EventValidationError::InvalidText($label));
                }
                Ok(Self(value))
            }
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl TryFrom<String> for $name {
            type Error = EventValidationError;
            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }
        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

validated_text!(EventId, "event ID", 64);
validated_text!(ModelName, "model name", 64);
validated_text!(ModelVersion, "model version", 64);
validated_text!(FailureCategory, "failure category", 64);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "u16", into = "u16")]
pub struct SchemaVersion(u16);
impl SchemaVersion {
    /// # Errors
    /// Returns an error unless the version is the supported current version.
    pub fn new(value: u16) -> Result<Self, EventValidationError> {
        if value == CURRENT_SCHEMA_VERSION {
            Ok(Self(value))
        } else {
            Err(EventValidationError::UnsupportedSchemaVersion(value))
        }
    }
    #[must_use]
    pub const fn current() -> Self {
        Self(CURRENT_SCHEMA_VERSION)
    }
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}
impl TryFrom<u16> for SchemaVersion {
    type Error = EventValidationError;
    fn try_from(v: u16) -> Result<Self, Self::Error> {
        Self::new(v)
    }
}
impl From<SchemaVersion> for u16 {
    fn from(v: SchemaVersion) -> Self {
        v.0
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "i64", into = "i64")]
pub struct OccurredAtUnixMicros(i64);
impl OccurredAtUnixMicros {
    /// # Errors
    /// Returns an error for timestamps at or before the Unix epoch.
    pub fn new(value: i64) -> Result<Self, EventValidationError> {
        if value > 0 {
            Ok(Self(value))
        } else {
            Err(EventValidationError::InvalidOccurredAt)
        }
    }
    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }
}
impl TryFrom<i64> for OccurredAtUnixMicros {
    type Error = EventValidationError;
    fn try_from(v: i64) -> Result<Self, Self::Error> {
        Self::new(v)
    }
}
impl From<OccurredAtUnixMicros> for i64 {
    fn from(v: OccurredAtUnixMicros) -> Self {
        v.0
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SourceMediaTime {
    pub epoch: NonZeroU64,
    pub ticks: i64,
    pub timebase_hz: NonZeroU64,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(try_from = "f32", into = "f32")]
pub struct Confidence(f32);
impl Confidence {
    /// # Errors
    /// Returns an error unless confidence is finite and within `0..=1`.
    pub fn new(value: f32) -> Result<Self, EventValidationError> {
        if value.is_finite() && (0.0..=1.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(EventValidationError::InvalidConfidence)
        }
    }
    #[must_use]
    pub const fn get(self) -> f32 {
        self.0
    }
}
impl TryFrom<f32> for Confidence {
    type Error = EventValidationError;
    fn try_from(v: f32) -> Result<Self, Self::Error> {
        Self::new(v)
    }
}
impl From<Confidence> for f32 {
    fn from(v: Confidence) -> Self {
        v.0
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(try_from = "[f32; 4]", into = "[f32; 4]")]
pub struct NormalizedBoundingBox {
    left: f32,
    top: f32,
    width: f32,
    height: f32,
}
impl TryFrom<[f32; 4]> for NormalizedBoundingBox {
    type Error = EventValidationError;
    fn try_from(value: [f32; 4]) -> Result<Self, Self::Error> {
        Self::new(value[0], value[1], value[2], value[3])
    }
}
impl From<NormalizedBoundingBox> for [f32; 4] {
    fn from(value: NormalizedBoundingBox) -> Self {
        [value.left, value.top, value.width, value.height]
    }
}
impl NormalizedBoundingBox {
    /// # Errors
    /// Returns an error for non-finite, empty, or out-of-frame geometry.
    pub fn new(left: f32, top: f32, width: f32, height: f32) -> Result<Self, EventValidationError> {
        let values = [left, top, width, height];
        if !values.iter().all(|v| v.is_finite())
            || left < 0.0
            || top < 0.0
            || width <= 0.0
            || height <= 0.0
            || left + width > 1.0
            || top + height > 1.0
        {
            return Err(EventValidationError::InvalidGeometry);
        }
        Ok(Self {
            left,
            top,
            width,
            height,
        })
    }
    #[must_use]
    pub const fn left(self) -> f32 {
        self.left
    }
    #[must_use]
    pub const fn top(self) -> f32 {
        self.top
    }
    #[must_use]
    pub const fn width(self) -> f32 {
        self.width
    }
    #[must_use]
    pub const fn height(self) -> f32 {
        self.height
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PersonDetection {
    pub confidence: Confidence,
    pub bounding_box: NormalizedBoundingBox,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(try_from = "Vec<PersonDetection>", into = "Vec<PersonDetection>")]
pub struct DetectionSet(Vec<PersonDetection>);
impl DetectionSet {
    /// # Errors
    /// Returns an error when the bounded result count is exceeded.
    pub fn new(values: Vec<PersonDetection>) -> Result<Self, EventValidationError> {
        if values.len() > MAX_DETECTIONS_PER_EVENT {
            Err(EventValidationError::TooManyDetections)
        } else {
            Ok(Self(values))
        }
    }
    #[must_use]
    pub fn as_slice(&self) -> &[PersonDetection] {
        &self.0
    }
}
impl TryFrom<Vec<PersonDetection>> for DetectionSet {
    type Error = EventValidationError;
    fn try_from(values: Vec<PersonDetection>) -> Result<Self, Self::Error> {
        Self::new(values)
    }
}
impl From<DetectionSet> for Vec<PersonDetection> {
    fn from(values: DetectionSet) -> Self {
        values.0
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CameraState {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
    Stopped,
    Terminal,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectorState {
    Disabled,
    Starting,
    Running,
    Degraded,
    Terminal,
    Stopping,
    Stopped,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StoragePressure {
    Warning,
    Critical,
    Recovered,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventPayload {
    CameraStateChanged {
        state: CameraState,
    },
    RecordingSegmentCompleted {
        recording_id: RecordingId,
        size_bytes: u64,
    },
    RecordingSegmentFailed {
        recording_id: RecordingId,
        failure_category: FailureCategory,
    },
    StoragePressureChanged {
        pressure: StoragePressure,
    },
    PersonDetectionObserved {
        media_time: SourceMediaTime,
        model: ModelName,
        model_version: ModelVersion,
        detections: DetectionSet,
    },
    DetectorStateChanged {
        state: DetectorState,
        failure_category: Option<FailureCategory>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryClass {
    CriticalLifecycle,
    LatestState,
    Telemetry,
    Detection,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventKind {
    CameraStateChanged,
    RecordingSegmentCompleted,
    RecordingSegmentFailed,
    StoragePressureChanged,
    PersonDetectionObserved,
    DetectorStateChanged,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(try_from = "DomainEventWire")]
pub struct DomainEvent {
    pub event_id: EventId,
    pub schema_version: SchemaVersion,
    pub occurred_at: OccurredAtUnixMicros,
    pub camera_id: Option<CameraId>,
    pub delivery_class: DeliveryClass,
    pub payload: EventPayload,
}

#[derive(Deserialize)]
struct DomainEventWire {
    event_id: EventId,
    schema_version: SchemaVersion,
    occurred_at: OccurredAtUnixMicros,
    camera_id: Option<CameraId>,
    delivery_class: DeliveryClass,
    payload: EventPayload,
}

impl TryFrom<DomainEventWire> for DomainEvent {
    type Error = EventValidationError;
    fn try_from(value: DomainEventWire) -> Result<Self, Self::Error> {
        if value.schema_version != SchemaVersion::current() {
            return Err(EventValidationError::UnsupportedSchemaVersion(
                value.schema_version.get(),
            ));
        }
        Self::new(
            value.event_id,
            value.occurred_at,
            value.camera_id,
            value.delivery_class,
            value.payload,
        )
    }
}
impl DomainEvent {
    /// # Errors
    /// Returns an error when payload scope and delivery class are inconsistent.
    pub fn new(
        event_id: EventId,
        occurred_at: OccurredAtUnixMicros,
        camera_id: Option<CameraId>,
        delivery_class: DeliveryClass,
        payload: EventPayload,
    ) -> Result<Self, EventValidationError> {
        if let EventPayload::PersonDetectionObserved { .. } = &payload {
            if camera_id.is_none() {
                return Err(EventValidationError::DetectionWithoutCamera);
            }
            if delivery_class != DeliveryClass::Detection {
                return Err(EventValidationError::WrongDeliveryClass);
            }
        }
        Ok(Self {
            event_id,
            schema_version: SchemaVersion::current(),
            occurred_at,
            camera_id,
            delivery_class,
            payload,
        })
    }
    #[must_use]
    pub fn state_key(&self) -> Option<(Option<&CameraId>, &'static str)> {
        let kind = match self.payload {
            EventPayload::CameraStateChanged { .. } => "camera",
            EventPayload::DetectorStateChanged { .. } => "detector",
            EventPayload::StoragePressureChanged { .. } => "storage",
            _ => return None,
        };
        Some((self.camera_id.as_ref(), kind))
    }
    #[must_use]
    pub const fn kind(&self) -> EventKind {
        match self.payload {
            EventPayload::CameraStateChanged { .. } => EventKind::CameraStateChanged,
            EventPayload::RecordingSegmentCompleted { .. } => EventKind::RecordingSegmentCompleted,
            EventPayload::RecordingSegmentFailed { .. } => EventKind::RecordingSegmentFailed,
            EventPayload::StoragePressureChanged { .. } => EventKind::StoragePressureChanged,
            EventPayload::PersonDetectionObserved { .. } => EventKind::PersonDetectionObserved,
            EventPayload::DetectorStateChanged { .. } => EventKind::DetectorStateChanged,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventValidationError {
    Empty(&'static str),
    InvalidText(&'static str),
    UnsupportedSchemaVersion(u16),
    InvalidOccurredAt,
    InvalidConfidence,
    InvalidGeometry,
    TooManyDetections,
    DetectionWithoutCamera,
    WrongDeliveryClass,
}
impl fmt::Display for EventValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid domain event: {self:?}")
    }
}
impl Error for EventValidationError {}

#[cfg(test)]
mod tests {
    use super::{
        Confidence, DeliveryClass, DetectionSet, DomainEvent, EventId, EventPayload,
        EventValidationError, ModelName, ModelVersion, NormalizedBoundingBox, OccurredAtUnixMicros,
        PersonDetection, SourceMediaTime, MAX_DETECTIONS_PER_EVENT,
    };
    use std::num::NonZeroU64;
    use surveillance_common::types::CameraId;

    fn person() -> PersonDetection {
        PersonDetection {
            confidence: Confidence::new(0.75).unwrap_or_else(|error| panic!("confidence: {error}")),
            bounding_box: NormalizedBoundingBox::new(0.1, 0.2, 0.3, 0.4)
                .unwrap_or_else(|error| panic!("box: {error}")),
        }
    }

    #[test]
    fn rejects_nonfinite_confidence_invalid_geometry_and_excess_results() {
        assert_eq!(
            Confidence::new(f32::NAN),
            Err(EventValidationError::InvalidConfidence)
        );
        assert_eq!(
            NormalizedBoundingBox::new(0.8, 0.0, 0.3, 0.5),
            Err(EventValidationError::InvalidGeometry)
        );
        assert_eq!(
            DetectionSet::new(vec![person(); MAX_DETECTIONS_PER_EVENT + 1]),
            Err(EventValidationError::TooManyDetections)
        );
    }

    #[test]
    fn person_event_round_trips_with_stable_versioned_json() {
        let event = DomainEvent::new(
            EventId::new("event-001").unwrap_or_else(|error| panic!("ID: {error}")),
            OccurredAtUnixMicros::new(1_700_000_000_000_000)
                .unwrap_or_else(|error| panic!("time: {error}")),
            Some(CameraId::new("camera-01").unwrap_or_else(|error| panic!("camera: {error}"))),
            DeliveryClass::Detection,
            EventPayload::PersonDetectionObserved {
                media_time: SourceMediaTime {
                    epoch: NonZeroU64::new(2).unwrap_or_else(|| panic!("epoch")),
                    ticks: 90_000,
                    timebase_hz: NonZeroU64::new(90_000).unwrap_or_else(|| panic!("timebase")),
                },
                model: ModelName::new("fixture-person")
                    .unwrap_or_else(|error| panic!("model: {error}")),
                model_version: ModelVersion::new("1.0.0")
                    .unwrap_or_else(|error| panic!("version: {error}")),
                detections: DetectionSet::new(vec![person()])
                    .unwrap_or_else(|error| panic!("detections: {error}")),
            },
        )
        .unwrap_or_else(|error| panic!("event: {error}"));
        let json =
            serde_json::to_string(&event).unwrap_or_else(|error| panic!("serialize: {error}"));
        let decoded: DomainEvent =
            serde_json::from_str(&json).unwrap_or_else(|error| panic!("deserialize: {error}"));
        assert_eq!(decoded, event);
        assert!(json.contains("\"schema_version\":1"));
        assert!(json.contains("person_detection_observed"));
    }

    #[test]
    fn external_json_cannot_bypass_geometry_validation() {
        assert!(serde_json::from_str::<NormalizedBoundingBox>("[0.9,0.0,0.2,0.5]").is_err());
    }

    #[test]
    fn external_json_cannot_bypass_detection_scope_or_delivery_class() {
        let json = r#"{"event_id":"event-001","schema_version":1,"occurred_at":1700000000000000,"camera_id":null,"delivery_class":"telemetry","payload":{"kind":"person_detection_observed","media_time":{"epoch":1,"ticks":1,"timebase_hz":90000},"model":"fixture","model_version":"1.0","detections":[]}}"#;
        assert!(serde_json::from_str::<DomainEvent>(json).is_err());
    }
}
