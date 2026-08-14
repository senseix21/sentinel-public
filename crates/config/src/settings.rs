//! Transactionally validated settings for the one-camera Phase 2 runtime.

use std::{error::Error, fmt, num::NonZeroUsize, path::PathBuf, time::Duration};

use surveillance_camera::{
    rtsp::{
        RtspCredentials, RtspCredentialsError, RtspEndpoint, RtspEndpointError, RtspFrameLimits,
        RtspFrameLimitsError, RtspSourceConfig, RtspSourceConfigError,
    },
    supervisor::{ReconnectPolicy, ReconnectPolicyError},
};
use surveillance_common::types::{CameraId, IdentifierError};
use surveillance_detector::sampler::{SampleRateHz, SamplerError};
use surveillance_livekit::config::{LiveKitConfig, LiveKitConfigError, LiveKitSettings};
use surveillance_media::distribution::{QueueCapacity, QueueCapacityError};
use surveillance_recorder::{
    clock::{ClockConfigError, ClockDriftTolerance},
    fmp4::{Fmp4ConfigError, Fmp4SinkConfig, SegmentDuration},
};
use surveillance_storage::{
    filesystem::{RecordingLayout, RecordingLayoutError},
    reconciliation::{ReconciliationConfigError, ReconciliationLimit},
};

/// Secret input kept separate from the endpoint and redacted by domain types.
pub struct CameraCredentialsSettings {
    pub username: String,
    pub password: String,
}

/// Complete unvalidated Phase 2 input. It is accepted only as one transaction.
pub struct OneCameraSettings {
    pub camera_id: String,
    pub rtsp_endpoint: String,
    pub credentials: Option<CameraCredentialsSettings>,
    pub connect_timeout: Duration,
    pub max_access_unit_bytes: usize,
    pub max_nals_per_access_unit: usize,
    pub reconnect_initial: Duration,
    pub reconnect_maximum: Duration,
    pub reconnect_jitter_basis_points: u16,
    pub teardown_timeout: Duration,
    pub reconnect_stable_session_reset: Duration,
    pub recorder_queue_items: usize,
    pub live_queue_items: usize,
    pub detection_enabled: bool,
    pub detection_queue_items: usize,
    pub detection_sample_fps: u16,
    pub detection_max_gap: Duration,
    pub decoded_frame_max_bytes: usize,
    pub detection_decoder: PathBuf,
    pub detection_decode_timeout: Duration,
    pub width: u32,
    pub height: u32,
    pub segment_duration: Duration,
    pub fragment_duration: Duration,
    pub max_segment_bytes: usize,
    pub max_fragment_bytes: usize,
    pub max_segment_samples: usize,
    pub max_pipeline_memory_bytes: usize,
    pub recording_root: PathBuf,
    pub ffprobe: PathBuf,
    pub clock_drift_tolerance: Duration,
    pub lifecycle_event_items: usize,
    pub reconciliation_scan_entries: usize,
    pub livekit: Option<LiveKitSettings>,
}

impl OneCameraSettings {
    /// Creates reversible H.264 MVP defaults around deployment-specific input.
    #[must_use]
    pub fn h264_mvp(
        camera_id: String,
        rtsp_endpoint: String,
        credentials: Option<CameraCredentialsSettings>,
        width: u32,
        height: u32,
        recording_root: PathBuf,
    ) -> Self {
        Self {
            camera_id,
            rtsp_endpoint,
            credentials,
            connect_timeout: Duration::from_secs(5),
            max_access_unit_bytes: 2 * 1024 * 1024,
            max_nals_per_access_unit: 128,
            reconnect_initial: Duration::from_millis(250),
            reconnect_maximum: Duration::from_secs(30),
            reconnect_jitter_basis_points: 2_000,
            teardown_timeout: Duration::from_secs(2),
            reconnect_stable_session_reset: Duration::from_secs(30),
            recorder_queue_items: 32,
            live_queue_items: 4,
            detection_enabled: true,
            detection_queue_items: 2,
            detection_sample_fps: 5,
            detection_max_gap: Duration::from_secs(2),
            decoded_frame_max_bytes: width as usize * height as usize * 3,
            detection_decoder: PathBuf::from("ffmpeg"),
            detection_decode_timeout: Duration::from_secs(2),
            width,
            height,
            segment_duration: Duration::from_secs(30),
            fragment_duration: Duration::from_secs(1),
            max_segment_bytes: 32 * 1024 * 1024,
            max_fragment_bytes: 8 * 1024 * 1024,
            max_segment_samples: 1_000,
            max_pipeline_memory_bytes: 128 * 1024 * 1024,
            recording_root,
            ffprobe: PathBuf::from("ffprobe"),
            clock_drift_tolerance: Duration::from_secs(5),
            lifecycle_event_items: 64,
            reconciliation_scan_entries: 100_000,
            livekit: None,
        }
    }
}

