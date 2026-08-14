//! Immutable, versioned domain-event contracts.

use serde::{Deserialize, Serialize};
use std::{error::Error, fmt, num::NonZeroU64};
use surveillance_common::types::{CameraId, RecordingId};

pub const CURRENT_SCHEMA_VERSION: u16 = 1;
pub const MAX_DETECTIONS_PER_EVENT: usize = 64;
/// Bounds a configuration-change payload.
///
/// The bus is bounded, so an unbounded payload would be a way around that
/// bound. A batch larger than this is still committed and audited; only the
/// notification is refused, because the audit table is the record.
pub const MAX_CHANGED_SETTINGS_PER_EVENT: usize = 256;

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
validated_text!(SettingKeyName, "setting key", 128);
validated_text!(ChangeOutcomeName, "change outcome", 32);

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

/// One setting touched by a configuration change.
///
/// Carries the setting's identity and what happened to it, never its value.
/// A consumer that needs the new value reads it back through the settings API.
/// Keeping values off the bus is structural rather than a redaction step each
/// publication site has to remember.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ChangedSetting {
    pub key: SettingKeyName,
    pub camera_id: Option<CameraId>,
    pub outcome: ChangeOutcomeName,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "Vec<ChangedSetting>", into = "Vec<ChangedSetting>")]
pub struct ChangedSettingSet(Vec<ChangedSetting>);

impl ChangedSettingSet {
    /// # Errors
    /// Returns an error when the bounded change count is exceeded. The bus is
    /// bounded, so an unbounded payload would be a way around that bound.
    pub fn new(values: Vec<ChangedSetting>) -> Result<Self, EventValidationError> {
        if values.len() > MAX_CHANGED_SETTINGS_PER_EVENT {
            Err(EventValidationError::TooManyChangedSettings)
        } else {
            Ok(Self(values))
        }
    }

    #[must_use]
    pub fn as_slice(&self) -> &[ChangedSetting] {
        &self.0
    }
}

impl TryFrom<Vec<ChangedSetting>> for ChangedSettingSet {
    type Error = EventValidationError;
    fn try_from(values: Vec<ChangedSetting>) -> Result<Self, Self::Error> {
        Self::new(values)
    }
}

