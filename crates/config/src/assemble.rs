//! Assembles resolved settings into the existing validated domain types.
//!
//! This is the join between the Phase 7 configuration domain and the types the
//! pipeline already takes. Group validation goes through the real constructors —
//! [`OneCameraConfig::validate`], [`StorageWatermarks::new`],
//! [`RetentionPolicy::new`] — rather than a second copy of their arithmetic. A
//! duplicated memory-budget calculation that drifts from the one the pipeline
//! actually uses would accept configurations the pipeline then rejects at
//! startup, which is the failure this avoids.

use std::{error::Error, fmt, num::NonZeroU64, num::NonZeroUsize, path::PathBuf, time::Duration};

use surveillance_common::types::CameraId;
use surveillance_storage::retention::{RetentionPolicy, StorageWatermarks};

use crate::{
    classification::SettingId,
    resolution::ResolvedSettings,
    settings::{CameraCredentialsSettings, OneCameraSettings},
    value::SettingValue,
};

/// Reads one camera's settings into the unvalidated transaction input.
///
/// # Errors
/// Returns [`AssemblyError::Missing`] naming the first setting that has no
/// value in any layer, or [`AssemblyError::WrongKind`] when a value's shape does
/// not match its declared kind.
#[allow(
    clippy::too_many_lines,
    reason = "one function reads the whole camera transaction; splitting it would \
              hide which settings form the unit that OneCameraConfig validates"
)]
pub fn one_camera_settings(
    resolved: &ResolvedSettings,
    camera: &CameraId,
) -> Result<OneCameraSettings, AssemblyError> {
    let reader = Reader {
        resolved,
        camera: Some(camera),
    };

    // Both halves of a credential or neither: a username with no password is a
    // configuration mistake, not a partial credential to attempt a connection
    // with.
    let username = reader.optional_secret(SettingId::CameraRtspUsername);
    let password = reader.optional_secret(SettingId::CameraRtspPassword);
    let credentials = match (username, password) {
        (Some(username), Some(password)) => Some(CameraCredentialsSettings { username, password }),
        (None, None) => None,
        (Some(_), None) => return Err(AssemblyError::Missing(SettingId::CameraRtspPassword)),
        (None, Some(_)) => return Err(AssemblyError::Missing(SettingId::CameraRtspUsername)),
    };

    Ok(OneCameraSettings {
        camera_id: camera.as_str().to_owned(),
        rtsp_endpoint: reader.text(SettingId::CameraRtspEndpoint)?,
        credentials,
        connect_timeout: reader.duration(SettingId::CameraRtspConnectTimeout)?,
        max_access_unit_bytes: reader.usize(SettingId::CameraRtspMaxAccessUnitBytes)?,
        max_nals_per_access_unit: reader.usize(SettingId::CameraRtspMaxNalsPerAccessUnit)?,
        reconnect_initial: reader.duration(SettingId::CameraReconnectInitial)?,
        reconnect_maximum: reader.duration(SettingId::CameraReconnectMaximum)?,
        reconnect_jitter_basis_points: reader.u16(SettingId::CameraReconnectJitterBasisPoints)?,
        teardown_timeout: reader.duration(SettingId::CameraReconnectTeardownTimeout)?,
        reconnect_stable_session_reset: reader
            .duration(SettingId::CameraReconnectStableSessionReset)?,
        recorder_queue_items: reader.usize(SettingId::CameraQueueRecorderItems)?,
        live_queue_items: reader.usize(SettingId::CameraQueueLiveItems)?,
        detection_enabled: reader.boolean(SettingId::CameraDetectionEnabled)?,
        detection_queue_items: reader.usize(SettingId::CameraDetectionQueueItems)?,
        detection_sample_fps: reader.u16(SettingId::CameraDetectionSampleFps)?,
        detection_max_gap: reader.duration(SettingId::CameraDetectionMaxGap)?,
        decoded_frame_max_bytes: reader.usize(SettingId::CameraDetectionDecodedFrameMaxBytes)?,
        detection_decoder: reader.path(SettingId::CameraDetectionDecoder)?,
        detection_decode_timeout: reader.duration(SettingId::CameraDetectionDecodeTimeout)?,
        width: reader.u32(SettingId::CameraVideoWidth)?,
        height: reader.u32(SettingId::CameraVideoHeight)?,
        segment_duration: reader.duration(SettingId::CameraRecordingSegmentDuration)?,
        fragment_duration: reader.duration(SettingId::CameraRecordingFragmentDuration)?,
        max_segment_bytes: reader.usize(SettingId::CameraRecordingMaxSegmentBytes)?,
        max_fragment_bytes: reader.usize(SettingId::CameraRecordingMaxFragmentBytes)?,
        max_segment_samples: reader.usize(SettingId::CameraRecordingMaxSegmentSamples)?,
        max_pipeline_memory_bytes: reader.usize(SettingId::CameraMemoryMaxPipelineBytes)?,
        recording_root: Reader {
            resolved,
            camera: None,
        }
        .path(SettingId::StorageRecordingRoot)?,
        ffprobe: reader.path(SettingId::CameraRecordingFfprobe)?,
        clock_drift_tolerance: reader.duration(SettingId::CameraRecordingClockDriftTolerance)?,
        lifecycle_event_items: reader.usize(SettingId::CameraQueueLifecycleEventItems)?,
        reconciliation_scan_entries: reader
            .usize(SettingId::CameraRecordingReconciliationScanEntries)?,
        // LiveKit publication is optional and is assembled separately, because
        // a camera without a publisher is a supported configuration and must
        // not fail assembly.
        livekit: None,
    })
}

