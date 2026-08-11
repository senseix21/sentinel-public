//! Environment boundary for required deployment values and secrets.

use std::{error::Error, fmt, net::SocketAddr, num::NonZeroUsize, path::PathBuf, time::Duration};

use surveillance_config::settings::{CameraCredentialsSettings, OneCameraSettings};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SupervisionSettings {
    pub maximum_cameras: NonZeroUsize,
    pub command_queue_items: NonZeroUsize,
    pub command_timeout: Duration,
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
    Ok(OneCameraSettings::h264_mvp(
        camera_id,
        endpoint,
        credentials,
        width,
        height,
        recording_root,
    ))
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
    InvalidSocketAddress(&'static str),
    HealthBindNotLoopback,
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
            Self::InvalidSocketAddress(key) => {
                write!(
                    formatter,
                    "environment variable {key} must be a socket address"
                )
            }
            Self::HealthBindNotLoopback => {
                formatter.write_str("Phase 2 health endpoint must bind to a loopback address")
            }
        }
    }
}

impl Error for EnvironmentConfigError {}

#[cfg(test)]
mod tests {
    use super::{load_health_bind_with, load_supervision_with, load_with, EnvironmentConfigError};
    use std::collections::HashMap;

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
}