impl From<ChangedSettingSet> for Vec<ChangedSetting> {
    fn from(values: ChangedSettingSet) -> Self {
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
    /// Published after the settings transaction commits.
    ///
    /// This is a notification, not the record: the bus provides no durability
    /// (ADR 002), and the audit table is where the change is remembered. A
    /// dropped notification means a consumer is briefly behind, not that the
    /// change was lost.
    ConfigurationChanged {
        generation: u64,
        changed: ChangedSettingSet,
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
    ConfigurationChanged,
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
        if let EventPayload::ConfigurationChanged { .. } = &payload {
            // Configuration is state, not a stream of independently meaningful
            // facts: a consumer that missed generation 7 and receives 8 is
            // correct, because 8 supersedes 7. `CriticalLifecycle` would be
            // wrong — it would imply the bus is where the change is remembered,
            // and ADR 002 says it provides no durability.
            if delivery_class != DeliveryClass::LatestState {
                return Err(EventValidationError::WrongDeliveryClass);
            }
            // A batch may span cameras, so the event is deployment-scoped and
            // carries per-setting camera identity inside the payload instead.
            if camera_id.is_some() {
                return Err(EventValidationError::ConfigurationWithCamera);
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
            EventPayload::ConfigurationChanged { .. } => "configuration",
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
            EventPayload::ConfigurationChanged { .. } => EventKind::ConfigurationChanged,
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
    TooManyChangedSettings,
    DetectionWithoutCamera,
    ConfigurationWithCamera,
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
        ChangeOutcomeName, ChangedSetting, ChangedSettingSet, Confidence, DeliveryClass,
        DetectionSet, DomainEvent, EventId, EventKind, EventPayload, EventValidationError,
        ModelName, ModelVersion, NormalizedBoundingBox, OccurredAtUnixMicros, PersonDetection,
        SettingKeyName, SourceMediaTime, MAX_CHANGED_SETTINGS_PER_EVENT, MAX_DETECTIONS_PER_EVENT,
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

    fn changed(key: &str, outcome: &str) -> ChangedSetting {
        ChangedSetting {
            key: SettingKeyName::new(key).unwrap_or_else(|error| panic!("valid key: {error:?}")),
            camera_id: None,
            outcome: ChangeOutcomeName::new(outcome)
                .unwrap_or_else(|error| panic!("valid outcome: {error:?}")),
        }
    }

    fn configuration_event(
        delivery_class: DeliveryClass,
        camera_id: Option<CameraId>,
        changed: Vec<ChangedSetting>,
    ) -> Result<DomainEvent, EventValidationError> {
        DomainEvent::new(
            EventId::new("event-config-001").unwrap_or_else(|error| panic!("id: {error:?}")),
            OccurredAtUnixMicros::new(1_700_000_000_000_000)
                .unwrap_or_else(|error| panic!("timestamp: {error:?}")),
            camera_id,
            delivery_class,
            EventPayload::ConfigurationChanged {
                generation: 7,
                changed: ChangedSettingSet::new(changed)
                    .unwrap_or_else(|error| panic!("set: {error:?}")),
            },
        )
    }

    #[test]
    fn a_configuration_change_is_latest_state() {
        let event = configuration_event(
            DeliveryClass::LatestState,
            None,
            vec![changed("retention.max_age", "applied")],
        )
        .unwrap_or_else(|error| panic!("valid configuration event: {error:?}"));
        assert_eq!(event.kind(), EventKind::ConfigurationChanged);
        assert_eq!(event.state_key(), Some((None, "configuration")));
    }

    #[test]
    fn a_configuration_change_rejects_other_delivery_classes() {
        // CriticalLifecycle would imply the bus is where the change is
        // remembered. It is not; the audit table is.
        for class in [
            DeliveryClass::CriticalLifecycle,
            DeliveryClass::Telemetry,
            DeliveryClass::Detection,
        ] {
            assert_eq!(
                configuration_event(class, None, vec![changed("retention.max_age", "applied")]),
                Err(EventValidationError::WrongDeliveryClass)
            );
        }
    }

    #[test]
    fn a_configuration_change_is_deployment_scoped() {
        let camera = CameraId::new("camera-01").unwrap_or_else(|error| panic!("camera: {error}"));
        assert_eq!(
            configuration_event(
                DeliveryClass::LatestState,
                Some(camera),
                vec![changed("camera.detection.sample_fps", "applied")],
            ),
            Err(EventValidationError::ConfigurationWithCamera)
        );
    }

    #[test]
    fn a_configuration_payload_is_bounded() {
        let oversized = (0..=MAX_CHANGED_SETTINGS_PER_EVENT)
            .map(|_| changed("retention.max_age", "applied"))
            .collect::<Vec<_>>();
        assert_eq!(
            ChangedSettingSet::new(oversized).err(),
            Some(EventValidationError::TooManyChangedSettings),
            "an unbounded payload would be a way around the bus bound"
        );
    }

    #[test]
    fn a_configuration_event_carries_no_values() {
        let event = configuration_event(
            DeliveryClass::LatestState,
            None,
            vec![changed("camera.rtsp.password", "applied")],
        )
        .unwrap_or_else(|error| panic!("valid configuration event: {error:?}"));
        let rendered = serde_json::to_string(&event).unwrap_or_default();
        assert!(rendered.contains("camera.rtsp.password"));
        assert!(
            !rendered.contains("value"),
            "the payload must carry keys and outcomes only: {rendered}"
        );
    }

    #[test]
    fn external_json_cannot_smuggle_a_configuration_event_onto_another_class() {
        let json = r#"{"event_id":"event-002","schema_version":1,"occurred_at":1700000000000000,"camera_id":null,"delivery_class":"critical_lifecycle","payload":{"kind":"configuration_changed","generation":7,"changed":[]}}"#;
        assert!(serde_json::from_str::<DomainEvent>(json).is_err());
    }
}
