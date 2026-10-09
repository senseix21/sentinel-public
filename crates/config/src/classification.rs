//! The configuration domain: every setting, classified.
//!
//! The `settings!` macro takes one row per setting and generates [`SettingId`],
//! `SettingId::ALL`, and every classification accessor. A row that omits an axis
//! does not parse, so an unclassified setting cannot be added — the guarantee
//! the P7.1 contract requires. The rendered table lives in
//! `docs/phase-7/contracts.md`.
//!
//! A setting key is a durable identity: it appears in the database, in the API,
//! in metric labels, and in audit rows. Renaming one is a migration, not a
//! refactor.

use std::fmt;

/// Who may change a setting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingClass {
    /// Resolvable before the settings store exists, or governing the mechanism
    /// by which settings are changed. Never editable through the API.
    Bootstrap,
    /// Editable through the API, persisted, and audited.
    Operational,
}

/// When a change becomes effective.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ApplicationBoundary {
    /// Read at each use; effective on the next use with no coordination.
    ///
    /// Only settings whose consumer genuinely re-reads them carry this. A
    /// setting whose owner takes it once at construction is restart-scoped,
    /// however cheap it would be to change: reporting `Immediate` for a value
    /// that will not take effect until restart is the failure this avoids.
    Immediate,
    /// Effective when the recorder rolls to a new segment. Never mid-segment.
    NextSegment,
    /// Effective when the camera session is next established. Never mid-session.
    NextReconnect,
    /// Requires rebuilding that camera's pipeline. Operator-triggered, never
    /// automatic, because it interrupts recording.
    PipelineRestart,
    /// Requires restarting the server process.
    ProcessRestart,
}

impl ApplicationBoundary {
    /// Stable label for metrics and API responses.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Immediate => "immediate",
            Self::NextSegment => "next_segment",
            Self::NextReconnect => "next_reconnect",
            Self::PipelineRestart => "pipeline_restart",
            Self::ProcessRestart => "process_restart",
        }
    }

    /// Every boundary, for metric initialization.
    pub const ALL: &'static [Self] = &[
        Self::Immediate,
        Self::NextSegment,
        Self::NextReconnect,
        Self::PipelineRestart,
        Self::ProcessRestart,
    ];
}

impl fmt::Display for ApplicationBoundary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Whether a value may ever be rendered.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Secrecy {
    Public,
    /// Held as `Secret<T>`, never serialized, and audited only as a fingerprint.
    Secret,
}

/// What a setting applies to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scope {
    Global,
    PerCamera,
}

/// Settings that cannot be validated independently of one another.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValidationGroup {
    StorageWatermarks,
    Retention,
    CameraMemoryBudget,
    DecodedFrameBudget,
    ReconnectPolicy,
}

impl ValidationGroup {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StorageWatermarks => "storage_watermarks",
            Self::Retention => "retention",
            Self::CameraMemoryBudget => "camera_memory_budget",
            Self::DecodedFrameBudget => "decoded_frame_budget",
            Self::ReconnectPolicy => "reconnect_policy",
        }
    }

    pub const ALL: &'static [Self] = &[
        Self::StorageWatermarks,
        Self::Retention,
        Self::CameraMemoryBudget,
        Self::DecodedFrameBudget,
        Self::ReconnectPolicy,
    ];
}

impl fmt::Display for ValidationGroup {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The shape of a setting's value.
///
/// Declared per setting so that parsing a layer is type-directed rather than
/// guessed from whatever the file happened to contain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValueKind {
    Boolean,
    /// A non-negative integer: counts, byte bounds, percentages, dimensions.
    Unsigned,
    /// A span written with a unit suffix, such as `250ms`, `30s`, or `7d`.
    Duration,
    Text,
    Path,
}

impl ValueKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Boolean => "boolean",
            Self::Unsigned => "unsigned",
            Self::Duration => "duration",
            Self::Text => "text",
            Self::Path => "path",
        }
    }
}

