#![allow(
    clippy::missing_errors_doc,
    reason = "typed validation errors are documented at the module boundary"
)]
use std::{error::Error, fmt, num::NonZeroUsize, path::PathBuf, time::Duration};

use url::Url;

use crate::domain::{IdentityError, PublicationIdentity};
use surveillance_common::types::CameraId;

#[derive(Clone)]
pub struct LiveKitCredentials {
    api_key: String,
    api_secret: String,
}

impl LiveKitCredentials {
    pub fn new(api_key: String, api_secret: String) -> Result<Self, LiveKitConfigError> {
        if api_key.trim().is_empty() || api_secret.len() < 8 {
            return Err(LiveKitConfigError::InvalidCredentials);
        }
        Ok(Self {
            api_key,
            api_secret,
        })
    }

    #[must_use]
    pub fn api_key(&self) -> &str {
        &self.api_key
    }
    #[must_use]
    pub fn api_secret(&self) -> &str {
        &self.api_secret
    }
}

impl fmt::Debug for LiveKitCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LiveKitCredentials([REDACTED])")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PublicationBackoff {
    pub initial: Duration,
    pub maximum: Duration,
    pub stable_reset: Duration,
}

impl PublicationBackoff {
    pub fn new(
        initial: Duration,
        maximum: Duration,
        stable_reset: Duration,
    ) -> Result<Self, LiveKitConfigError> {
        if initial.is_zero() || maximum < initial || stable_reset.is_zero() {
            return Err(LiveKitConfigError::InvalidBackoff);
        }
        Ok(Self {
            initial,
            maximum,
            stable_reset,
        })
    }

    #[must_use]
    pub fn delay(self, failures: u32) -> Duration {
        let multiplier = 1_u32.checked_shl(failures.min(20)).unwrap_or(u32::MAX);
        self.initial.saturating_mul(multiplier).min(self.maximum)
    }
}

#[derive(Clone)]
pub struct LiveKitConfig {
    pub endpoint: Url,
    pub credentials: LiveKitCredentials,
    pub identity: PublicationIdentity,
    pub deployment_namespace: String,
    pub cli_path: PathBuf,
    pub cli_version: String,
    pub queue_items: NonZeroUsize,
    pub max_lag: Duration,
    pub max_access_unit_bytes: NonZeroUsize,
    pub ipc_write_timeout: Duration,
    pub shutdown_timeout: Duration,
    pub backoff: PublicationBackoff,
}

impl fmt::Debug for LiveKitConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LiveKitConfig")
            .field("endpoint", &self.endpoint.origin().ascii_serialization())
            .field("credentials", &self.credentials)
            .field("identity", &self.identity)
            .field("deployment_namespace", &self.deployment_namespace)
            .field("cli_path", &self.cli_path)
            .field("cli_version", &self.cli_version)
            .field("queue_items", &self.queue_items)
            .field("max_lag", &self.max_lag)
            .finish_non_exhaustive()
    }
}

pub struct LiveKitSettings {
    pub endpoint: String,
    pub api_key: String,
    pub api_secret: String,
    pub deployment_namespace: String,
    pub cli_path: PathBuf,
    pub cli_version: String,
    pub queue_items: usize,
    pub max_lag: Duration,
    pub max_access_unit_bytes: usize,
    pub ipc_write_timeout: Duration,
    pub shutdown_timeout: Duration,
    pub reconnect_initial: Duration,
    pub reconnect_maximum: Duration,
    pub reconnect_stable_reset: Duration,
}

impl LiveKitConfig {
    pub fn validate(
        camera_id: &CameraId,
        settings: LiveKitSettings,
    ) -> Result<Self, LiveKitConfigError> {
        let endpoint =
            Url::parse(&settings.endpoint).map_err(|_| LiveKitConfigError::InvalidEndpoint)?;
        if !matches!(endpoint.scheme(), "ws" | "wss" | "http" | "https")
            || endpoint.username() != ""
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(LiveKitConfigError::InvalidEndpoint);
        }
        if settings.cli_path.as_os_str().is_empty() || settings.cli_version.trim().is_empty() {
            return Err(LiveKitConfigError::InvalidAdapter);
        }
        let queue_items =
            NonZeroUsize::new(settings.queue_items).ok_or(LiveKitConfigError::InvalidQueue)?;
        let max_access_unit_bytes = NonZeroUsize::new(settings.max_access_unit_bytes)
            .ok_or(LiveKitConfigError::InvalidMediaLimit)?;
        if settings.max_lag.is_zero()
            || settings.ipc_write_timeout.is_zero()
            || settings.shutdown_timeout.is_zero()
        {
            return Err(LiveKitConfigError::InvalidTiming);
        }
        Ok(Self {
            endpoint,
            credentials: LiveKitCredentials::new(settings.api_key, settings.api_secret)?,
            identity: PublicationIdentity::for_camera(&settings.deployment_namespace, camera_id)
                .map_err(LiveKitConfigError::Identity)?,
            deployment_namespace: settings.deployment_namespace,
            cli_path: settings.cli_path,
            cli_version: settings.cli_version,
            queue_items,
            max_lag: settings.max_lag,
            max_access_unit_bytes,
            ipc_write_timeout: settings.ipc_write_timeout,
            shutdown_timeout: settings.shutdown_timeout,
            backoff: PublicationBackoff::new(
                settings.reconnect_initial,
                settings.reconnect_maximum,
                settings.reconnect_stable_reset,
            )?,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LiveKitConfigError {
    InvalidEndpoint,
    InvalidCredentials,
    InvalidAdapter,
    InvalidQueue,
    InvalidMediaLimit,
    InvalidTiming,
    InvalidBackoff,
    Identity(IdentityError),
}

impl fmt::Display for LiveKitConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid LiveKit publication configuration")
    }
}

impl Error for LiveKitConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> LiveKitSettings {
        LiveKitSettings {
            endpoint: "ws://127.0.0.1:7880".to_owned(),
            api_key: "devkey".to_owned(),
            api_secret: "development-secret".to_owned(),
            deployment_namespace: "site-a".to_owned(),
            cli_path: PathBuf::from("lk"),
            cli_version: "2.18.2".to_owned(),
            queue_items: 4,
            max_lag: Duration::from_millis(500),
            max_access_unit_bytes: 2 * 1024 * 1024,
            ipc_write_timeout: Duration::from_millis(250),
            shutdown_timeout: Duration::from_secs(2),
            reconnect_initial: Duration::from_millis(250),
            reconnect_maximum: Duration::from_secs(30),
            reconnect_stable_reset: Duration::from_secs(30),
        }
    }

    #[test]
    fn validation_is_transactional_and_debug_is_secret_safe() {
        let camera = CameraId::new("camera-01").unwrap_or_else(|error| panic!("{error}"));
        let config =
            LiveKitConfig::validate(&camera, valid()).unwrap_or_else(|error| panic!("{error}"));
        let debug = format!("{config:?}");
        assert!(!debug.contains("development-secret"));
        let mut invalid = valid();
        invalid.endpoint = "ws://user:secret@127.0.0.1:7880?token=x".to_owned();
        assert_eq!(
            LiveKitConfig::validate(&camera, invalid).err(),
            Some(LiveKitConfigError::InvalidEndpoint)
        );
    }

    #[test]
    fn backoff_is_bounded_and_resettable_by_owner() {
        let policy = PublicationBackoff::new(
            Duration::from_millis(10),
            Duration::from_millis(40),
            Duration::from_secs(1),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(policy.delay(0), Duration::from_millis(10));
        assert_eq!(policy.delay(20), Duration::from_millis(40));
    }
}
