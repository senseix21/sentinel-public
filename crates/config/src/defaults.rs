//! The compiled default layer.
//!
//! Every value here is reversible and safe for local development. Deployment
//! values arrive from higher layers. Settings that have no safe default —
//! a camera's endpoint, the database URL, credentials — are deliberately absent
//! and must be supplied, which [`ResolvedSettings::missing`] surfaces.
//!
//! [`ResolvedSettings::missing`]: crate::resolution::ResolvedSettings::missing

use std::{path::PathBuf, time::Duration};

use crate::{
    classification::{Origin, SettingId},
    resolution::{Layer, SettingKey},
    value::SettingValue,
};

/// Settings with no compiled default, because no default is safe.
///
/// `database.url` absent means persistence is disabled rather than pointed at a
/// guess. The camera identity and endpoint have no meaningful default at all.
/// `audit.fingerprint_salt` has none because a shared default salt would make
/// fingerprints comparable across deployments, which is exactly what salting is
/// for.
pub const WITHOUT_DEFAULTS: &[SettingId] = &[
    SettingId::DatabaseUrl,
    SettingId::AuditFingerprintSalt,
    SettingId::CameraRtspEndpoint,
    SettingId::CameraRtspUsername,
    SettingId::CameraRtspPassword,
    SettingId::CameraVideoWidth,
    SettingId::CameraVideoHeight,
    SettingId::CameraDetectionDecodedFrameMaxBytes,
    // No default on purpose: an unset model path is how a deployment says it
    // has not configured a production model, and `Path` rejects the empty
    // string. A configured path that cannot be loaded is a startup failure,
    // which is a different condition entirely.
    SettingId::CameraDetectionModelPath,
    SettingId::CameraDetectionModelSha256,
    SettingId::CameraLiveKitEndpoint,
    SettingId::CameraLiveKitApiKey,
    SettingId::CameraLiveKitApiSecret,
    SettingId::CameraLiveKitDeploymentNamespace,
];

/// Builds the compiled default layer.
///
/// A kind mismatch here would be a source defect, not input, so entries are
/// inserted without a fallible path. Such an entry would be dropped rather than
/// coerced, which `every_setting_either_has_a_default_or_is_declared_as_having_none`
/// detects as a missing default.
#[must_use]
pub fn layer() -> Layer {
    let mut layer = Layer::new(Origin::Default);
    bootstrap(&mut layer);
    global_operational(&mut layer);
    camera_templates(&mut layer);
    layer
}

/// Inserts a compiled default.
///
/// The default layer is compiled, so a kind mismatch is a source defect rather
/// than input. Such an entry is dropped rather than coerced, which the module's
/// completeness test detects as a missing default.
fn put(layer: &mut Layer, id: SettingId, value: SettingValue) {
    let _ = layer.insert(SettingKey::unscoped(id), value);
}

fn bootstrap(layer: &mut Layer) {
    let mut set = |id, value| put(layer, id, value);
    set(
        SettingId::ServerHealthBind,
        SettingValue::Text("127.0.0.1:3000".to_owned()),
    );
    set(
        SettingId::StorageRecordingRoot,
        SettingValue::Path(PathBuf::from("recordings")),
    );
    set(SettingId::DatabaseMaxConnections, SettingValue::Unsigned(8));
    set(
        SettingId::DatabaseAcquireTimeout,
        SettingValue::Duration(Duration::from_secs(5)),
    );
    set(
        SettingId::SupervisionMaximumCameras,
        SettingValue::Unsigned(256),
    );
    set(
        SettingId::SupervisionCommandQueueItems,
        SettingValue::Unsigned(64),
    );
    // Default off, per ADR 008: a deployment must opt in to being reconfigured
    // over HTTP.
    set(
        SettingId::ApiMutatingSurfaceEnabled,
        SettingValue::Boolean(false),
    );
    set(
        SettingId::ConfigCachePath,
        SettingValue::Path(PathBuf::from("config/last-known-good.toml")),
    );
    set(
        SettingId::ConfigStaleRetryInterval,
        SettingValue::Duration(Duration::from_secs(5)),
    );
    // Long enough to outlast typical service start ordering for a local
    // PostgreSQL, which ADR 008 names as the dominant cause of `Stale`.
    set(
        SettingId::ConfigStaleRetryBudget,
        SettingValue::Duration(Duration::from_secs(300)),
    );
}