/// Builds the validated storage watermarks from resolved settings.
///
/// # Errors
/// Returns [`AssemblyError::Missing`] for an absent setting, or
/// [`AssemblyError::Invariant`] when the four values are not a legal set.
pub fn storage_watermarks(resolved: &ResolvedSettings) -> Result<StorageWatermarks, AssemblyError> {
    let reader = Reader {
        resolved,
        camera: None,
    };
    StorageWatermarks::new(
        reader.u8(SettingId::StorageWatermarkWarning)?,
        reader.u8(SettingId::StorageWatermarkCritical)?,
        reader.u8(SettingId::StorageWatermarkEmergency)?,
        reader.u8(SettingId::StorageWatermarkHysteresis)?,
    )
    .map_err(|_| AssemblyError::Invariant {
        group: "storage watermarks",
        detail: "thresholds must strictly increase below 100, with a nonzero \
                 hysteresis smaller than each transition gap",
    })
}

/// Builds the validated retention policy from resolved settings.
///
/// # Errors
/// Returns [`AssemblyError::Missing`] for an absent setting, or
/// [`AssemblyError::Invariant`] for an unsafe policy.
pub fn retention_policy(resolved: &ResolvedSettings) -> Result<RetentionPolicy, AssemblyError> {
    let reader = Reader {
        resolved,
        camera: None,
    };
    let watermarks = storage_watermarks(resolved)?;
    let files = NonZeroUsize::new(reader.usize(SettingId::RetentionBatchFiles)?).ok_or(
        AssemblyError::Invariant {
            group: "retention",
            detail: "batch_files must be greater than zero",
        },
    )?;
    let bytes = NonZeroU64::new(reader.u64(SettingId::RetentionBatchBytes)?).ok_or(
        AssemblyError::Invariant {
            group: "retention",
            detail: "batch_bytes must be greater than zero",
        },
    )?;
    RetentionPolicy::new(
        reader.duration(SettingId::RetentionMaxAge)?,
        reader.duration(SettingId::RetentionProtectedRecent)?,
        files,
        bytes,
        watermarks,
    )
    .map_err(|_| AssemblyError::Invariant {
        group: "retention",
        detail: "protected_recent must be nonzero and strictly shorter than max_age",
    })
}

/// Reads typed values for one scope.
struct Reader<'a> {
    resolved: &'a ResolvedSettings,
    camera: Option<&'a CameraId>,
}

