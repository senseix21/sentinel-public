//! Environment boundary for required deployment values and secrets.

use std::{
    error::Error,
    fmt,
    net::SocketAddr,
    num::{NonZeroU32, NonZeroUsize},
    path::PathBuf,
    time::Duration,
};

use surveillance_camera::{
    ffmpeg::{FfmpegCaptureBackend, FfmpegSourceConfig, WebcamInput},
    source::H264SourceConfig,
};
use surveillance_config::{
    classification::{Origin, SettingId},
    defaults, environment, file,
    resolution::{Layer, ResolvedSettings},
    settings::{CameraCredentialsSettings, OneCameraSettings},
    value::SettingValue,
};
use surveillance_database::DatabaseConfig;
use surveillance_livekit::config::LiveKitSettings;
use surveillance_storage::retention::RetentionPolicy;

const CAMERA_ID: &str = "SURVEILLANCE_CAMERA_ID";
const RTSP_URL: &str = "SURVEILLANCE_RTSP_URL";
const RTSP_USERNAME: &str = "SURVEILLANCE_RTSP_USERNAME";
const RTSP_PASSWORD: &str = "SURVEILLANCE_RTSP_PASSWORD";
const WIDTH: &str = "SURVEILLANCE_VIDEO_WIDTH";
const HEIGHT: &str = "SURVEILLANCE_VIDEO_HEIGHT";
const RECORDING_ROOT: &str = "SURVEILLANCE_RECORDING_ROOT";
const HEALTH_BIND: &str = "SURVEILLANCE_HEALTH_BIND";
const DEFAULT_HEALTH_BIND: &str = "127.0.0.1:3000";
const MAXIMUM_CAMERAS: &str = "SURVEILLANCE_MAXIMUM_CAMERAS";
const COMMAND_QUEUE_ITEMS: &str = "SURVEILLANCE_COMMAND_QUEUE_ITEMS";
const COMMAND_TIMEOUT_SECONDS: &str = "SURVEILLANCE_COMMAND_TIMEOUT_SECONDS";
const DATABASE_URL: &str = "SURVEILLANCE_DATABASE_URL";
const DATABASE_MAX_CONNECTIONS: &str = "SURVEILLANCE_DATABASE_MAX_CONNECTIONS";
const DATABASE_ACQUIRE_TIMEOUT_SECONDS: &str = "SURVEILLANCE_DATABASE_ACQUIRE_TIMEOUT_SECONDS";
// The SURVEILLANCE_RETENTION_* variables are no longer read here: they reach
// retention through the alias table in `surveillance_config::environment`,
// which is what let the hardcoded watermarks be removed.
const FFMPEG: &str = "SURVEILLANCE_FFMPEG";
const LIVEKIT_URL: &str = "SURVEILLANCE_LIVEKIT_URL";
const LIVEKIT_API_KEY: &str = "SURVEILLANCE_LIVEKIT_API_KEY";
const LIVEKIT_API_SECRET: &str = "SURVEILLANCE_LIVEKIT_API_SECRET";
const LIVEKIT_NAMESPACE: &str = "SURVEILLANCE_LIVEKIT_NAMESPACE";
const DETECTION_ENABLED: &str = "SURVEILLANCE_DETECTION_ENABLED";
const DETECTION_QUEUE_ITEMS: &str = "SURVEILLANCE_DETECTION_QUEUE_ITEMS";
const DETECTION_SAMPLE_FPS: &str = "SURVEILLANCE_DETECTION_SAMPLE_FPS";
const DETECTION_MAX_GAP_MS: &str = "SURVEILLANCE_DETECTION_MAX_GAP_MS";
// ONNX backend selection. `MODEL_PATH` empty or unset keeps the fixture
// backend; a non-empty value switches to the real backend and requires
// `MODEL_SHA256` to match the file on disk.
const DETECTION_MODEL_PATH: &str = "SURVEILLANCE_DETECTION_MODEL_PATH";
const DETECTION_MODEL_SHA256: &str = "SURVEILLANCE_DETECTION_MODEL_SHA256";
// Score and NMS thresholds, percent (0..=100). The default score threshold
// is 70%, as selected for local operation. These apply to real inference
// only; the fixture backend ignores them. Explicit configuration overrides
// the default.
const DETECTION_SCORE_THRESHOLD_PERCENT: &str = "SURVEILLANCE_DETECTION_SCORE_THRESHOLD_PERCENT";
const DETECTION_NMS_IOU_PERCENT: &str = "SURVEILLANCE_DETECTION_NMS_IOU_PERCENT";
// Model input geometry. YOLOX-Tiny expects 416, YOLOX-S/M/L expect 640.
// Must match the model's declared input shape; a mismatch is a hard startup
// failure, not a quality issue.
const DETECTION_INPUT_WIDTH: &str = "SURVEILLANCE_DETECTION_INPUT_WIDTH";
const DETECTION_INPUT_HEIGHT: &str = "SURVEILLANCE_DETECTION_INPUT_HEIGHT";
// Normalized rectangle "x,y,w,h" in 0..=1 whose detections are dropped
// before they reach the tracker. Format "0.0,0.0,0.3,1.0" excludes the
// leftmost 30% of the frame. All four values must parse, all must be in
// 0..=1, and left+width/top+height must not exceed 1.0.
const DETECTION_ROI_EXCLUDE: &str = "SURVEILLANCE_DETECTION_ROI_EXCLUDE";
const CONFIG_FILE: &str = "SURVEILLANCE_CONFIG_FILE";
const WEBCAM_DEVICE: &str = "SURVEILLANCE_WEBCAM_DEVICE";
const WEBCAM_BACKEND: &str = "SURVEILLANCE_WEBCAM_BACKEND";
const WEBCAM_FPS: &str = "SURVEILLANCE_WEBCAM_FPS";
const WEBCAM_BITRATE_KBPS: &str = "SURVEILLANCE_WEBCAM_BITRATE_KBPS";
const WEBCAM_KEYFRAME_INTERVAL: &str = "SURVEILLANCE_WEBCAM_KEYFRAME_INTERVAL";
const WEBCAM_STARTUP_TIMEOUT_SECONDS: &str = "SURVEILLANCE_WEBCAM_STARTUP_TIMEOUT_SECONDS";
const WEBCAM_READ_TIMEOUT_SECONDS: &str = "SURVEILLANCE_WEBCAM_READ_TIMEOUT_SECONDS";
const WEBCAM_PARSER_BYTES: &str = "SURVEILLANCE_WEBCAM_PARSER_BYTES";

/// The bootstrap layers: compiled defaults, an optional file, and the
/// environment.
///
/// The persisted layer is absent here on purpose. It loads after bootstrap,
/// because reaching the settings store requires values from these layers.
pub struct ConfigurationLayers {
    /// Defaults and file, in precedence order.
    pub base: Vec<Layer>,
    /// Highest precedence, and the only source of secrets.
    pub environment: Layer,
    /// Bootstrap values resolved from `base` plus `environment`.
    pub bootstrap: ResolvedSettings,
}