impl fmt::Display for ValueKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Which layer supplied the effective value.
///
/// Tracked per setting rather than per layer, because the question an operator
/// asks is "why does this setting have this value", one setting at a time.
///
/// Ordering is precedence: later variants win.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Origin {
    Default,
    File,
    Persisted,
    Environment,
}

impl Origin {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::File => "file",
            Self::Persisted => "persisted",
            Self::Environment => "environment",
        }
    }

    pub const ALL: &'static [Self] = &[
        Self::Default,
        Self::File,
        Self::Persisted,
        Self::Environment,
    ];
}

impl fmt::Display for Origin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Generates the setting enumeration and every classification accessor from one
/// table. Each row must supply all six axes or the macro fails to parse.
macro_rules! settings {
    ($(
        $variant:ident => $key:literal, $class:ident, $boundary:ident,
        $secrecy:ident, $scope:ident, $kind:ident, [$($group:ident),*];
    )*) => {
        /// Every setting in the system.
        ///
        /// Generated from the table in this module. To add a setting, add a row;
        /// there is no way to add a variant without classifying it.
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub enum SettingId {
            $($variant,)*
        }

        impl SettingId {
            /// Every setting, generated from the same table as the enum, so the
            /// two cannot drift.
            pub const ALL: &'static [Self] = &[$(Self::$variant,)*];

            /// The durable key. Changing one of these is a migration.
            #[must_use]
            pub const fn key(self) -> &'static str {
                match self { $(Self::$variant => $key,)* }
            }

            #[must_use]
            pub const fn class(self) -> SettingClass {
                match self { $(Self::$variant => SettingClass::$class,)* }
            }

            #[must_use]
            pub const fn boundary(self) -> ApplicationBoundary {
                match self { $(Self::$variant => ApplicationBoundary::$boundary,)* }
            }

            #[must_use]
            pub const fn secrecy(self) -> Secrecy {
                match self { $(Self::$variant => Secrecy::$secrecy,)* }
            }

            #[must_use]
            pub const fn scope(self) -> Scope {
                match self { $(Self::$variant => Scope::$scope,)* }
            }

            #[must_use]
            pub const fn kind(self) -> ValueKind {
                match self { $(Self::$variant => ValueKind::$kind,)* }
            }

            /// The validation groups this setting participates in.
            ///
            /// A change touching any member re-validates the whole group
            /// against current effective values. Raising a queue capacity alone
            /// can breach the camera memory budget even though the field itself
            /// is a legal integer.
            #[must_use]
            pub const fn groups(self) -> &'static [ValidationGroup] {
                match self { $(Self::$variant => &[$(ValidationGroup::$group,)*],)* }
            }
        }
    };
}