fn global_operational(layer: &mut Layer) {
    let mut set = |id, value| put(layer, id, value);
    set(
        SettingId::SupervisionCommandTimeout,
        SettingValue::Duration(Duration::from_secs(10)),
    );
    set(
        SettingId::ApiMaxRequestBytes,
        SettingValue::Unsigned(16 * 1024),
    );
    set(
        SettingId::ApiIoTimeout,
        SettingValue::Duration(Duration::from_secs(5)),
    );
    // Sixty seconds is a reversible guess, not a measurement: it is longer than
    // any media buffer observed in a browser here and short enough that a client
    // that has actually gone away releases its assembler child within a minute.
    set(
        SettingId::ApiPlaybackStallTimeout,
        SettingValue::Duration(Duration::from_secs(60)),
    );
    set(
        SettingId::RetentionMaxAge,
        SettingValue::Duration(Duration::from_secs(30 * 24 * 60 * 60)),
    );
    set(
        SettingId::RetentionProtectedRecent,
        SettingValue::Duration(Duration::from_secs(60 * 60)),
    );
    set(SettingId::RetentionBatchFiles, SettingValue::Unsigned(100));
    set(
        SettingId::RetentionBatchBytes,
        SettingValue::Unsigned(10 * 1024 * 1024 * 1024),
    );
    set(
        SettingId::RetentionInterval,
        SettingValue::Duration(Duration::from_secs(60)),
    );
    set(
        SettingId::StorageWatermarkWarning,
        SettingValue::Unsigned(80),
    );
    set(
        SettingId::StorageWatermarkCritical,
        SettingValue::Unsigned(90),
    );
    set(
        SettingId::StorageWatermarkEmergency,
        SettingValue::Unsigned(97),
    );
    set(
        SettingId::StorageWatermarkHysteresis,
        SettingValue::Unsigned(2),
    );
    set(
        SettingId::ReconciliationQueueItems,
        SettingValue::Unsigned(256),
    );
    set(
        SettingId::ReconciliationInterval,
        SettingValue::Duration(Duration::from_secs(120)),
    );
    set(
        SettingId::ReconciliationBackoff,
        SettingValue::Duration(Duration::from_secs(30)),
    );
    set(SettingId::MetadataQueueItems, SettingValue::Unsigned(256));
    set(
        SettingId::MetadataFlushInterval,
        SettingValue::Duration(Duration::from_millis(250)),
    );
    set(
        SettingId::MetadataRetryBackoff,
        SettingValue::Duration(Duration::from_secs(30)),
    );
    set(
        SettingId::PlaybackFfmpeg,
        SettingValue::Path(PathBuf::from("ffmpeg")),
    );
    set(
        SettingId::PlaybackProbeTimeout,
        SettingValue::Duration(Duration::from_secs(5)),
    );
    set(
        SettingId::LiveKitViewerTokenTtl,
        SettingValue::Duration(Duration::from_secs(5 * 60)),
    );
}

/// All-camera templates. A camera overrides any of these individually.
fn camera_templates(layer: &mut Layer) {
    camera_source(layer);
    camera_recording(layer);
    camera_detection(layer);
    camera_livekit(layer);
}

fn camera_source(layer: &mut Layer) {
    let mut set = |id, value| put(layer, id, value);
    set(
        SettingId::CameraRtspConnectTimeout,
        SettingValue::Duration(Duration::from_secs(5)),
    );
    set(
        SettingId::CameraRtspMaxAccessUnitBytes,
        SettingValue::Unsigned(2 * 1024 * 1024),
    );
    set(
        SettingId::CameraRtspMaxNalsPerAccessUnit,
        SettingValue::Unsigned(128),
    );
    set(
        SettingId::CameraReconnectInitial,
        SettingValue::Duration(Duration::from_millis(250)),
    );
    set(
        SettingId::CameraReconnectMaximum,
        SettingValue::Duration(Duration::from_secs(30)),
    );
    set(
        SettingId::CameraReconnectJitterBasisPoints,
        SettingValue::Unsigned(2_000),
    );
    set(
        SettingId::CameraReconnectTeardownTimeout,
        SettingValue::Duration(Duration::from_secs(2)),
    );
    set(
        SettingId::CameraReconnectStableSessionReset,
        SettingValue::Duration(Duration::from_secs(30)),
    );
    set(
        SettingId::CameraQueueRecorderItems,
        SettingValue::Unsigned(32),
    );
    set(SettingId::CameraQueueLiveItems, SettingValue::Unsigned(4));
    set(
        SettingId::CameraQueueLifecycleEventItems,
        SettingValue::Unsigned(64),
    );
}

