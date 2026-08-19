//! Validates `LiveKit` credentials, endpoint settings, and bounded publication backoff.

#![allow(
    clippy::missing_errors_doc,
    reason = "typed validation errors are documented at the module boundary"
)]
use std::{error::Error, fmt, num::NonZeroUsize, time::Duration};

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
    pub queue_items: NonZeroUsize,
    pub max_lag: Duration,
    pub max_access_unit_bytes: NonZeroUsize,
    pub shutdown_timeout: Duration,
    pub backoff: PublicationBackoff,
    /// Program that decodes H.264 for the live-viewing path.
    ///
    /// Live publication transcodes (ADR 012), so this is on the path to a
    /// working live view rather than an optional extra. Named rather than
    /// hardcoded because the binary's location is a deployment fact.
    pub decoder_program: String,
    /// Ceiling on published picture height; `0` means no ceiling.
    ///
    /// The single largest CPU lever on the live path — publishing 4K into a
    /// browser tile is waste. It defaults to no ceiling because choosing a
    /// viewing resolution is a product decision, not one this code may make
    /// silently.
    pub max_publish_height: u32,
    /// How long publication may stay warm after the final viewer leaves.
    pub viewer_idle_grace: Duration,
}

impl fmt::Debug for LiveKitConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LiveKitConfig")
            .field("endpoint", &self.endpoint.origin().ascii_serialization())
            .field("credentials", &self.credentials)
            .field("identity", &self.identity)
            .field("deployment_namespace", &self.deployment_namespace)
            .field("queue_items", &self.queue_items)
            .field("max_lag", &self.max_lag)
            .finish_non_exhaustive()
    }
}

/// Deliberately has no `Debug`: two of its fields are the API key and secret,
/// and unlike [`LiveKitConfig`] there is no redacting implementation to stand
/// between them and a log line.
///
/// `Clone` exists because these values are a property of the deployment, not of
/// a camera — one unit publishes to one server — so the process owner keeps a
/// template and stamps a copy onto every camera that publishes, including the
/// ones created through the API after startup.
#[derive(Clone)]
pub struct LiveKitSettings {
    pub endpoint: String,
    pub api_key: String,
    pub api_secret: String,
    pub deployment_namespace: String,
    pub queue_items: usize,
    pub max_lag: Duration,
    pub max_access_unit_bytes: usize,
    pub shutdown_timeout: Duration,
    pub reconnect_initial: Duration,
    pub reconnect_maximum: Duration,
    pub reconnect_stable_reset: Duration,
    /// Program that decodes H.264 for the live-viewing path.
    pub decoder_program: String,
    /// Ceiling on published picture height; `0` means no ceiling.
    pub max_publish_height: u32,
    /// Bounded warm-session interval used to absorb short navigation gaps.
    pub viewer_idle_grace: Duration,
}

impl LiveKitConfig {
    /// Prevents a mistyped duration from turning viewer gating into permanent
    /// background transcoding. Zero remains a valid immediate-teardown value.
    pub const MAX_VIEWER_IDLE_GRACE: Duration = Duration::from_secs(300);

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
        let queue_items =
            NonZeroUsize::new(settings.queue_items).ok_or(LiveKitConfigError::InvalidQueue)?;
        let max_access_unit_bytes = NonZeroUsize::new(settings.max_access_unit_bytes)
            .ok_or(LiveKitConfigError::InvalidMediaLimit)?;
        if settings.max_lag.is_zero()
            || settings.shutdown_timeout.is_zero()
            || settings.viewer_idle_grace > Self::MAX_VIEWER_IDLE_GRACE
        {
            return Err(LiveKitConfigError::InvalidTiming);
        }
        // An empty program name would fail at spawn time, one camera at a
        // time, long after the operator could connect it to what they typed.
        if settings.decoder_program.trim().is_empty() {
            return Err(LiveKitConfigError::InvalidDecoder);
        }
        Ok(Self {
            decoder_program: settings.decoder_program,
            max_publish_height: settings.max_publish_height,
            viewer_idle_grace: settings.viewer_idle_grace,
            endpoint,
            credentials: LiveKitCredentials::new(settings.api_key, settings.api_secret)?,
            identity: PublicationIdentity::for_camera(&settings.deployment_namespace, camera_id)
                .map_err(LiveKitConfigError::Identity)?,
            deployment_namespace: settings.deployment_namespace,
            queue_items,
            max_lag: settings.max_lag,
            max_access_unit_bytes,
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
    InvalidQueue,
    InvalidMediaLimit,
    InvalidTiming,
    InvalidBackoff,
    InvalidDecoder,
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
            decoder_program: "ffmpeg".to_owned(),
            max_publish_height: 0,
            viewer_idle_grace: Duration::from_secs(5),
            endpoint: "ws://127.0.0.1:7880".to_owned(),
            api_key: "devkey".to_owned(),
            api_secret: "development-secret".to_owned(),
            deployment_namespace: "site-a".to_owned(),
            queue_items: 4,
            max_lag: Duration::from_millis(500),
            max_access_unit_bytes: 2 * 1024 * 1024,
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

    #[test]
    fn viewer_idle_grace_is_optional_but_cannot_disable_viewer_gating() {
        let camera = CameraId::new("camera-01").unwrap_or_else(|error| panic!("{error}"));
        let mut immediate = valid();
        immediate.viewer_idle_grace = Duration::ZERO;
        assert!(LiveKitConfig::validate(&camera, immediate).is_ok());

        let mut effectively_permanent = valid();
        effectively_permanent.viewer_idle_grace = Duration::from_secs(301);
        assert_eq!(
            LiveKitConfig::validate(&camera, effectively_permanent).err(),
            Some(LiveKitConfigError::InvalidTiming)
        );
    }
}