/// Builds the bootstrap layers.
///
/// The configuration file is optional and its path comes from
/// `SURVEILLANCE_CONFIG_FILE`. A file that is named but unreadable, or that
/// contains an unknown key, fails startup rather than being skipped: a
/// configuration file that silently does nothing is the failure this avoids.
///
/// # Errors
/// Returns a typed error naming the offending file or variable.
pub fn load_configuration_layers() -> Result<ConfigurationLayers, EnvironmentConfigError> {
    let mut base = vec![defaults::layer()];
    if let Some(path) = std::env::var(CONFIG_FILE)
        .ok()
        .filter(|path| !path.is_empty())
    {
        let layer = file::load(Origin::File, std::path::Path::new(&path))
            .map_err(|error| EnvironmentConfigError::InvalidConfigurationFile(error.to_string()))?;
        base.push(layer);
    }
    let environment = environment::layer()
        .map_err(|error| EnvironmentConfigError::InvalidEnvironment(error.to_string()))?;

    let mut layers = base.clone();
    layers.push(environment.clone());
    let bootstrap = ResolvedSettings::resolve(layers);

    Ok(ConfigurationLayers {
        base,
        environment,
        bootstrap,
    })
}

/// Reads a bootstrap boolean, defaulting to `false` when absent.
#[must_use]
pub fn bootstrap_flag(resolved: &ResolvedSettings, id: SettingId) -> bool {
    matches!(
        resolved
            .effective(id, None)
            .map(|value| value.value().clone()),
        Some(SettingValue::Boolean(true))
    )
}

/// Builds retention settings from the resolved configuration.
///
/// Replaces the environment-only path that hardcoded
/// `StorageWatermarks::new(80, 90, 97, 2)`. The four thresholds and the
/// retention policy now come from the configuration domain, and the legacy
/// `SURVEILLANCE_RETENTION_*` variables still reach them through the alias
/// table, so an existing deployment is unaffected.
///
/// # Errors
/// Returns [`EnvironmentConfigError::InvalidRetentionConfiguration`] when the
/// resolved values are not a legal policy.
pub fn load_retention(
    resolved: &ResolvedSettings,
) -> Result<RetentionSettings, EnvironmentConfigError> {
    let policy = surveillance_config::assemble::retention_policy(resolved)
        .map_err(|_| EnvironmentConfigError::InvalidRetentionConfiguration)?;
    let interval = bootstrap_duration(resolved, SettingId::RetentionInterval)
        .ok_or(EnvironmentConfigError::InvalidRetentionConfiguration)?;
    Ok(RetentionSettings { policy, interval })
}

/// Reads a bootstrap count, falling back when absent or out of range.
#[must_use]
pub fn bootstrap_count(resolved: &ResolvedSettings, id: SettingId, fallback: usize) -> usize {
    match resolved
        .effective(id, None)
        .map(|value| value.value().clone())
    {
        Some(SettingValue::Unsigned(value)) => usize::try_from(value).unwrap_or(fallback),
        _ => fallback,
    }
}

/// Reads a bootstrap percentage, falling back when absent or out of range.
#[must_use]
pub fn bootstrap_percent(resolved: &ResolvedSettings, id: SettingId, fallback: u8) -> u8 {
    match resolved
        .effective(id, None)
        .map(|value| value.value().clone())
    {
        Some(SettingValue::Unsigned(value)) => u8::try_from(value).unwrap_or(fallback),
        _ => fallback,
    }
}

/// Reads a bootstrap path.
#[must_use]
pub fn bootstrap_path(resolved: &ResolvedSettings, id: SettingId) -> Option<PathBuf> {
    match resolved.effective(id, None)?.value() {
        SettingValue::Path(path) => Some(path.clone()),
        _ => None,
    }
}

/// Reads a bootstrap duration.
#[must_use]
pub fn bootstrap_duration(resolved: &ResolvedSettings, id: SettingId) -> Option<Duration> {
    match resolved.effective(id, None)?.value() {
        SettingValue::Duration(value) => Some(*value),
        _ => None,
    }
}

