//! Environment boundary for required deployment values and secrets.

use std::{
    error::Error,
    fmt,
    net::SocketAddr,
    num::{NonZeroU32, NonZeroUsize},
    path::PathBuf,
    time::Duration,
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
const CONFIG_FILE: &str = "SURVEILLANCE_CONFIG_FILE";

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
        }
    }
}

impl Error for EnvironmentConfigError {}

#[cfg(test)]
mod tests {
    use super::{
        defaults, load_database_with, load_health_bind_with, load_retention, load_supervision_with,
        load_with, EnvironmentConfigError, ResolvedSettings,
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
}