fn camera_recording(layer: &mut Layer) {
    let mut set = |id, value| put(layer, id, value);
    set(
        SettingId::CameraRecordingSegmentDuration,
        SettingValue::Duration(Duration::from_secs(30)),
    );
    set(
        SettingId::CameraRecordingFragmentDuration,
        SettingValue::Duration(Duration::from_secs(1)),
    );
    set(
        SettingId::CameraRecordingMaxSegmentBytes,
        SettingValue::Unsigned(32 * 1024 * 1024),
    );
    set(
        SettingId::CameraRecordingMaxFragmentBytes,
        SettingValue::Unsigned(8 * 1024 * 1024),
    );
    set(
        SettingId::CameraRecordingMaxSegmentSamples,
        SettingValue::Unsigned(1_000),
    );
    set(
        SettingId::CameraRecordingClockDriftTolerance,
        SettingValue::Duration(Duration::from_secs(5)),
    );
    set(
        SettingId::CameraRecordingFfprobe,
        SettingValue::Path(PathBuf::from("ffprobe")),
    );
    set(
        SettingId::CameraRecordingReconciliationScanEntries,
        SettingValue::Unsigned(100_000),
    );
    set(
        SettingId::CameraMemoryMaxPipelineBytes,
        SettingValue::Unsigned(128 * 1024 * 1024),
    );
}

fn camera_detection(layer: &mut Layer) {
    let mut set = |id, value| put(layer, id, value);
    set(
        SettingId::CameraDetectionEnabled,
        SettingValue::Boolean(true),
    );
    set(
        SettingId::CameraDetectionQueueItems,
        SettingValue::Unsigned(2),
    );
    set(
        SettingId::CameraDetectionSampleFps,
        SettingValue::Unsigned(5),
    );
    set(
        SettingId::CameraDetectionMaxGap,
        SettingValue::Duration(Duration::from_secs(2)),
    );
    set(
        SettingId::CameraDetectionDecoder,
        SettingValue::Path(PathBuf::from("ffmpeg")),
    );
    set(
        SettingId::CameraDetectionDecodeTimeout,
        SettingValue::Duration(Duration::from_secs(2)),
    );
    set(
        SettingId::CameraDetectionInputWidth,
        SettingValue::Unsigned(416),
    );
    set(
        SettingId::CameraDetectionInputHeight,
        SettingValue::Unsigned(416),
    );
    set(
        SettingId::CameraDetectionExecutionProviders,
        SettingValue::Text("cpu".to_owned()),
    );
    // One thread each by default: a session per camera with machine-sized
    // pools would contend through one blocking pool on a small target.
    set(
        SettingId::CameraDetectionIntraOpThreads,
        SettingValue::Unsigned(1),
    );
    set(
        SettingId::CameraDetectionInterOpThreads,
        SettingValue::Unsigned(1),
    );
    set(
        SettingId::CameraDetectionScoreThresholdPercent,
        SettingValue::Unsigned(30),
    );
    set(
        SettingId::CameraDetectionNmsIouPercent,
        SettingValue::Unsigned(45),
    );
    set(
        SettingId::CameraDetectionMotionThresholdPercent,
        SettingValue::Unsigned(2),
    );
    set(
        SettingId::CameraDetectionGateCooldown,
        SettingValue::Duration(Duration::from_secs(5)),
    );
    set(
        SettingId::CameraDetectionGateHeartbeat,
        SettingValue::Duration(Duration::from_secs(10)),
    );
}