/// All validated domain configurations needed to construct one pipeline.
pub struct OneCameraConfig {
    pub camera_id: CameraId,
    pub source: RtspSourceConfig,
    pub reconnect: ReconnectPolicy,
    pub recorder_queue: QueueCapacity,
    pub live_queue: QueueCapacity,
    pub detection_enabled: bool,
    pub detection_queue: QueueCapacity,
    pub detection_sample_rate: SampleRateHz,
    pub detection_max_gap: Duration,
    pub decoded_frame_max_bytes: usize,
    pub detection_decoder: PathBuf,
    pub detection_decode_timeout: Duration,
    pub max_access_unit_bytes: usize,
    pub recorder: Fmp4SinkConfig,
    pub layout: RecordingLayout,
    pub ffprobe: PathBuf,
    pub clock_drift: ClockDriftTolerance,
    pub lifecycle_event_items: NonZeroUsize,
    pub reconciliation_limit: ReconciliationLimit,
    pub bounded_memory_bytes: usize,
    pub livekit: Option<LiveKitConfig>,
}

impl OneCameraConfig {
    /// Validates every field before exposing any runnable configuration.
    ///
    /// # Errors
    ///
    /// Returns the first typed invariant violation. The input is consumed and
    /// no partially validated configuration is returned.
    #[allow(
        clippy::too_many_lines,
        reason = "one transaction validates all coupled pipeline resource bounds before activation"
    )]
    pub fn validate(settings: OneCameraSettings) -> Result<Self, OneCameraConfigError> {
        let camera_id =
            CameraId::new(settings.camera_id).map_err(OneCameraConfigError::CameraId)?;
        let endpoint =
            RtspEndpoint::parse(&settings.rtsp_endpoint).map_err(OneCameraConfigError::Endpoint)?;
        let credentials = settings
            .credentials
            .map(|credentials| RtspCredentials::new(credentials.username, credentials.password))
            .transpose()
            .map_err(OneCameraConfigError::Credentials)?;
        let limits = RtspFrameLimits::new(
            settings.max_access_unit_bytes,
            settings.max_nals_per_access_unit,
        )
        .map_err(OneCameraConfigError::FrameLimits)?;
        let source = RtspSourceConfig::new(endpoint, credentials, settings.connect_timeout, limits)
            .and_then(|source| source.require_dimensions(settings.width, settings.height))
            .map_err(OneCameraConfigError::Source)?;
        let reconnect = ReconnectPolicy::new(
            settings.reconnect_initial,
            settings.reconnect_maximum,
            settings.reconnect_jitter_basis_points,
            settings.teardown_timeout,
            settings.reconnect_stable_session_reset,
        )
        .map_err(OneCameraConfigError::Reconnect)?;
        let recorder_queue = QueueCapacity::new(settings.recorder_queue_items)
            .map_err(OneCameraConfigError::RecorderQueue)?;
        let live_queue = QueueCapacity::new(settings.live_queue_items)
            .map_err(OneCameraConfigError::LiveQueue)?;
        let detection_queue = QueueCapacity::new(settings.detection_queue_items)
            .map_err(OneCameraConfigError::DetectionQueue)?;
        let detection_sample_rate = SampleRateHz::new(settings.detection_sample_fps)
            .map_err(OneCameraConfigError::DetectionSampleRate)?;
        if settings.detection_max_gap.is_zero() {
            return Err(OneCameraConfigError::ZeroDetectionMaximumGap);
        }
        if settings.detection_decoder.as_os_str().is_empty() {
            return Err(OneCameraConfigError::EmptyDetectionDecoder);
        }
        if settings.detection_decode_timeout.is_zero() {
            return Err(OneCameraConfigError::ZeroDetectionDecodeTimeout);
        }
        let required_decoded_bytes = usize::try_from(settings.width)
            .ok()
            .and_then(|width| width.checked_mul(settings.height as usize))
            .and_then(|pixels| pixels.checked_mul(3))
            .ok_or(OneCameraConfigError::MemoryBoundOverflow)?;
        if settings.decoded_frame_max_bytes < required_decoded_bytes {
            return Err(OneCameraConfigError::DecodedFrameBudgetTooSmall);
        }
        let duration = SegmentDuration::new(settings.segment_duration)
            .map_err(OneCameraConfigError::Recorder)?;
        let recorder = Fmp4SinkConfig::new_with_fragment_bounds(
            settings.width,
            settings.height,
            duration,
            settings.fragment_duration,
            settings.max_segment_bytes,
            settings.max_fragment_bytes,
            settings.max_segment_samples,
        )
        .map_err(OneCameraConfigError::Recorder)?;
        let queue_items = settings
            .recorder_queue_items
            .checked_add(settings.live_queue_items)
            .and_then(|items| {
                items.checked_add(if settings.detection_enabled {
                    settings.detection_queue_items
                } else {
                    0
                })
            })
            .ok_or(OneCameraConfigError::MemoryBoundOverflow)?;
        let queue_bytes = queue_items
            .checked_mul(settings.max_access_unit_bytes)
            .ok_or(OneCameraConfigError::MemoryBoundOverflow)?;
        let fragment_buffers = settings
            .max_fragment_bytes
            .checked_mul(2)
            .ok_or(OneCameraConfigError::MemoryBoundOverflow)?;
        let bounded_memory_bytes = queue_bytes
            .checked_add(fragment_buffers)
            .and_then(|bytes| {
                bytes.checked_add(if settings.detection_enabled {
                    settings.decoded_frame_max_bytes.checked_mul(2)?
                } else {
                    0
                })
            })
            .ok_or(OneCameraConfigError::MemoryBoundOverflow)?;
        if bounded_memory_bytes > settings.max_pipeline_memory_bytes {
            return Err(OneCameraConfigError::MemoryBudgetExceeded {
                required: bounded_memory_bytes,
                configured: settings.max_pipeline_memory_bytes,
            });
        }
        let layout = RecordingLayout::new(settings.recording_root)
            .map_err(OneCameraConfigError::RecordingRoot)?;
        if settings.ffprobe.as_os_str().is_empty() {
            return Err(OneCameraConfigError::EmptyFfprobe);
        }
        let clock_drift = ClockDriftTolerance::new(settings.clock_drift_tolerance)
            .map_err(OneCameraConfigError::Clock)?;
        let lifecycle_event_items = NonZeroUsize::new(settings.lifecycle_event_items)
            .ok_or(OneCameraConfigError::ZeroLifecycleEventItems)?;
        let reconciliation_limit = ReconciliationLimit::new(settings.reconciliation_scan_entries)
            .map_err(OneCameraConfigError::Reconciliation)?;
        let livekit = settings
            .livekit
            .map(|livekit| LiveKitConfig::validate(&camera_id, livekit))
            .transpose()
            .map_err(OneCameraConfigError::LiveKit)?;
        Ok(Self {
            camera_id,
            source,
            reconnect,
            recorder_queue,
            live_queue,
            detection_enabled: settings.detection_enabled,
            detection_queue,
            detection_sample_rate,
            detection_max_gap: settings.detection_max_gap,
            decoded_frame_max_bytes: settings.decoded_frame_max_bytes,
            detection_decoder: settings.detection_decoder,
            detection_decode_timeout: settings.detection_decode_timeout,
            max_access_unit_bytes: settings.max_access_unit_bytes,
            recorder,
            layout,
            ffprobe: settings.ffprobe,
            clock_drift,
            lifecycle_event_items,
            reconciliation_limit,
            bounded_memory_bytes,
            livekit,
        })
    }
}