settings! {
    // --- Global bootstrap ---------------------------------------------------
    ServerHealthBind =>
        "server.health_bind", Bootstrap, ProcessRestart, Public, Global, Text, [];
    StorageRecordingRoot =>
        "storage.recording_root", Bootstrap, ProcessRestart, Public, Global, Path, [];
    DatabaseUrl =>
        "database.url", Bootstrap, ProcessRestart, Secret, Global, Text, [];
    DatabaseMaxConnections =>
        "database.max_connections", Bootstrap, ProcessRestart, Public, Global, Unsigned, [];
    DatabaseAcquireTimeout =>
        "database.acquire_timeout", Bootstrap, ProcessRestart, Public, Global, Duration, [];
    SupervisionMaximumCameras =>
        "supervision.maximum_cameras", Bootstrap, ProcessRestart, Public, Global, Unsigned, [];
    SupervisionCommandQueueItems =>
        "supervision.command_queue_items", Bootstrap, ProcessRestart, Public, Global, Unsigned, [];
    ApiMutatingSurfaceEnabled =>
        "api.mutating_surface_enabled", Bootstrap, ProcessRestart, Public, Global, Boolean, [];
    AuditFingerprintSalt =>
        "audit.fingerprint_salt", Bootstrap, ProcessRestart, Secret, Global, Text, [];
    ConfigCachePath =>
        "config.cache_path", Bootstrap, ProcessRestart, Public, Global, Path, [];
    ConfigStaleRetryInterval =>
        "config.stale_retry_interval", Bootstrap, ProcessRestart, Public, Global, Duration, [];
    ConfigStaleRetryBudget =>
        "config.stale_retry_budget", Bootstrap, ProcessRestart, Public, Global, Duration, [];

    // --- Global operational -------------------------------------------------
    SupervisionCommandTimeout =>
        "supervision.command_timeout", Operational, ProcessRestart, Public, Global, Duration, [];
    ApiMaxRequestBytes =>
        "api.max_request_bytes", Operational, Immediate, Public, Global, Unsigned, [];
    ApiIoTimeout =>
        "api.io_timeout", Operational, Immediate, Public, Global, Duration, [];
    // Separate from `api.io_timeout` because it bounds a different thing. That
    // one bounds a request or a JSON response, which a client has no reason to
    // read slowly. A playback body is read by a media element, which stops
    // reading on purpose once its buffer is full and does not read again until
    // the buffer drains. At five seconds the server cancelled every browser
    // that paced itself -- the client saw a few seconds of footage and rewound
    // -- so this budget has to be sized for "the client is gone", not "the
    // client is buffering".
    ApiPlaybackStallTimeout =>
        "api.playback_stall_timeout", Operational, Immediate, Public, Global, Duration, [];
    RetentionMaxAge =>
        "retention.max_age", Operational, ProcessRestart, Public, Global, Duration, [Retention];
    RetentionProtectedRecent =>
        "retention.protected_recent", Operational, ProcessRestart, Public, Global, Duration, [Retention];
    RetentionBatchFiles =>
        "retention.batch_files", Operational, ProcessRestart, Public, Global, Unsigned, [];
    RetentionBatchBytes =>
        "retention.batch_bytes", Operational, ProcessRestart, Public, Global, Unsigned, [];
    RetentionInterval =>
        "retention.interval", Operational, ProcessRestart, Public, Global, Duration, [];
    StorageWatermarkWarning =>
        "storage.watermark_warning", Operational, ProcessRestart, Public, Global, Unsigned,
        [StorageWatermarks];
    StorageWatermarkCritical =>
        "storage.watermark_critical", Operational, ProcessRestart, Public, Global, Unsigned,
        [StorageWatermarks];
    StorageWatermarkEmergency =>
        "storage.watermark_emergency", Operational, ProcessRestart, Public, Global, Unsigned,
        [StorageWatermarks];
    StorageWatermarkHysteresis =>
        "storage.watermark_hysteresis", Operational, ProcessRestart, Public, Global, Unsigned,
        [StorageWatermarks];
    // Queue capacities are fixed at construction; resizing a bounded queue is a
    // media-plane change with its own overload semantics, not a settings change.
    ReconciliationQueueItems =>
        "reconciliation.queue_items", Operational, ProcessRestart, Public, Global, Unsigned, [];
    ReconciliationInterval =>
        "reconciliation.interval", Operational, ProcessRestart, Public, Global, Duration, [];
    ReconciliationBackoff =>
        "reconciliation.backoff", Operational, ProcessRestart, Public, Global, Duration, [];
    MetadataQueueItems =>
        "metadata.queue_items", Operational, ProcessRestart, Public, Global, Unsigned, [];
    MetadataFlushInterval =>
        "metadata.flush_interval", Operational, ProcessRestart, Public, Global, Duration, [];
    MetadataRetryBackoff =>
        "metadata.retry_backoff", Operational, ProcessRestart, Public, Global, Duration, [];
    PlaybackFfmpeg =>
        "playback.ffmpeg", Operational, ProcessRestart, Public, Global, Path, [];
    PlaybackProbeTimeout =>
        "playback.probe_timeout", Operational, ProcessRestart, Public, Global, Duration, [];
    LiveKitViewerTokenTtl =>
        "livekit.viewer_token_ttl", Operational, ProcessRestart, Public, Global, Duration, [];

    // --- Per-camera operational ---------------------------------------------
    CameraRtspEndpoint =>
        "camera.rtsp.endpoint", Operational, NextReconnect, Public, PerCamera, Text, [];
    CameraRtspUsername =>
        "camera.rtsp.username", Operational, NextReconnect, Secret, PerCamera, Text, [];
    CameraRtspPassword =>
        "camera.rtsp.password", Operational, NextReconnect, Secret, PerCamera, Text, [];
    CameraRtspConnectTimeout =>
        "camera.rtsp.connect_timeout", Operational, NextReconnect, Public, PerCamera, Duration, [];
    CameraRtspMaxAccessUnitBytes =>
        "camera.rtsp.max_access_unit_bytes", Operational, PipelineRestart, Public, PerCamera,
        Unsigned, [CameraMemoryBudget];
    CameraRtspMaxNalsPerAccessUnit =>
        "camera.rtsp.max_nals_per_access_unit", Operational, PipelineRestart, Public, PerCamera,
        Unsigned, [];
    CameraReconnectInitial =>
        "camera.reconnect.initial", Operational, NextReconnect, Public, PerCamera, Duration,
        [ReconnectPolicy];
    CameraReconnectMaximum =>
        "camera.reconnect.maximum", Operational, NextReconnect, Public, PerCamera, Duration,
        [ReconnectPolicy];
    CameraReconnectJitterBasisPoints =>
        "camera.reconnect.jitter_basis_points", Operational, NextReconnect, Public, PerCamera,
        Unsigned, [];
    CameraReconnectTeardownTimeout =>
        "camera.reconnect.teardown_timeout", Operational, NextReconnect, Public, PerCamera,
        Duration, [];
    CameraReconnectStableSessionReset =>
        "camera.reconnect.stable_session_reset", Operational, NextReconnect, Public, PerCamera,
        Duration, [];
    CameraVideoWidth =>
        "camera.video.width", Operational, PipelineRestart, Public, PerCamera, Unsigned,
        [CameraMemoryBudget];
    CameraVideoHeight =>
        "camera.video.height", Operational, PipelineRestart, Public, PerCamera, Unsigned,
        [CameraMemoryBudget];
    CameraQueueRecorderItems =>
        "camera.queue.recorder_items", Operational, PipelineRestart, Public, PerCamera, Unsigned,
        [CameraMemoryBudget];
    CameraQueueLiveItems =>
        "camera.queue.live_items", Operational, PipelineRestart, Public, PerCamera, Unsigned,
        [CameraMemoryBudget];
    CameraQueueLifecycleEventItems =>
        "camera.queue.lifecycle_event_items", Operational, PipelineRestart, Public, PerCamera,
        Unsigned, [];
    CameraRecordingSegmentDuration =>
        "camera.recording.segment_duration", Operational, NextSegment, Public, PerCamera, Duration,
        [];
    CameraRecordingFragmentDuration =>
        "camera.recording.fragment_duration", Operational, NextSegment, Public, PerCamera, Duration,
        [];
    CameraRecordingMaxSegmentBytes =>
        "camera.recording.max_segment_bytes", Operational, NextSegment, Public, PerCamera, Unsigned,
        [];
    CameraRecordingMaxFragmentBytes =>
        "camera.recording.max_fragment_bytes", Operational, NextSegment, Public, PerCamera,
        Unsigned, [CameraMemoryBudget];
    CameraRecordingMaxSegmentSamples =>
        "camera.recording.max_segment_samples", Operational, NextSegment, Public, PerCamera,
        Unsigned, [];
    CameraRecordingClockDriftTolerance =>
        "camera.recording.clock_drift_tolerance", Operational, PipelineRestart, Public, PerCamera,
        Duration, [];
    CameraRecordingFfprobe =>
        "camera.recording.ffprobe", Operational, PipelineRestart, Public, PerCamera, Path, [];
    CameraRecordingReconciliationScanEntries =>
        "camera.recording.reconciliation_scan_entries", Operational, PipelineRestart, Public, PerCamera,
        Unsigned, [];
    CameraMemoryMaxPipelineBytes =>
        "camera.memory.max_pipeline_bytes", Operational, PipelineRestart, Public, PerCamera,
        Unsigned, [CameraMemoryBudget];
    CameraDetectionEnabled =>
        "camera.detection.enabled", Operational, PipelineRestart, Public, PerCamera, Boolean,
        [CameraMemoryBudget];
    CameraDetectionQueueItems =>
        "camera.detection.queue_items", Operational, PipelineRestart, Public, PerCamera, Unsigned,
        [CameraMemoryBudget];
    CameraDetectionSampleFps =>
        "camera.detection.sample_fps", Operational, PipelineRestart, Public, PerCamera, Unsigned,
        [];
    CameraDetectionMaxGap =>
        "camera.detection.max_gap", Operational, PipelineRestart, Public, PerCamera, Duration, [];
    CameraDetectionDecodedFrameMaxBytes =>
        "camera.detection.decoded_frame_max_bytes", Operational, PipelineRestart, Public, PerCamera,
        Unsigned, [CameraMemoryBudget, DecodedFrameBudget];
    CameraDetectionDecoder =>
        "camera.detection.decoder", Operational, PipelineRestart, Public, PerCamera, Path, [];
    CameraDetectionDecodeTimeout =>
        "camera.detection.decode_timeout", Operational, PipelineRestart, Public, PerCamera,
        Duration, [];
    CameraDetectionOverlayMaxAge =>
        "camera.detection.overlay_max_age", Operational, PipelineRestart, Public, PerCamera, Duration, [];
    CameraDetectionPresenceConfirmation =>
        "camera.detection.presence_confirmation_duration", Operational, PipelineRestart, Public, PerCamera, Duration, [];
    CameraDetectionPresenceGrace =>
        "camera.detection.presence_grace_duration", Operational, PipelineRestart, Public, PerCamera, Duration, [];
    CameraDetectionPresenceFreshness =>
        "camera.detection.presence_freshness_duration", Operational, PipelineRestart, Public, PerCamera, Duration, [];
    CameraDetectionFaceSampleFps =>
        "camera.detection.face_sample_fps", Operational, PipelineRestart, Public, PerCamera, Unsigned, [];
    CameraDetectionInferenceConcurrency =>
        "detection.inference_concurrency", Operational, ProcessRestart, Public, Global, Unsigned, [];
    CameraDetectionMode =>
        "camera.detection.mode", Operational, PipelineRestart, Public, PerCamera, Text, [];
    CameraDetectionFaceModelPath =>
        "camera.detection.face_model_path", Operational, PipelineRestart, Public, PerCamera, Path, [];
    CameraDetectionFaceModelSha256 =>
        "camera.detection.face_model_sha256", Operational, PipelineRestart, Public, PerCamera, Text, [];
    CameraDetectionFaceScoreThresholdPercent =>
        "camera.detection.face_score_threshold_percent", Operational, PipelineRestart, Public, PerCamera, Unsigned, [];
    CameraDetectionModelPath =>
        "camera.detection.model_path", Operational, PipelineRestart, Public, PerCamera, Path, [];
    CameraDetectionModelSha256 =>
        "camera.detection.model_sha256", Operational, PipelineRestart, Public, PerCamera, Text, [];
    CameraDetectionInputWidth =>
        "camera.detection.input_width", Operational, PipelineRestart, Public, PerCamera, Unsigned,
        [CameraMemoryBudget, DecodedFrameBudget];
    CameraDetectionInputHeight =>
        "camera.detection.input_height", Operational, PipelineRestart, Public, PerCamera, Unsigned,
        [CameraMemoryBudget, DecodedFrameBudget];
    CameraDetectionExecutionProviders =>
        "camera.detection.execution_providers", Operational, PipelineRestart, Public, PerCamera,
        Text, [];
    CameraDetectionIntraOpThreads =>
        "camera.detection.intra_op_threads", Operational, PipelineRestart, Public, PerCamera,
        Unsigned, [];
    CameraDetectionInterOpThreads =>
        "camera.detection.inter_op_threads", Operational, PipelineRestart, Public, PerCamera,
        Unsigned, [];
    // PipelineRestart (not Immediate): `OnnxPersonInference` captures a
    // `DecodeThresholds` at construction (crates/detector/src/onnx.rs:134 and
    // :158) and reuses it in every call to `infer` (:371). Reclassifying
    // as Immediate without threading a live config source into `infer`
    // would make the config system report "applied immediately" for a knob
    // that only takes effect on restart — the specific lie the settings
    // domain is designed to avoid. Make the field live first, then flip.
    CameraDetectionScoreThresholdPercent =>
        "camera.detection.score_threshold_percent", Operational, PipelineRestart, Public, PerCamera,
        Unsigned, [];
    CameraDetectionNmsIouPercent =>
        "camera.detection.nms_iou_percent", Operational, PipelineRestart, Public, PerCamera, Unsigned,
        [];
    CameraDetectionMotionThresholdPercent =>
        "camera.detection.motion_threshold_percent", Operational, PipelineRestart, Public, PerCamera,
        Unsigned, [];
    CameraDetectionGateCooldown =>
        "camera.detection.gate_cooldown", Operational, PipelineRestart, Public, PerCamera, Duration,
        [];
    CameraDetectionGateHeartbeat =>
        "camera.detection.gate_heartbeat", Operational, PipelineRestart, Public, PerCamera, Duration,
        [];
    CameraDetectionHwaccel =>
        "camera.detection.hwaccel", Operational, PipelineRestart, Public, PerCamera, Text, [];
    CameraLiveKitEndpoint =>
        "camera.livekit.endpoint", Operational, PipelineRestart, Public, PerCamera, Text, [];
    CameraLiveKitApiKey =>
        "camera.livekit.api_key", Operational, PipelineRestart, Secret, PerCamera, Text, [];
    CameraLiveKitApiSecret =>
        "camera.livekit.api_secret", Operational, PipelineRestart, Secret, PerCamera, Text, [];
    CameraLiveKitDeploymentNamespace =>
        "camera.livekit.deployment_namespace", Operational, PipelineRestart, Public, PerCamera,
        Text, [];
    CameraLiveKitQueueItems =>
        "camera.livekit.queue_items", Operational, PipelineRestart, Public, PerCamera, Unsigned, [];
    CameraLiveKitMaxLag =>
        "camera.livekit.max_lag", Operational, PipelineRestart, Public, PerCamera, Duration, [];
    CameraLiveKitMaxAccessUnitBytes =>
        "camera.livekit.max_access_unit_bytes", Operational, PipelineRestart, Public, PerCamera,
        Unsigned, [];
    CameraLiveKitShutdownTimeout =>
        "camera.livekit.shutdown_timeout", Operational, PipelineRestart, Public, PerCamera,
        Duration, [];
    CameraLiveKitReconnectInitial =>
        "camera.livekit.reconnect_initial", Operational, PipelineRestart, Public, PerCamera,
        Duration, [];
    CameraLiveKitReconnectMaximum =>
        "camera.livekit.reconnect_maximum", Operational, PipelineRestart, Public, PerCamera,
        Duration, [];
    CameraLiveKitReconnectStableReset =>
        "camera.livekit.reconnect_stable_reset", Operational, PipelineRestart, Public, PerCamera,
        Duration, [];
    CameraLiveKitDecoder =>
        "camera.livekit.decoder", Operational, PipelineRestart, Public, PerCamera, Path, [];
    CameraLiveKitMaxPublishHeight =>
        "camera.livekit.max_publish_height", Operational, PipelineRestart, Public, PerCamera,
        Unsigned, [];
    CameraLiveKitViewerIdleGrace =>
        "camera.livekit.viewer_idle_grace", Operational, PipelineRestart, Public, PerCamera,
        Duration, [];
}