fn camera_livekit(layer: &mut Layer) {
    let mut set = |id, value| put(layer, id, value);
    set(
        SettingId::CameraLiveKitQueueItems,
        SettingValue::Unsigned(8),
    );
    set(
        SettingId::CameraLiveKitMaxLag,
        SettingValue::Duration(Duration::from_secs(2)),
    );
    set(
        SettingId::CameraLiveKitMaxAccessUnitBytes,
        SettingValue::Unsigned(2 * 1024 * 1024),
    );
    set(
        SettingId::CameraLiveKitShutdownTimeout,
        SettingValue::Duration(Duration::from_secs(5)),
    );
    set(
        SettingId::CameraLiveKitReconnectInitial,
        SettingValue::Duration(Duration::from_millis(250)),
    );
    set(
        SettingId::CameraLiveKitReconnectMaximum,
        SettingValue::Duration(Duration::from_secs(30)),
    );
    set(
        SettingId::CameraLiveKitReconnectStableReset,
        SettingValue::Duration(Duration::from_secs(30)),
    );
    set(
        SettingId::CameraLiveKitDecoder,
        SettingValue::Path(PathBuf::from("ffmpeg")),
    );
    // No ceiling by default. Choosing a viewing resolution is a product
    // decision; capping silently would publish something the operator never
    // asked for, and uncapped is at least the honest reading of the stream.
    set(
        SettingId::CameraLiveKitMaxPublishHeight,
        SettingValue::Unsigned(0),
    );
    set(
        SettingId::CameraLiveKitViewerIdleGrace,
        SettingValue::Duration(Duration::from_secs(5)),
    );
}

#[cfg(test)]
mod tests {
    use super::{layer, WITHOUT_DEFAULTS};
    use crate::{
        classification::{Origin, SettingClass, SettingId},
        resolution::ResolvedSettings,
    };

    #[test]
    fn every_setting_either_has_a_default_or_is_declared_as_having_none() {
        let resolved = ResolvedSettings::resolve(vec![layer()]);
        for id in SettingId::ALL {
            let present = resolved.effective(*id, None).is_some();
            let declared_absent = WITHOUT_DEFAULTS.contains(id);
            assert_ne!(
                present, declared_absent,
                "{id} must either have a compiled default or appear in WITHOUT_DEFAULTS"
            );
        }
    }

    #[test]
    fn defaults_carry_the_default_origin() {
        let resolved = ResolvedSettings::resolve(vec![layer()]);
        assert_eq!(
            resolved.origin_of(SettingId::RetentionInterval, None),
            Some(Origin::Default)
        );
    }

    #[test]
    fn every_default_matches_its_declared_kind() {
        // A kind mismatch is silently dropped by the builder, so this proves the
        // layer actually contains what the table says it should.
        let resolved = ResolvedSettings::resolve(vec![layer()]);
        for id in SettingId::ALL {
            if let Some(effective) = resolved.effective(*id, None) {
                assert_eq!(
                    effective.value().kind(),
                    id.kind(),
                    "{id} default has the wrong kind"
                );
            }
        }
    }

    #[test]
    fn no_secret_has_a_compiled_default() {
        let resolved = ResolvedSettings::resolve(vec![layer()]);
        for id in SettingId::ALL {
            if id.secrecy() == crate::classification::Secrecy::Secret {
                assert!(
                    resolved.effective(*id, None).is_none(),
                    "{id} ships a compiled credential"
                );
            }
        }
    }

    #[test]
    fn the_mutating_api_surface_is_off_by_default() {
        let resolved = ResolvedSettings::resolve(vec![layer()]);
        assert_eq!(
            resolved
                .effective(SettingId::ApiMutatingSurfaceEnabled, None)
                .map(|value| value.value().render()),
            Some("false".to_owned()),
            "ADR 008 requires the mutating surface to default off"
        );
    }

    #[test]
    fn the_stale_retry_budget_outlasts_service_start_ordering() {
        let resolved = ResolvedSettings::resolve(vec![layer()]);
        let budget = resolved
            .effective(SettingId::ConfigStaleRetryBudget, None)
            .map(|value| value.value().render());
        assert_eq!(budget, Some("5m".to_owned()));
    }

    #[test]
    fn bootstrap_settings_without_defaults_are_only_the_credential_bearing_ones() {
        for id in WITHOUT_DEFAULTS {
            if id.class() == SettingClass::Bootstrap {
                assert!(
                    matches!(id, SettingId::DatabaseUrl | SettingId::AuditFingerprintSalt),
                    "{id} is bootstrap with no default; confirm the server can still start"
                );
            }
        }
    }
}