#[derive(Debug)]
pub enum OneCameraConfigError {
    CameraId(IdentifierError),
    Endpoint(RtspEndpointError),
    Credentials(RtspCredentialsError),
    FrameLimits(RtspFrameLimitsError),
    Source(RtspSourceConfigError),
    Reconnect(ReconnectPolicyError),
    RecorderQueue(QueueCapacityError),
    LiveQueue(QueueCapacityError),
    DetectionQueue(QueueCapacityError),
    DetectionSampleRate(SamplerError),
    ZeroDetectionMaximumGap,
    DecodedFrameBudgetTooSmall,
    EmptyDetectionDecoder,
    ZeroDetectionDecodeTimeout,
    Recorder(Fmp4ConfigError),
    RecordingRoot(RecordingLayoutError),
    Clock(ClockConfigError),
    Reconciliation(ReconciliationConfigError),
    LiveKit(LiveKitConfigError),
    ZeroLifecycleEventItems,
    EmptyFfprobe,
    MemoryBoundOverflow,
    MemoryBudgetExceeded { required: usize, configured: usize },
}

impl fmt::Display for OneCameraConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid one-camera runtime configuration: {self:?}"
        )
    }
}

impl Error for OneCameraConfigError {}

#[cfg(test)]
mod tests {
    use super::{OneCameraConfig, OneCameraConfigError, OneCameraSettings};
    use std::{path::PathBuf, time::Duration};

