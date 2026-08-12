//! Environment boundary for required deployment values and secrets.

use std::{
    error::Error,
    fmt,
    net::SocketAddr,
    num::{NonZeroU32, NonZeroU64, NonZeroUsize},
    path::PathBuf,
    time::Duration,
};

use surveillance_config::settings::{CameraCredentialsSettings, OneCameraSettings};
use surveillance_database::DatabaseConfig;
use surveillance_livekit::config::LiveKitSettings;
use surveillance_storage::retention::{RetentionPolicy, StorageWatermarks};

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
const RETENTION_DAYS: &str = "SURVEILLANCE_RETENTION_DAYS";
const RETENTION_PROTECTED_HOURS: &str = "SURVEILLANCE_RETENTION_PROTECTED_HOURS";
const RETENTION_BATCH_FILES: &str = "SURVEILLANCE_RETENTION_BATCH_FILES";
const RETENTION_BATCH_BYTES: &str = "SURVEILLANCE_RETENTION_BATCH_BYTES";
const RETENTION_INTERVAL_SECONDS: &str = "SURVEILLANCE_RETENTION_INTERVAL_SECONDS";
const FFMPEG: &str = "SURVEILLANCE_FFMPEG";
const LIVEKIT_URL: &str = "SURVEILLANCE_LIVEKIT_URL";
const LIVEKIT_API_KEY: &str = "SURVEILLANCE_LIVEKIT_API_KEY";
const LIVEKIT_API_SECRET: &str = "SURVEILLANCE_LIVEKIT_API_SECRET";
const LIVEKIT_NAMESPACE: &str = "SURVEILLANCE_LIVEKIT_NAMESPACE";
const LIVEKIT_CLI: &str = "SURVEILLANCE_LIVEKIT_CLI";
const LIVEKIT_CLI_VERSION: &str = "SURVEILLANCE_LIVEKIT_CLI_VERSION";

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

/// Loads restart-applied versioned retention defaults and overrides.
///
/// # Errors
/// Returns a typed error for malformed or unsafe values.
pub fn load_retention_from_environment() -> Result<RetentionSettings, EnvironmentConfigError> {
    load_retention_with(|key| std::env::var(key).ok())
}

fn load_retention_with(
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<RetentionSettings, EnvironmentConfigError> {
    let positive = |key: &'static str, default: u64| -> Result<u64, EnvironmentConfigError> {
        lookup(key).map_or(Ok(default), |value| {
            value
                .parse::<u64>()
                .ok()
                .filter(|value| *value != 0)
                .ok_or(EnvironmentConfigError::InvalidPositiveInteger(key))
        })
    };
    let days = positive(RETENTION_DAYS, 30)?;
    let protected_hours = positive(RETENTION_PROTECTED_HOURS, 1)?;
    let files = usize::try_from(positive(RETENTION_BATCH_FILES, 100)?)
        .ok()
        .and_then(NonZeroUsize::new)
        .ok_or(EnvironmentConfigError::InvalidPositiveInteger(
            RETENTION_BATCH_FILES,
        ))?;
    let bytes = NonZeroU64::new(positive(RETENTION_BATCH_BYTES, 10 * 1024 * 1024 * 1024)?).ok_or(
        EnvironmentConfigError::InvalidPositiveInteger(RETENTION_BATCH_BYTES),
    )?;
    let interval = Duration::from_secs(positive(RETENTION_INTERVAL_SECONDS, 60)?);
    let watermarks = StorageWatermarks::new(80, 90, 97, 2)
        .map_err(|_| EnvironmentConfigError::InvalidRetentionConfiguration)?;
    let policy = RetentionPolicy::new(
        Duration::from_secs(days.saturating_mul(24 * 60 * 60)),
        Duration::from_secs(protected_hours.saturating_mul(60 * 60)),
        files,
        bytes,
        watermarks,
    )
    .map_err(|_| EnvironmentConfigError::InvalidRetentionConfiguration)?;
    Ok(RetentionSettings { policy, interval })
}

/// Loads required deployment values without ever providing a public camera URL.
///
/// # Errors
///
/// Returns a typed error for missing, malformed, or incomplete environment
/// configuration.
pub fn load_from_environment() -> Result<OneCameraSettings, EnvironmentConfigError> {
    load_with(|key| std::env::var(key).ok())
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

fn load_with(
    lookup: impl Fn(&str) -> Option<String>,
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
            cli_path: lookup(LIVEKIT_CLI).map_or_else(|| PathBuf::from("lk"), PathBuf::from),
            cli_version: lookup(LIVEKIT_CLI_VERSION).unwrap_or_else(|| "2.18.2".to_owned()),
            queue_items: 4,
            max_lag: Duration::from_millis(500),
            max_access_unit_bytes: 2 * 1024 * 1024,
            ipc_write_timeout: Duration::from_millis(250),
            shutdown_timeout: Duration::from_secs(2),
            reconnect_initial: Duration::from_millis(250),
            reconnect_maximum: Duration::from_secs(30),
            reconnect_stable_reset: Duration::from_secs(30),
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnvironmentConfigError {
    Missing(&'static str),
    InvalidPositiveInteger(&'static str),
    IncompleteCredentials,
    IncompleteLiveKitCredentials,
    InvalidSocketAddress(&'static str),
    HealthBindNotLoopback,
    InvalidDatabaseConfiguration,
    InvalidRetentionConfiguration,
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
            Self::InvalidRetentionConfiguration => {
                formatter.write_str("retention configuration is invalid")
            }
        }
    }
}

impl Error for EnvironmentConfigError {}

#[cfg(test)]
mod tests {
    use super::{
        load_database_with, load_health_bind_with, load_retention_with, load_supervision_with,
        load_with, EnvironmentConfigError,
    };
    use std::{collections::HashMap, time::Duration};

    #[test]
    fn requires_deployment_identity_source_and_dimensions() {
        let values = HashMap::<&str, &str>::new();
        assert_eq!(
            load_with(|key| values.get(key).map(|value| (*value).to_owned())).err(),
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
            load_with(|key| values.get(key).map(|value| (*value).to_owned())).err(),
            Some(EnvironmentConfigError::IncompleteCredentials)
        );
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

    #[test]
    fn retention_defaults_are_versioned_and_invalid_overrides_are_rejected() {
        let settings =
            load_retention_with(|_| None).unwrap_or_else(|error| panic!("valid defaults: {error}"));
        assert_eq!(
            settings.policy.retention(),
            Duration::from_secs(30 * 24 * 60 * 60)
        );
        assert_eq!(settings.policy.maximum_files(), 100);
        assert_eq!(
            load_retention_with(|key| {
                (key == "SURVEILLANCE_RETENTION_DAYS").then(|| "0".to_owned())
            })
            .err(),
            Some(EnvironmentConfigError::InvalidPositiveInteger(
                "SURVEILLANCE_RETENTION_DAYS"
            ))
        );
    }
}