impl Reader<'_> {
    fn value(&self, id: SettingId) -> Result<SettingValue, AssemblyError> {
        self.resolved
            .effective(id, self.camera)
            .map(|resolved| resolved.value().clone())
            .ok_or(AssemblyError::Missing(id))
    }

    fn duration(&self, id: SettingId) -> Result<Duration, AssemblyError> {
        match self.value(id)? {
            SettingValue::Duration(value) => Ok(value),
            _ => Err(AssemblyError::WrongKind(id)),
        }
    }

    fn boolean(&self, id: SettingId) -> Result<bool, AssemblyError> {
        match self.value(id)? {
            SettingValue::Boolean(value) => Ok(value),
            _ => Err(AssemblyError::WrongKind(id)),
        }
    }

    fn text(&self, id: SettingId) -> Result<String, AssemblyError> {
        match self.value(id)? {
            SettingValue::Text(value) => Ok(value),
            _ => Err(AssemblyError::WrongKind(id)),
        }
    }

    fn path(&self, id: SettingId) -> Result<PathBuf, AssemblyError> {
        match self.value(id)? {
            SettingValue::Path(value) => Ok(value),
            _ => Err(AssemblyError::WrongKind(id)),
        }
    }

    fn optional_secret(&self, id: SettingId) -> Option<String> {
        match self.resolved.effective(id, self.camera)?.value() {
            SettingValue::Secret(value) => Some(value.expose().clone()),
            _ => None,
        }
    }

    fn u64(&self, id: SettingId) -> Result<u64, AssemblyError> {
        match self.value(id)? {
            SettingValue::Unsigned(value) => Ok(value),
            _ => Err(AssemblyError::WrongKind(id)),
        }
    }

    fn usize(&self, id: SettingId) -> Result<usize, AssemblyError> {
        usize::try_from(self.u64(id)?).map_err(|_| AssemblyError::OutOfRange(id))
    }

    fn u32(&self, id: SettingId) -> Result<u32, AssemblyError> {
        u32::try_from(self.u64(id)?).map_err(|_| AssemblyError::OutOfRange(id))
    }

    fn u16(&self, id: SettingId) -> Result<u16, AssemblyError> {
        u16::try_from(self.u64(id)?).map_err(|_| AssemblyError::OutOfRange(id))
    }

    fn u8(&self, id: SettingId) -> Result<u8, AssemblyError> {
        u8::try_from(self.u64(id)?).map_err(|_| AssemblyError::OutOfRange(id))
    }
}

/// Resolved settings that could not be assembled into a domain type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssemblyError {
    /// No layer supplies this setting and it has no compiled default.
    Missing(SettingId),
    /// The value's shape disagrees with the setting's declared kind.
    WrongKind(SettingId),
    /// The value does not fit the domain type's width.
    OutOfRange(SettingId),
    /// A group of settings is individually legal but jointly invalid.
    Invariant {
        group: &'static str,
        detail: &'static str,
    },
}

impl fmt::Display for AssemblyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(id) => write!(formatter, "setting {id} has no value and no default"),
            Self::WrongKind(id) => write!(formatter, "setting {id} has a value of the wrong kind"),
            Self::OutOfRange(id) => write!(formatter, "setting {id} is out of range"),
            Self::Invariant { group, detail } => write!(formatter, "{group}: {detail}"),
        }
    }
}

impl Error for AssemblyError {}

#[cfg(test)]
mod tests {
    use super::{one_camera_settings, retention_policy, storage_watermarks, AssemblyError};
    use crate::{
        classification::{Origin, SettingId},
        defaults,
        resolution::{Layer, ResolvedSettings, SettingKey},
        settings::OneCameraConfig,
        value::SettingValue,
    };
    use surveillance_common::types::CameraId;

    fn camera() -> CameraId {
        CameraId::new("camera-01".to_owned()).expect("valid test camera identifier")
    }

    /// Defaults plus the deployment values that have no safe default.
    fn deployable() -> Vec<Layer> {
        let mut deployment = Layer::new(Origin::File);
        let mut set = |id, value| {
            deployment
                .insert(SettingKey::unscoped(id), value)
                .expect("valid test entry");
        };
        set(
            SettingId::CameraRtspEndpoint,
            SettingValue::Text("rtsp://127.0.0.1:8554/camera-01".to_owned()),
        );
        set(SettingId::CameraVideoWidth, SettingValue::Unsigned(1920));
        set(SettingId::CameraVideoHeight, SettingValue::Unsigned(1080));
        set(
            SettingId::CameraDetectionDecodedFrameMaxBytes,
            SettingValue::Unsigned(1920 * 1080 * 3),
        );
        vec![defaults::layer(), deployment]
    }

    #[test]
    fn defaults_plus_deployment_values_assemble_and_validate() {
        let resolved = ResolvedSettings::resolve(deployable());
        let settings = one_camera_settings(&resolved, &camera()).expect("assembles");
        OneCameraConfig::validate(settings).expect("the assembled transaction validates");
    }

    #[test]
    fn a_missing_deployment_value_names_the_setting() {
        let resolved = ResolvedSettings::resolve(vec![defaults::layer()]);
        // `OneCameraSettings` carries credentials and deliberately has no
        // `Debug`, so the assertion matches rather than formats.
        assert!(matches!(
            one_camera_settings(&resolved, &camera()),
            Err(AssemblyError::Missing(SettingId::CameraRtspEndpoint))
        ));
    }