    fn valid() -> OneCameraSettings {
        OneCameraSettings {
            camera_id: "camera-01".to_owned(),
            rtsp_endpoint: "rtsp://127.0.0.1:8554/camera-01".to_owned(),
            credentials: None,
            connect_timeout: Duration::from_secs(5),
            max_access_unit_bytes: 2 * 1024 * 1024,
            max_nals_per_access_unit: 128,
            reconnect_initial: Duration::from_millis(250),
            reconnect_maximum: Duration::from_secs(30),
            reconnect_jitter_basis_points: 2_000,
            teardown_timeout: Duration::from_secs(2),
            reconnect_stable_session_reset: Duration::from_secs(30),
            recorder_queue_items: 32,
            live_queue_items: 4,
            detection_enabled: true,
            detection_queue_items: 2,
            detection_sample_fps: 5,
            detection_max_gap: Duration::from_secs(2),
            decoded_frame_max_bytes: 2560 * 1440 * 3,
            detection_decoder: PathBuf::from("ffmpeg"),
            detection_decode_timeout: Duration::from_secs(2),
            width: 2560,
            height: 1440,
            segment_duration: Duration::from_secs(30),
            fragment_duration: Duration::from_secs(1),
            max_segment_bytes: 32 * 1024 * 1024,
            max_fragment_bytes: 8 * 1024 * 1024,
            max_segment_samples: 1_000,
            max_pipeline_memory_bytes: 128 * 1024 * 1024,
            recording_root: PathBuf::from("recordings"),
            ffprobe: PathBuf::from("ffprobe"),
            clock_drift_tolerance: Duration::from_secs(5),
            lifecycle_event_items: 64,
            reconciliation_scan_entries: 100_000,
            livekit: None,
        }
    }

    #[test]
    fn validates_complete_configuration_and_calculates_memory_bound() {
        let config = OneCameraConfig::validate(valid())
            .unwrap_or_else(|error| panic!("valid configuration: {error}"));
        assert_eq!(config.bounded_memory_bytes, 118_587_392);
        assert_eq!(config.recorder_queue.get(), 32);
        assert_eq!(config.live_queue.get(), 4);
    }

    #[test]
    fn rejects_transaction_when_combined_memory_exceeds_budget() {
        let mut settings = valid();
        settings.max_pipeline_memory_bytes = 80 * 1024 * 1024;
        assert!(matches!(
            OneCameraConfig::validate(settings),
            Err(OneCameraConfigError::MemoryBudgetExceeded {
                required: 118_587_392,
                configured: 83_886_080,
            })
        ));
    }

    #[test]
    fn rejects_secrets_embedded_in_endpoint() {
        let mut settings = valid();
        settings.rtsp_endpoint = "rtsp://user:secret@127.0.0.1/camera".to_owned();
        assert!(matches!(
            OneCameraConfig::validate(settings),
            Err(OneCameraConfigError::Endpoint(_))
        ));
    }
}