impl SettingId {
    /// Resolves a durable key back to its identity.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|id| id.key() == key)
    }

    /// Whether this setting may be written to the settings store.
    ///
    /// Secret-valued settings are not persisted in Phase 7. ADR 003 leaves the
    /// production secret store and rotation mechanism open, and writing
    /// credentials into the ordinary settings table would decide that question
    /// by accident. Until it is decided, secrets are supplied by the
    /// environment layer only.
    #[must_use]
    pub const fn persistable(self) -> bool {
        matches!(self.class(), SettingClass::Operational)
            && matches!(self.secrecy(), Secrecy::Public)
    }

    /// Whether this setting may be changed through the API.
    ///
    /// An API change writes the persisted layer, so a setting that cannot be
    /// persisted cannot be changed through the API either.
    #[must_use]
    pub const fn api_editable(self) -> bool {
        self.persistable()
    }
}

impl fmt::Display for SettingId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.key())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ApplicationBoundary, Scope, Secrecy, SettingClass, SettingId, ValidationGroup, ValueKind,
    };
    use std::collections::BTreeSet;

    #[test]
    fn keys_are_unique() {
        let unique: BTreeSet<&str> = SettingId::ALL.iter().map(|id| id.key()).collect();
        assert_eq!(unique.len(), SettingId::ALL.len(), "duplicate setting key");
    }

    #[test]
    fn every_key_round_trips_through_from_key() {
        for id in SettingId::ALL {
            assert_eq!(SettingId::from_key(id.key()), Some(*id));
        }
        assert_eq!(SettingId::from_key("no.such.setting"), None);
    }

    #[test]
    fn keys_are_namespaced_lowercase() {
        for id in SettingId::ALL {
            let key = id.key();
            assert!(!key.is_empty(), "empty key for {id:?}");
            assert!(key.contains('.'), "key {key} is not namespaced");
            assert!(
                key.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_'),
                "key {key} is not lowercase dotted snake case"
            );
        }
    }

    #[test]
    fn bootstrap_settings_are_process_restart_and_not_api_editable() {
        for id in SettingId::ALL {
            if id.class() == SettingClass::Bootstrap {
                assert_eq!(
                    id.boundary(),
                    ApplicationBoundary::ProcessRestart,
                    "{id} is bootstrap but claims a runtime boundary"
                );
                assert!(!id.api_editable(), "{id} is bootstrap but API-editable");
            }
        }
    }

    #[test]
    fn captured_detection_thresholds_and_gate_settings_require_pipeline_restart() {
        // All five are captured at pipeline construction (inference thresholds
        // in `OnnxPersonInference`, gate settings in `GatePolicy`) and are
        // never re-read for the life of the pipeline. Reclassifying to
        // Immediate without a code-path refactor would make the config system
        // report "applied immediately" for knobs that only take effect on
        // restart — the exact lie the settings domain is designed to avoid.
        for id in [
            SettingId::CameraDetectionScoreThresholdPercent,
            SettingId::CameraDetectionNmsIouPercent,
            SettingId::CameraDetectionMotionThresholdPercent,
            SettingId::CameraDetectionGateCooldown,
            SettingId::CameraDetectionGateHeartbeat,
        ] {
            assert_eq!(id.boundary(), ApplicationBoundary::PipelineRestart, "{id}");
        }
    }

    #[test]
    fn public_operational_settings_are_api_editable() {
        for id in SettingId::ALL {
            if id.class() == SettingClass::Operational && id.secrecy() == Secrecy::Public {
                assert!(id.api_editable(), "{id} is operational but not editable");
            }
        }
    }

    #[test]
    fn secrets_are_never_persistable_or_api_editable() {
        // Phase 7 deliberately does not persist credentials; the secret store
        // decision is still open in ADR 003.
        for id in SettingId::ALL {
            if id.secrecy() == Secrecy::Secret {
                assert!(!id.persistable(), "{id} would persist a credential");
                assert!(!id.api_editable(), "{id} would accept a credential by API");
            }
        }
    }

    #[test]
    fn scope_and_key_namespace_agree() {
        for id in SettingId::ALL {
            assert_eq!(
                id.scope() == Scope::PerCamera,
                id.key().starts_with("camera."),
                "{id} disagrees between scope and key namespace"
            );
        }
    }

    #[test]
    fn secret_settings_are_the_credential_bearing_ones() {
        let secrets: BTreeSet<&str> = SettingId::ALL
            .iter()
            .filter(|id| id.secrecy() == Secrecy::Secret)
            .map(|id| id.key())
            .collect();
        assert_eq!(
            secrets,
            BTreeSet::from([
                "audit.fingerprint_salt",
                "camera.livekit.api_key",
                "camera.livekit.api_secret",
                "camera.rtsp.password",
                "camera.rtsp.username",
                "database.url",
            ]),
            "the secret set changed; confirm the new setting genuinely holds \
             credential material, and that it is a Secret<T> everywhere, before \
             accepting this"
        );
    }

    #[test]
    fn watermark_group_has_exactly_four_members() {
        let members = SettingId::ALL
            .iter()
            .filter(|id| id.groups().contains(&ValidationGroup::StorageWatermarks))
            .count();
        assert_eq!(members, 4, "watermark group membership changed");
    }

    #[test]
    fn every_group_has_at_least_two_members() {
        for group in ValidationGroup::ALL {
            let count = SettingId::ALL
                .iter()
                .filter(|id| id.groups().contains(group))
                .count();
            assert!(
                count >= 2,
                "{group} has {count} member(s); a group of one is not a group"
            );
        }
    }

    #[test]
    fn grouped_settings_share_their_group_scope() {
        // A group spanning both global and per-camera settings could not be
        // validated against a single scope's effective values.
        for group in ValidationGroup::ALL {
            let scopes: BTreeSet<bool> = SettingId::ALL
                .iter()
                .filter(|id| id.groups().contains(group))
                .map(|id| id.scope() == Scope::PerCamera)
                .collect();
            assert_eq!(scopes.len(), 1, "{group} mixes global and per-camera scope");
        }
    }

    #[test]
    fn boundary_labels_are_distinct() {
        let labels: BTreeSet<&str> = ApplicationBoundary::ALL
            .iter()
            .map(|boundary| boundary.as_str())
            .collect();
        assert_eq!(labels.len(), ApplicationBoundary::ALL.len());
    }

    #[test]
    fn secret_settings_are_textual() {
        // A secret held as an integer or a boolean would be pointless to
        // protect, and fingerprinting relies on a byte representation.
        for id in SettingId::ALL {
            if id.secrecy() == Secrecy::Secret {
                assert_eq!(id.kind(), ValueKind::Text, "{id} is a non-textual secret");
            }
        }
    }

    #[test]
    fn duration_settings_are_named_for_time() {
        // Catches a mis-typed row: a duration whose key reads like a count is
        // almost always a copy-paste error in the table.
        for id in SettingId::ALL {
            if id.kind() == ValueKind::Duration {
                let key = id.key();
                assert!(
                    [
                        "timeout",
                        "interval",
                        "duration",
                        "backoff",
                        "ttl",
                        "lag",
                        "age",
                        "initial",
                        "maximum",
                        "reset",
                        "recent",
                        "budget",
                        "gap",
                        "tolerance",
                        "grace",
                        // The detection gate's vocabulary. Both words denote
                        // time as plainly as the rest of this list, so the
                        // guard is extended rather than the keys contorted.
                        "cooldown",
                        "heartbeat"
                    ]
                    .iter()
                    .any(|marker| key.contains(marker)),
                    "{key} is a duration but its name does not read like one"
                );
            }
        }
    }

    #[test]
    fn table_covers_the_documented_setting_count() {
        // Guards against an accidental deletion of a table row. The number is
        // the contracts table's row count; changing it is a deliberate act.
        assert_eq!(SettingId::ALL.len(), 103);
    }
}