/// Reads a bootstrap secret without rendering it.
#[must_use]
pub fn bootstrap_secret(
    resolved: &ResolvedSettings,
    id: SettingId,
) -> Option<surveillance_config::secret::Secret<String>> {
    match resolved.effective(id, None)?.value() {
        SettingValue::Secret(value) => Some(value.clone()),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SupervisionSettings {
    pub maximum_cameras: NonZeroUsize,
    pub command_queue_items: NonZeroUsize,
    pub command_timeout: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetentionSettings {
    pub policy: RetentionPolicy,
    pub interval: Duration,
}

/// Loads required deployment values without ever providing a public camera URL.
///
/// # Errors
///
/// Returns a typed error for missing, malformed, or incomplete environment
/// configuration.
pub fn load_from_environment(
    resolved: &ResolvedSettings,
) -> Result<OneCameraSettings, EnvironmentConfigError> {
    load_with(|key| std::env::var(key).ok(), resolved)
}

/// Loads the loopback-only Phase 2 health bind address.
///
/// # Errors
///
/// Returns an error for an invalid address or a non-loopback interface.
pub fn load_health_bind_from_environment() -> Result<SocketAddr, EnvironmentConfigError> {
    load_health_bind_with(|key| std::env::var(key).ok())
}

/// Loads bounded supervision settings with versioned Phase 3 defaults.
///
/// # Errors
///
/// Returns an error when an override is not a positive integer.
pub fn load_supervision_from_environment() -> Result<SupervisionSettings, EnvironmentConfigError> {
    load_supervision_with(|key| std::env::var(key).ok())
}

/// Loads optional Phase 4 database connectivity. Absence keeps legacy media
/// fixtures runnable but is reported as degraded by the application owner.
///
/// # Errors
/// Returns a typed, credential-free configuration failure.
pub fn load_database_from_environment() -> Result<Option<DatabaseConfig>, EnvironmentConfigError> {
    load_database_with(|key| std::env::var(key).ok())
}

#[must_use]
pub fn load_ffmpeg_from_environment() -> PathBuf {
    std::env::var(FFMPEG).map_or_else(|_| PathBuf::from("ffmpeg"), PathBuf::from)
}

/// Builds an optional typed local-webcam source. Absence preserves the direct
/// RTSP path. The API accepts no arbitrary `FFmpeg` arguments.
///
/// # Errors
/// Returns a credential-free configuration error for an invalid backend,
/// numeric bound, or source invariant.
pub fn load_webcam_source_from_environment(
    width: u32,
    height: u32,
    max_access_unit_bytes: usize,
    max_nals_per_access_unit: usize,
) -> Result<Option<H264SourceConfig>, EnvironmentConfigError> {
    load_webcam_source_with(
        |key| std::env::var(key).ok(),
        width,
        height,
        max_access_unit_bytes,
        max_nals_per_access_unit,
    )
}

fn load_webcam_source_with(
    lookup: impl Fn(&str) -> Option<String>,
    width: u32,
    height: u32,
    max_access_unit_bytes: usize,
    max_nals_per_access_unit: usize,
) -> Result<Option<H264SourceConfig>, EnvironmentConfigError> {
    let Some(device) = lookup(WEBCAM_DEVICE).filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };
    let backend_name = lookup(WEBCAM_BACKEND).unwrap_or_else(default_webcam_backend);
    let backend = match backend_name.as_str() {
        "avfoundation" => FfmpegCaptureBackend::AvFoundation,
        "v4l2" => FfmpegCaptureBackend::Video4Linux2,
        "dshow" => FfmpegCaptureBackend::DirectShow,
        _ => return Err(EnvironmentConfigError::InvalidWebcamBackend),
    };
    let fps = positive_u32_override(&lookup, WEBCAM_FPS, 15)?;
    let bitrate_kbps = positive_u32_override(&lookup, WEBCAM_BITRATE_KBPS, 1_500)?;
    let keyframe_interval = positive_u32_override(&lookup, WEBCAM_KEYFRAME_INTERVAL, fps)?;
    let startup_timeout = positive_u64_override(&lookup, WEBCAM_STARTUP_TIMEOUT_SECONDS, 15)?;
    let read_timeout = positive_u64_override(&lookup, WEBCAM_READ_TIMEOUT_SECONDS, 5)?;
    let minimum_parser_bytes = max_access_unit_bytes
        .checked_add(8)
        .ok_or(EnvironmentConfigError::InvalidWebcamConfiguration)?;
    let parser_bytes = lookup(WEBCAM_PARSER_BYTES).map_or(Ok(minimum_parser_bytes), |value| {
        value
            .parse::<usize>()
            .ok()
            .filter(|value| *value != 0)
            .ok_or(EnvironmentConfigError::InvalidPositiveInteger(
                WEBCAM_PARSER_BYTES,
            ))
    })?;
    let input = WebcamInput::new(backend, device)
        .map_err(|_| EnvironmentConfigError::InvalidWebcamConfiguration)?;
    FfmpegSourceConfig::webcam(
        lookup(FFMPEG).map_or_else(|| PathBuf::from("ffmpeg"), PathBuf::from),
        input,
        width,
        height,
        fps,
        bitrate_kbps,
        keyframe_interval,
        Duration::from_secs(startup_timeout),
        Duration::from_secs(read_timeout),
        max_access_unit_bytes,
        max_nals_per_access_unit,
        parser_bytes,
    )
    .map(H264SourceConfig::from)
    .map(Some)
    .map_err(|_| EnvironmentConfigError::InvalidWebcamConfiguration)
}

fn default_webcam_backend() -> String {
    if cfg!(target_os = "macos") {
        "avfoundation".to_owned()
    } else if cfg!(target_os = "linux") {
        "v4l2".to_owned()
    } else if cfg!(target_os = "windows") {
        "dshow".to_owned()
    } else {
        "unsupported".to_owned()
    }
}

fn positive_u32_override(
    lookup: &impl Fn(&str) -> Option<String>,
    key: &'static str,
    default: u32,
) -> Result<u32, EnvironmentConfigError> {
    lookup(key).map_or(Ok(default), |value| {
        value
            .parse::<u32>()
            .ok()
            .filter(|value| *value != 0)
            .ok_or(EnvironmentConfigError::InvalidPositiveInteger(key))
    })
}

fn positive_u64_override(
    lookup: &impl Fn(&str) -> Option<String>,
    key: &'static str,
    default: u64,
) -> Result<u64, EnvironmentConfigError> {
    lookup(key).map_or(Ok(default), |value| {
        value
            .parse::<u64>()
            .ok()
            .filter(|value| *value != 0)
            .ok_or(EnvironmentConfigError::InvalidPositiveInteger(key))
    })
}

fn load_database_with(
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<Option<DatabaseConfig>, EnvironmentConfigError> {
    let Some(dsn) = lookup(DATABASE_URL) else {
        return Ok(None);
    };
    let maximum = lookup(DATABASE_MAX_CONNECTIONS).map_or(Ok(8_u32), |value| {
        value.parse::<u32>().ok().filter(|value| *value != 0).ok_or(
            EnvironmentConfigError::InvalidPositiveInteger(DATABASE_MAX_CONNECTIONS),
        )
    })?;
    let timeout = lookup(DATABASE_ACQUIRE_TIMEOUT_SECONDS).map_or(Ok(5_u64), |value| {
        value.parse::<u64>().ok().filter(|value| *value != 0).ok_or(
            EnvironmentConfigError::InvalidPositiveInteger(DATABASE_ACQUIRE_TIMEOUT_SECONDS),
        )
    })?;
    let maximum = NonZeroU32::new(maximum).ok_or(
        EnvironmentConfigError::InvalidPositiveInteger(DATABASE_MAX_CONNECTIONS),
    )?;
    DatabaseConfig::new(&dsn, maximum, Duration::from_secs(timeout))
        .map(Some)
        .map_err(|_| EnvironmentConfigError::InvalidDatabaseConfiguration)
}

fn load_supervision_with(
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<SupervisionSettings, EnvironmentConfigError> {
    let maximum_cameras = positive_usize_override(&lookup, MAXIMUM_CAMERAS, 256)?;
    let command_queue_items = positive_usize_override(&lookup, COMMAND_QUEUE_ITEMS, 64)?;
    let timeout = positive_usize_override(&lookup, COMMAND_TIMEOUT_SECONDS, 10)?;
    Ok(SupervisionSettings {
        maximum_cameras,
        command_queue_items,
        command_timeout: Duration::from_secs(timeout.get() as u64),
    })
}

/// Parses a comma-separated `x,y,w,h` rectangle in normalized coordinates.
/// Returns `None` for empty / missing / malformed input, or for geometry that
/// escapes the unit square. A rectangle that is partially outside the frame
/// would silently clip detections no reasonable person would expect to clip,
/// so we refuse it here rather than clamp.
fn parse_roi_exclude(
    lookup: &impl Fn(&str) -> Option<String>,
    key: &'static str,
) -> Option<[f32; 4]> {
    let raw = lookup(key).filter(|value| !value.is_empty())?;
    let parts: Vec<f32> = raw
        .split(',')
        .map(|p| p.trim().parse::<f32>())
        .collect::<Result<_, _>>()
        .ok()?;
    if parts.len() != 4 {
        return None;
    }
    let [x, y, w, h] = [parts[0], parts[1], parts[2], parts[3]];
    if !(0.0..=1.0).contains(&x)
        || !(0.0..=1.0).contains(&y)
        || !(0.0..=1.0).contains(&w)
        || !(0.0..=1.0).contains(&h)
        || x + w > 1.0 + f32::EPSILON
        || y + h > 1.0 + f32::EPSILON
        || w <= 0.0
        || h <= 0.0
    {
        return None;
    }
    Some([x, y, w, h])
}

/// Parses an env var holding a value in `0..=100`. Empty, missing, or
/// malformed input returns `None` so the caller preserves the existing value
/// — a bad override is better off ignored than silently defaulted.
fn parse_percent_u8(lookup: &impl Fn(&str) -> Option<String>, key: &'static str) -> Option<u8> {
    lookup(key)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<u8>().ok())
        .filter(|value| *value <= 100)
}

fn positive_usize_override(
    lookup: &impl Fn(&str) -> Option<String>,
    key: &'static str,
    default: usize,
) -> Result<NonZeroUsize, EnvironmentConfigError> {
    let value = match lookup(key) {
        Some(value) => value.parse::<usize>().ok(),
        None => Some(default),
    };
    value
        .and_then(NonZeroUsize::new)
        .ok_or(EnvironmentConfigError::InvalidPositiveInteger(key))
}

fn load_health_bind_with(
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<SocketAddr, EnvironmentConfigError> {
    let value = lookup(HEALTH_BIND).unwrap_or_else(|| DEFAULT_HEALTH_BIND.to_owned());
    let address = value
        .parse::<SocketAddr>()
        .map_err(|_| EnvironmentConfigError::InvalidSocketAddress(HEALTH_BIND))?;
    if !address.ip().is_loopback() {
        return Err(EnvironmentConfigError::HealthBindNotLoopback);
    }
    Ok(address)
}

#[allow(
    clippy::too_many_lines,
    reason = "one env-var manifest keeps the layering single-source"
)]
fn load_with(
    lookup: impl Fn(&str) -> Option<String>,
    resolved: &ResolvedSettings,
) -> Result<OneCameraSettings, EnvironmentConfigError> {
    let camera_id = required(&lookup, CAMERA_ID)?;
    let endpoint = required(&lookup, RTSP_URL)?;
    let width_value = required(&lookup, WIDTH)?;
    let height_value = required(&lookup, HEIGHT)?;
    let width = parse_u32(&width_value, WIDTH)?;
    let height = parse_u32(&height_value, HEIGHT)?;
    let recording_root =
        lookup(RECORDING_ROOT).map_or_else(|| PathBuf::from("recordings"), PathBuf::from);
    let credentials = match (lookup(RTSP_USERNAME), lookup(RTSP_PASSWORD)) {
        (None, None) => None,
        (Some(username), Some(password)) => Some(CameraCredentialsSettings { username, password }),
        _ => return Err(EnvironmentConfigError::IncompleteCredentials),
    };
    let mut settings = OneCameraSettings::h264_mvp(
        camera_id,
        endpoint,
        credentials,
        width,
        height,
        recording_root,
    );
    settings.detection_enabled = match lookup(DETECTION_ENABLED).as_deref() {
        None | Some("true" | "1") => true,
        Some("false" | "0") => false,
        Some(_) => return Err(EnvironmentConfigError::InvalidBoolean(DETECTION_ENABLED)),
    };
    settings.detection_queue_items =
        positive_usize_override(&lookup, DETECTION_QUEUE_ITEMS, 2)?.get();
    settings.detection_sample_fps =
        u16::try_from(positive_usize_override(&lookup, DETECTION_SAMPLE_FPS, 5)?.get())
            .map_err(|_| EnvironmentConfigError::InvalidPositiveInteger(DETECTION_SAMPLE_FPS))?;
    settings.detection_max_gap = Duration::from_millis(
        u64::try_from(positive_usize_override(&lookup, DETECTION_MAX_GAP_MS, 2_000)?.get())
            .unwrap_or(u64::MAX),
    );
    settings.detection_decoder =
        lookup(FFMPEG).map_or_else(|| PathBuf::from("ffmpeg"), PathBuf::from);
    // Promote the fixture backend to the real ONNX one when both env vars are
    // set and non-empty. Either one alone keeps the fixture: a path without a
    // SHA cannot be verified, and a SHA without a path has nothing to apply to.
    let camera =
        surveillance_common::types::CameraId::new(settings.camera_id.clone()).map_err(|_| {
            EnvironmentConfigError::InvalidEnvironment("invalid camera identifier".to_owned())
        })?;
    if let Some(value) = resolved.effective(SettingId::CameraDetectionOverlayMaxAge, Some(&camera))
    {
        if let SettingValue::Duration(age) = value.value() {
            settings.detection_overlay_max_age = *age;
        }
    }
    for (id, target) in [
        (
            SettingId::CameraDetectionPresenceConfirmation,
            &mut settings.detection_presence_confirmation,
        ),
        (
            SettingId::CameraDetectionPresenceGrace,
            &mut settings.detection_presence_grace,
        ),
        (
            SettingId::CameraDetectionPresenceFreshness,
            &mut settings.detection_presence_freshness,
        ),
    ] {
        if let Some(value) = resolved.effective(id, Some(&camera)) {
            if let SettingValue::Duration(age) = value.value() {
                *target = *age;
            }
        }
    }
    if let Some(value) = resolved.effective(SettingId::CameraDetectionFaceSampleFps, Some(&camera))
    {
        if let SettingValue::Unsigned(n) = value.value() {
            settings.detection_face_sample_fps = u16::try_from(*n).map_err(|_| {
                EnvironmentConfigError::InvalidEnvironment(
                    "face sample FPS out of range".to_owned(),
                )
            })?;
        }
    }
    if let Some(value) = resolved.effective(SettingId::CameraDetectionInferenceConcurrency, None) {
        if let SettingValue::Unsigned(n) = value.value() {
            settings.detection_inference_concurrency = usize::try_from(*n).map_err(|_| {
                EnvironmentConfigError::InvalidEnvironment(
                    "inference concurrency out of range".to_owned(),
                )
            })?;
        }
    }
    let configured_text = |id| {
        resolved
            .effective(id, Some(&camera))
            .and_then(|value| match value.value() {
                SettingValue::Text(text) => Some(text.clone()),
                _ => None,
            })
    };
    let configured_path = |id| {
        resolved
            .effective(id, Some(&camera))
            .and_then(|value| match value.value() {
                SettingValue::Path(path) => Some(path.clone()),
                _ => None,
            })
    };
    settings.detection_mode = lookup("SURVEILLANCE_DETECTION_MODE")
        .or_else(|| configured_text(SettingId::CameraDetectionMode))
        .unwrap_or_else(|| "person".to_owned());
    settings.detection_face_model_path = lookup("SURVEILLANCE_DETECTION_FACE_MODEL_PATH")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| configured_path(SettingId::CameraDetectionFaceModelPath));
    settings.detection_face_model_sha256 = lookup("SURVEILLANCE_DETECTION_FACE_MODEL_SHA256")
        .filter(|s| !s.is_empty())
        .or_else(|| configured_text(SettingId::CameraDetectionFaceModelSha256));
    if let Some(value) = lookup("SURVEILLANCE_DETECTION_FACE_SCORE_THRESHOLD_PERCENT") {
        settings.detection_face_score_threshold_percent =
            value.parse::<u8>().ok().filter(|p| *p <= 100).ok_or(
                EnvironmentConfigError::InvalidEnvironment(
                    "face confidence must be 0..=100".to_owned(),
                ),
            )?;
    } else if let Some(value) = resolved.effective(
        SettingId::CameraDetectionFaceScoreThresholdPercent,
        Some(&camera),
    ) {
        if let SettingValue::Unsigned(percent) = value.value() {
            settings.detection_face_score_threshold_percent = u8::try_from(*percent)
                .ok()
                .filter(|p| *p <= 100)
                .ok_or(EnvironmentConfigError::InvalidEnvironment(
                    "face confidence must be 0..=100".to_owned(),
                ))?;
        }
    }
    let model_path = lookup(DETECTION_MODEL_PATH).filter(|s| !s.is_empty());
    let model_sha256 = lookup(DETECTION_MODEL_SHA256).filter(|s| !s.is_empty());
    if let (Some(path), Some(sha)) = (model_path, model_sha256) {
        settings.detection_model_path = Some(PathBuf::from(path));
        settings.detection_model_sha256 = Some(sha);
    }
    if let Some(percent) = parse_percent_u8(&lookup, DETECTION_SCORE_THRESHOLD_PERCENT) {
        settings.detection_score_threshold_percent = percent;
    }
    if let Some(percent) = parse_percent_u8(&lookup, DETECTION_NMS_IOU_PERCENT) {
        settings.detection_nms_iou_percent = percent;
    }
    if settings.detection_mode == "face" {
        settings.detection_input_width = 640;
        settings.detection_input_height = 640;
    }
    if let Some(value) = lookup(DETECTION_INPUT_WIDTH).and_then(|v| v.parse::<u32>().ok()) {
        settings.detection_input_width = value;
    }
    if let Some(value) = lookup(DETECTION_INPUT_HEIGHT).and_then(|v| v.parse::<u32>().ok()) {
        settings.detection_input_height = value;
    }
    // The decoded-frame budget must cover one full model-input frame.
    // Raising the input geometry past the default forces the budget up with
    // it; otherwise YOLOX-S/M at 640x640 fails startup with
    // `DecodedFrameBudgetTooSmall`. We raise, never lower — a user that
    // explicitly sets a smaller input is not punished for it.
    let needed = if settings.detection_mode == "person_face" {
        640 * 640 * 3
    } else {
        u64::from(settings.detection_input_width) * u64::from(settings.detection_input_height) * 3
    };
    if (settings.decoded_frame_max_bytes as u64) < needed {
        if let Ok(needed_usize) = usize::try_from(needed) {
            settings.decoded_frame_max_bytes = needed_usize;
        }
    }
    if let Some(rect) = parse_roi_exclude(&lookup, DETECTION_ROI_EXCLUDE) {
        settings.detection_roi_exclude = Some(rect);
    }
    settings.livekit = match (
        lookup(LIVEKIT_URL),
        lookup(LIVEKIT_API_KEY),
        lookup(LIVEKIT_API_SECRET),
    ) {
        (None, None, None) => None,
        (Some(endpoint), Some(api_key), Some(api_secret)) => Some(LiveKitSettings {
            endpoint,
            api_key,
            api_secret,
            deployment_namespace: lookup(LIVEKIT_NAMESPACE)
                .unwrap_or_else(|| "surveillance".to_owned()),
            // These came from literals that shadowed the configuration domain:
            // `camera.livekit.*` was classified, defaulted, validated, and
            // audited, but nothing read it, so an operator could persist a
            // change and watch it do nothing.
            //
            // Identity and credentials deliberately stay on the environment
            // path above: they are secrets.
            queue_items: bootstrap_count(resolved, SettingId::CameraLiveKitQueueItems, 8),
            max_lag: bootstrap_duration(resolved, SettingId::CameraLiveKitMaxLag)
                .unwrap_or_else(|| Duration::from_secs(2)),
            max_access_unit_bytes: bootstrap_count(
                resolved,
                SettingId::CameraLiveKitMaxAccessUnitBytes,
                2 * 1024 * 1024,
            ),
            shutdown_timeout: bootstrap_duration(resolved, SettingId::CameraLiveKitShutdownTimeout)
                .unwrap_or_else(|| Duration::from_secs(5)),
            reconnect_initial: bootstrap_duration(
                resolved,
                SettingId::CameraLiveKitReconnectInitial,
            )
            .unwrap_or_else(|| Duration::from_millis(250)),
            reconnect_maximum: bootstrap_duration(
                resolved,
                SettingId::CameraLiveKitReconnectMaximum,
            )
            .unwrap_or_else(|| Duration::from_secs(30)),
            reconnect_stable_reset: bootstrap_duration(
                resolved,
                SettingId::CameraLiveKitReconnectStableReset,
            )
            .unwrap_or_else(|| Duration::from_secs(30)),
            // Live publication transcodes (ADR 012). The decoder shares the
            // detection path's default binary but is a separate setting: the
            // two have different lifetimes and a deployment may well point
            // them at different builds.
            decoder_program: bootstrap_path(resolved, SettingId::CameraLiveKitDecoder)
                .map_or_else(|| "ffmpeg".to_owned(), |path| path.display().to_string()),
            max_publish_height: u32::try_from(bootstrap_count(
                resolved,
                SettingId::CameraLiveKitMaxPublishHeight,
                0,
            ))
            .unwrap_or(u32::MAX),
            viewer_idle_grace: bootstrap_duration(
                resolved,
                SettingId::CameraLiveKitViewerIdleGrace,
            )
            .unwrap_or_else(|| Duration::from_secs(5)),
        }),
        _ => return Err(EnvironmentConfigError::IncompleteLiveKitCredentials),
    };
    Ok(settings)
}

fn required(
    lookup: &impl Fn(&str) -> Option<String>,
    key: &'static str,
) -> Result<String, EnvironmentConfigError> {
    lookup(key)
        .filter(|value| !value.is_empty())
        .ok_or(EnvironmentConfigError::Missing(key))
}

fn parse_u32(value: &str, key: &'static str) -> Result<u32, EnvironmentConfigError> {
    value
        .parse::<u32>()
        .ok()
        .filter(|value| *value != 0)
        .ok_or(EnvironmentConfigError::InvalidPositiveInteger(key))
}

// No longer `Copy`: the file and environment variants carry the detail an
// operator needs to find the offending key, which is worth more than the
// convenience of implicit copying.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EnvironmentConfigError {
    Missing(&'static str),
    InvalidBoolean(&'static str),
    InvalidPositiveInteger(&'static str),
    IncompleteCredentials,
    IncompleteLiveKitCredentials,
    InvalidSocketAddress(&'static str),
    HealthBindNotLoopback,
    InvalidDatabaseConfiguration,
    InvalidRetentionConfiguration,
    InvalidWebcamBackend,
    InvalidWebcamConfiguration,
    /// A named configuration file that is unreadable, malformed, or contains an
    /// unknown key. Startup fails rather than skipping it.
    InvalidConfigurationFile(String),
    /// An environment layer that is ambiguous or malformed.
    InvalidEnvironment(String),
}

impl fmt::Display for EnvironmentConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(key) => {
                write!(formatter, "required environment variable {key} is missing")
            }
            Self::InvalidPositiveInteger(key) => {
                write!(
                    formatter,
                    "environment variable {key} must be a positive integer"
                )
            }
            Self::InvalidBoolean(key) => {
                write!(
                    formatter,
                    "environment variable {key} must be true, false, 1, or 0"
                )
            }
            Self::IncompleteCredentials => formatter
                .write_str("RTSP username and password must either both be set or both be absent"),
            Self::IncompleteLiveKitCredentials => formatter.write_str(
                "LiveKit URL, API key, and API secret must either all be set or all be absent",
            ),
            Self::InvalidSocketAddress(key) => {
                write!(
                    formatter,
                    "environment variable {key} must be a socket address"
                )
            }
            Self::HealthBindNotLoopback => {
                formatter.write_str("Phase 2 health endpoint must bind to a loopback address")
            }
            Self::InvalidDatabaseConfiguration => {
                formatter.write_str("database connection configuration is invalid")
            }
            Self::InvalidConfigurationFile(detail) => {
                write!(formatter, "configuration file is invalid: {detail}")
            }
            Self::InvalidEnvironment(detail) => {
                write!(formatter, "environment configuration is invalid: {detail}")
            }
            Self::InvalidRetentionConfiguration => {
                formatter.write_str("retention configuration is invalid")
            }
            Self::InvalidWebcamBackend => {
                formatter.write_str("webcam backend must be avfoundation, v4l2, or dshow")
            }
            Self::InvalidWebcamConfiguration => {
                formatter.write_str("webcam source configuration is invalid")
            }
        }
    }
}

impl Error for EnvironmentConfigError {}

#[cfg(test)]
mod tests {
    use super::{
        defaults, load_database_with, load_health_bind_with, load_retention, load_supervision_with,
        load_webcam_source_with, load_with, EnvironmentConfigError, ResolvedSettings,
    };
    use std::{collections::HashMap, time::Duration};

    fn compiled_defaults() -> ResolvedSettings {
        ResolvedSettings::resolve(vec![defaults::layer()])
    }

    /// The `LiveKit` publisher's tuning must come from the configuration domain,
    /// not from literals in this file.
    ///
    /// This exists because those literals silently shadowed `camera.livekit.*`
    /// for its whole life: the settings were classified, defaulted, validated,
    /// and audited, and the publisher never read one of them. (The literal
    /// `ipc_write_timeout` was 250 ms against a declared default of 2 s; that
    /// setting has since been retired with the child-process publisher it
    /// belonged to.)
    #[test]
    fn livekit_tuning_comes_from_configuration_not_literals() {
        let base = HashMap::from([
            ("SURVEILLANCE_CAMERA_ID", "camera-01"),
            ("SURVEILLANCE_RTSP_URL", "rtsp://127.0.0.1/camera"),
            ("SURVEILLANCE_VIDEO_WIDTH", "640"),
            ("SURVEILLANCE_VIDEO_HEIGHT", "360"),
            ("SURVEILLANCE_LIVEKIT_URL", "ws://127.0.0.1:7880"),
            ("SURVEILLANCE_LIVEKIT_API_KEY", "devkey"),
            ("SURVEILLANCE_LIVEKIT_API_SECRET", "secret"),
        ]);
        let settings = load_with(
            |key| base.get(key).map(|value| (*value).to_owned()),
            &compiled_defaults(),
        )
        .unwrap_or_else(|error| panic!("settings: {error}"));
        let livekit = settings.livekit.expect("livekit configured");

        // Every one of these is a value the declared default disagrees with the
        // old literal about, so a regression to literals fails here.
        assert_eq!(
            livekit.queue_items, 8,
            "queue_items must be configured, not the old 4"
        );
        assert_eq!(livekit.max_lag, Duration::from_secs(2));
        assert_eq!(livekit.shutdown_timeout, Duration::from_secs(5));
        assert_eq!(livekit.viewer_idle_grace, Duration::from_secs(5));

        // Identity stays on the environment path.
        assert_eq!(livekit.endpoint, "ws://127.0.0.1:7880");
    }

    #[test]
    fn requires_deployment_identity_source_and_dimensions() {
        let values = HashMap::<&str, &str>::new();
        assert_eq!(
            load_with(
                |key| values.get(key).map(|value| (*value).to_owned()),
                &compiled_defaults()
            )
            .err(),
            Some(EnvironmentConfigError::Missing("SURVEILLANCE_CAMERA_ID"))
        );
    }

    #[test]
    fn rejects_half_configured_credentials() {
        let values = HashMap::from([
            ("SURVEILLANCE_CAMERA_ID", "camera-01"),
            ("SURVEILLANCE_RTSP_URL", "rtsp://127.0.0.1/camera"),
            ("SURVEILLANCE_VIDEO_WIDTH", "1920"),
            ("SURVEILLANCE_VIDEO_HEIGHT", "1080"),
            ("SURVEILLANCE_RTSP_USERNAME", "operator"),
        ]);
        assert_eq!(
            load_with(
                |key| values.get(key).map(|value| (*value).to_owned()),
                &compiled_defaults()
            )
            .err(),
            Some(EnvironmentConfigError::IncompleteCredentials)
        );
    }

    #[test]
    fn detection_defaults_and_restart_applied_overrides_are_explicit() {
        let base = HashMap::from([
            ("SURVEILLANCE_CAMERA_ID", "camera-01"),
            ("SURVEILLANCE_RTSP_URL", "rtsp://127.0.0.1/camera"),
            ("SURVEILLANCE_VIDEO_WIDTH", "640"),
            ("SURVEILLANCE_VIDEO_HEIGHT", "360"),
            ("SURVEILLANCE_DETECTION_ENABLED", "false"),
            ("SURVEILLANCE_DETECTION_QUEUE_ITEMS", "3"),
            ("SURVEILLANCE_DETECTION_SAMPLE_FPS", "7"),
        ]);
        let settings = load_with(
            |key| base.get(key).map(|value| (*value).to_owned()),
            &compiled_defaults(),
        )
        .unwrap_or_else(|error| panic!("settings: {error}"));
        assert!(!settings.detection_enabled);
        assert_eq!(settings.detection_queue_items, 3);
        assert_eq!(settings.detection_sample_fps, 7);
    }

    #[test]
    fn model_path_and_sha_promote_the_real_backend_when_both_are_set() {
        let values = HashMap::from([
            ("SURVEILLANCE_CAMERA_ID", "camera-01"),
            ("SURVEILLANCE_RTSP_URL", "rtsp://127.0.0.1/camera"),
            ("SURVEILLANCE_VIDEO_WIDTH", "640"),
            ("SURVEILLANCE_VIDEO_HEIGHT", "360"),
            (
                "SURVEILLANCE_DETECTION_MODEL_PATH",
                "/opt/models/yolox.onnx",
            ),
            (
                "SURVEILLANCE_DETECTION_MODEL_SHA256",
                "427cc366d34e27ff7a03e2899b5e3671425c262ea2291f88bb942bc1cc70b0f7",
            ),
        ]);
        let settings = load_with(
            |key| values.get(key).map(|value| (*value).to_owned()),
            &compiled_defaults(),
        )
        .unwrap_or_else(|error| panic!("settings: {error}"));
        assert_eq!(
            settings.detection_model_path.as_deref(),
            Some(std::path::Path::new("/opt/models/yolox.onnx"))
        );
        assert_eq!(
            settings.detection_model_sha256.as_deref(),
            Some("427cc366d34e27ff7a03e2899b5e3671425c262ea2291f88bb942bc1cc70b0f7"),
        );
    }

    #[test]
    fn model_path_without_sha_keeps_the_fixture_backend() {
        let values = HashMap::from([
            ("SURVEILLANCE_CAMERA_ID", "camera-01"),
            ("SURVEILLANCE_RTSP_URL", "rtsp://127.0.0.1/camera"),
            ("SURVEILLANCE_VIDEO_WIDTH", "640"),
            ("SURVEILLANCE_VIDEO_HEIGHT", "360"),
            (
                "SURVEILLANCE_DETECTION_MODEL_PATH",
                "/opt/models/yolox.onnx",
            ),
        ]);
        let settings = load_with(
            |key| values.get(key).map(|value| (*value).to_owned()),
            &compiled_defaults(),
        )
        .unwrap_or_else(|error| panic!("settings: {error}"));
        assert!(
            settings.detection_model_path.is_none(),
            "a path without a verifying sha must not take effect"
        );
    }

    #[test]
    fn score_threshold_env_overrides_the_default() {
        let values = HashMap::from([
            ("SURVEILLANCE_CAMERA_ID", "camera-01"),
            ("SURVEILLANCE_RTSP_URL", "rtsp://127.0.0.1/camera"),
            ("SURVEILLANCE_VIDEO_WIDTH", "640"),
            ("SURVEILLANCE_VIDEO_HEIGHT", "360"),
            ("SURVEILLANCE_DETECTION_SCORE_THRESHOLD_PERCENT", "65"),
            ("SURVEILLANCE_DETECTION_NMS_IOU_PERCENT", "55"),
        ]);
        let settings = load_with(
            |key| values.get(key).map(|value| (*value).to_owned()),
            &compiled_defaults(),
        )
        .unwrap_or_else(|error| panic!("settings: {error}"));
        assert_eq!(settings.detection_score_threshold_percent, 65);
        assert_eq!(settings.detection_nms_iou_percent, 55);
    }

    #[test]
    fn out_of_range_threshold_is_ignored_rather_than_clamped() {
        let values = HashMap::from([
            ("SURVEILLANCE_CAMERA_ID", "camera-01"),
            ("SURVEILLANCE_RTSP_URL", "rtsp://127.0.0.1/camera"),
            ("SURVEILLANCE_VIDEO_WIDTH", "640"),
            ("SURVEILLANCE_VIDEO_HEIGHT", "360"),
            ("SURVEILLANCE_DETECTION_SCORE_THRESHOLD_PERCENT", "150"),
            ("SURVEILLANCE_DETECTION_NMS_IOU_PERCENT", "garbage"),
        ]);
        let baseline = compiled_defaults();
        let settings = load_with(
            |key| values.get(key).map(|value| (*value).to_owned()),
            &baseline,
        )
        .unwrap_or_else(|error| panic!("settings: {error}"));
        // Defaults preserved, not silently zeroed.
        assert_ne!(settings.detection_score_threshold_percent, 0);
        assert_ne!(settings.detection_nms_iou_percent, 0);
    }

    #[test]
    fn valid_roi_exclude_env_populates_the_setting() {
        let values = HashMap::from([
            ("SURVEILLANCE_CAMERA_ID", "camera-01"),
            ("SURVEILLANCE_RTSP_URL", "rtsp://127.0.0.1/camera"),
            ("SURVEILLANCE_VIDEO_WIDTH", "640"),
            ("SURVEILLANCE_VIDEO_HEIGHT", "360"),
            ("SURVEILLANCE_DETECTION_ROI_EXCLUDE", "0.0,0.0,0.3,1.0"),
        ]);
        let settings = load_with(
            |key| values.get(key).map(|value| (*value).to_owned()),
            &compiled_defaults(),
        )
        .unwrap_or_else(|error| panic!("settings: {error}"));
        let rect = settings
            .detection_roi_exclude
            .expect("roi should parse from valid env");
        assert!((rect[0] - 0.0).abs() < 1e-6);
        assert!((rect[2] - 0.3).abs() < 1e-6);
        assert!((rect[3] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn roi_outside_unit_square_is_ignored_rather_than_clamped() {
        let values = HashMap::from([
            ("SURVEILLANCE_CAMERA_ID", "camera-01"),
            ("SURVEILLANCE_RTSP_URL", "rtsp://127.0.0.1/camera"),
            ("SURVEILLANCE_VIDEO_WIDTH", "640"),
            ("SURVEILLANCE_VIDEO_HEIGHT", "360"),
            ("SURVEILLANCE_DETECTION_ROI_EXCLUDE", "0.5,0.5,0.8,0.8"),
        ]);
        let settings = load_with(
            |key| values.get(key).map(|value| (*value).to_owned()),
            &compiled_defaults(),
        )
        .unwrap_or_else(|error| panic!("settings: {error}"));
        assert!(
            settings.detection_roi_exclude.is_none(),
            "rect that reaches past 1.0 must be refused, not clamped"
        );
    }

    #[test]
    fn malformed_roi_env_is_ignored() {
        for raw in ["", "a,b,c,d", "0.1,0.2", "0.1,0.2,0.3,0.4,0.5"] {
            let values = HashMap::from([
                ("SURVEILLANCE_CAMERA_ID", "camera-01"),
                ("SURVEILLANCE_RTSP_URL", "rtsp://127.0.0.1/camera"),
                ("SURVEILLANCE_VIDEO_WIDTH", "640"),
                ("SURVEILLANCE_VIDEO_HEIGHT", "360"),
                ("SURVEILLANCE_DETECTION_ROI_EXCLUDE", raw),
            ]);
            let settings = load_with(
                |key| values.get(key).map(|value| (*value).to_owned()),
                &compiled_defaults(),
            )
            .unwrap_or_else(|error| panic!("settings: {error}"));
            assert!(
                settings.detection_roi_exclude.is_none(),
                "malformed roi \"{raw}\" must be ignored"
            );
        }
    }

    #[test]
    fn empty_model_path_env_var_is_the_same_as_unset() {
        let values = HashMap::from([
            ("SURVEILLANCE_CAMERA_ID", "camera-01"),
            ("SURVEILLANCE_RTSP_URL", "rtsp://127.0.0.1/camera"),
            ("SURVEILLANCE_VIDEO_WIDTH", "640"),
            ("SURVEILLANCE_VIDEO_HEIGHT", "360"),
            ("SURVEILLANCE_DETECTION_MODEL_PATH", ""),
            ("SURVEILLANCE_DETECTION_MODEL_SHA256", ""),
        ]);
        let settings = load_with(
            |key| values.get(key).map(|value| (*value).to_owned()),
            &compiled_defaults(),
        )
        .unwrap_or_else(|error| panic!("settings: {error}"));
        assert!(settings.detection_model_path.is_none());
    }

    #[test]
    fn health_defaults_to_loopback_and_rejects_public_binding() {
        let default = load_health_bind_with(|_| None)
            .unwrap_or_else(|error| panic!("default health bind: {error}"));
        assert!(default.ip().is_loopback());
        assert_eq!(
            load_health_bind_with(|_| Some("0.0.0.0:3000".to_owned())).err(),
            Some(EnvironmentConfigError::HealthBindNotLoopback)
        );
    }

    #[test]
    fn supervision_defaults_are_bounded_and_overrides_require_positive_integers() {
        let defaults = load_supervision_with(|_| None)
            .unwrap_or_else(|error| panic!("valid defaults: {error}"));
        assert_eq!(defaults.maximum_cameras.get(), 256);
        assert_eq!(defaults.command_queue_items.get(), 64);
        assert_eq!(defaults.command_timeout.as_secs(), 10);
        assert_eq!(
            load_supervision_with(|key| {
                (key == "SURVEILLANCE_MAXIMUM_CAMERAS").then(|| "0".to_owned())
            })
            .err(),
            Some(EnvironmentConfigError::InvalidPositiveInteger(
                "SURVEILLANCE_MAXIMUM_CAMERAS"
            ))
        );
    }

    #[test]
    fn database_configuration_is_optional_bounded_and_secret_safe() {
        assert!(load_database_with(|_| None)
            .unwrap_or_else(|error| panic!("absent database is valid: {error}"))
            .is_none());
        let values = HashMap::from([(
            "SURVEILLANCE_DATABASE_URL",
            "postgres://operator:secret@localhost/surveillance",
        )]);
        let config = load_database_with(|key| values.get(key).map(|value| (*value).to_owned()))
            .unwrap_or_else(|error| panic!("valid database config: {error}"))
            .unwrap_or_else(|| panic!("database config must be present"));
        let debug = format!("{config:?}");
        assert!(!debug.contains("operator"));
        assert!(!debug.contains("secret"));
    }

    /// Ported from the deleted environment-only retention loader, which
    /// hardcoded `StorageWatermarks::new(80, 90, 97, 2)`. The same two claims
    /// are asserted through the configuration domain: versioned defaults, and a
    /// rejected invalid override.
    #[test]
    fn retention_defaults_are_versioned_and_invalid_overrides_are_rejected() {
        let resolved = ResolvedSettings::resolve(vec![defaults::layer()]);
        let settings =
            load_retention(&resolved).unwrap_or_else(|error| panic!("valid defaults: {error}"));
        assert_eq!(
            settings.policy.retention(),
            Duration::from_secs(30 * 24 * 60 * 60)
        );
        assert_eq!(settings.policy.maximum_files(), 100);
        assert_eq!(settings.interval, Duration::from_secs(60));

        // A zero retention age is rejected, and the rejection still names the
        // legacy variable that supplied it.
        let overrides = std::collections::BTreeMap::from([(
            "SURVEILLANCE_RETENTION_DAYS".to_owned(),
            "0".to_owned(),
        )]);
        let environment =
            surveillance_config::environment::layer_from(|key| overrides.get(key).cloned())
                .unwrap_or_else(|error| panic!("the layer itself parses: {error}"));
        let with_override = ResolvedSettings::resolve(vec![defaults::layer(), environment]);
        assert_eq!(
            load_retention(&with_override).err(),
            Some(EnvironmentConfigError::InvalidRetentionConfiguration),
            "a zero retention age must not become active"
        );
    }

    #[test]
    fn storage_watermarks_come_from_configuration_and_match_the_previous_constants() {
        // The values this replaces were `StorageWatermarks::new(80, 90, 97, 2)`
        // hardcoded in the retention loader. Making them configurable must not
        // have changed the shipped behavior.
        let resolved = ResolvedSettings::resolve(vec![defaults::layer()]);
        let watermarks = surveillance_config::assemble::storage_watermarks(&resolved)
            .unwrap_or_else(|error| panic!("shipped watermarks are legal: {error}"));
        let previous = surveillance_storage::retention::StorageWatermarks::new(80, 90, 97, 2)
            .unwrap_or_else(|error| panic!("previous constants were legal: {error:?}"));
        assert_eq!(watermarks, previous);
    }

    #[test]
    fn webcam_source_is_opt_in_and_typed() {
        assert!(load_webcam_source_with(|_| None, 640, 360, 1024, 16)
            .expect("absent webcam")
            .is_none());
        let values = HashMap::from([
            ("SURVEILLANCE_WEBCAM_DEVICE", "Camera Name With Spaces"),
            ("SURVEILLANCE_WEBCAM_BACKEND", "dshow"),
            ("SURVEILLANCE_WEBCAM_FPS", "20"),
        ]);
        let source = load_webcam_source_with(
            |key| values.get(key).map(|value| (*value).to_owned()),
            640,
            360,
            1024,
            16,
        )
        .expect("valid webcam")
        .expect("configured webcam");
        assert_eq!(
            source.kind(),
            surveillance_camera::source::H264SourceKind::FfmpegWebcam
        );
    }
}