    #[test]
    fn a_username_without_a_password_is_rejected() {
        let mut layers = deployable();
        let mut credentials = Layer::new(Origin::Environment);
        credentials
            .insert(
                SettingKey::unscoped(SettingId::CameraRtspUsername),
                SettingValue::Secret(crate::secret::Secret::new("operator".to_owned())),
            )
            .expect("valid entry");
        layers.push(credentials);
        let resolved = ResolvedSettings::resolve(layers);
        assert!(matches!(
            one_camera_settings(&resolved, &camera()),
            Err(AssemblyError::Missing(SettingId::CameraRtspPassword))
        ));
    }

    #[test]
    fn both_credential_halves_assemble_into_one_credential() {
        let mut layers = deployable();
        let mut credentials = Layer::new(Origin::Environment);
        for (id, value) in [
            (SettingId::CameraRtspUsername, "operator"),
            (SettingId::CameraRtspPassword, "hunter2"),
        ] {
            credentials
                .insert(
                    SettingKey::unscoped(id),
                    SettingValue::Secret(crate::secret::Secret::new(value.to_owned())),
                )
                .expect("valid entry");
        }
        layers.push(credentials);
        let resolved = ResolvedSettings::resolve(layers);
        let settings = one_camera_settings(&resolved, &camera()).expect("assembles");
        let credentials = settings.credentials.expect("credentials present");
        assert_eq!(credentials.username, "operator");
        assert_eq!(credentials.password, "hunter2");
    }

    #[test]
    fn a_camera_override_reaches_the_assembled_settings() {
        let mut layers = deployable();
        let mut override_layer = Layer::new(Origin::Persisted);
        override_layer
            .insert(
                SettingKey::scoped(SettingId::CameraDetectionSampleFps, camera())
                    .expect("camera scoped"),
                SettingValue::Unsigned(12),
            )
            .expect("valid entry");
        layers.push(override_layer);
        let resolved = ResolvedSettings::resolve(layers);
        let settings = one_camera_settings(&resolved, &camera()).expect("assembles");
        assert_eq!(settings.detection_sample_fps, 12);
    }

    #[test]
    fn the_memory_budget_is_enforced_by_the_real_validator() {
        // Raising a queue capacity alone is a legal integer that breaches the
        // pipeline memory budget. The rejection must come from the same
        // arithmetic the pipeline uses, not from a copy.
        let mut layers = deployable();
        let mut breach = Layer::new(Origin::Persisted);
        breach
            .insert(
                SettingKey::unscoped(SettingId::CameraQueueRecorderItems),
                SettingValue::Unsigned(4_096),
            )
            .expect("valid entry");
        layers.push(breach);
        let resolved = ResolvedSettings::resolve(layers);
        let settings = one_camera_settings(&resolved, &camera()).expect("assembles");
        assert!(
            OneCameraConfig::validate(settings).is_err(),
            "a queue capacity that breaches the memory budget must be rejected"
        );
    }

    #[test]
    fn default_watermarks_are_a_legal_set() {
        let resolved = ResolvedSettings::resolve(vec![defaults::layer()]);
        storage_watermarks(&resolved).expect("shipped watermark defaults are legal");
    }

    #[test]
    fn out_of_order_watermarks_are_rejected_as_a_group() {
        let mut broken = Layer::new(Origin::Persisted);
        broken
            .insert(
                SettingKey::unscoped(SettingId::StorageWatermarkWarning),
                SettingValue::Unsigned(95),
            )
            .expect("valid entry");
        let resolved = ResolvedSettings::resolve(vec![defaults::layer(), broken]);
        assert!(matches!(
            storage_watermarks(&resolved),
            Err(AssemblyError::Invariant { .. })
        ));
    }

    #[test]
    fn default_retention_is_a_legal_policy() {
        let resolved = ResolvedSettings::resolve(vec![defaults::layer()]);
        retention_policy(&resolved).expect("shipped retention defaults are legal");
    }

    #[test]
    fn a_protected_window_longer_than_retention_is_rejected() {
        let mut broken = Layer::new(Origin::Persisted);
        broken
            .insert(
                SettingKey::unscoped(SettingId::RetentionProtectedRecent),
                SettingValue::Duration(std::time::Duration::from_secs(90 * 24 * 60 * 60)),
            )
            .expect("valid entry");
        let resolved = ResolvedSettings::resolve(vec![defaults::layer(), broken]);
        assert!(matches!(
            retention_policy(&resolved),
            Err(AssemblyError::Invariant { .. })
        ));
    }
}
