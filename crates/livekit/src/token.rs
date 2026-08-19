//! Issues validated, camera-scoped `LiveKit` publisher and viewer access tokens.

#![allow(
    clippy::missing_errors_doc,
    reason = "token issuance exposes one secret-safe typed error contract"
)]
use std::{error::Error, fmt, time::Duration};

use livekit_api::access_token::{AccessToken, VideoGrants};

use crate::{
    config::LiveKitConfig,
    domain::{PublicationIdentity, RoomName},
};
use surveillance_common::types::CameraId;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ViewerIdentity(String);

impl ViewerIdentity {
    pub fn new(value: impl Into<String>) -> Result<Self, TokenIssueError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 128
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(TokenIssueError::InvalidViewerIdentity);
        }
        Ok(Self(value))
    }
}

#[derive(Clone)]
pub struct ViewerTokenIssuer {
    endpoint: String,
    api_key: String,
    api_secret: String,
    ttl: Duration,
}

#[derive(Clone, Debug)]
pub struct DevelopmentTokenService {
    issuer: ViewerTokenIssuer,
    namespace: String,
}

impl DevelopmentTokenService {
    pub fn new(
        config: &LiveKitConfig,
        namespace: String,
        ttl: Duration,
    ) -> Result<Self, TokenIssueError> {
        PublicationIdentity::for_camera(
            &namespace,
            &CameraId::new("validation-camera").map_err(|_| TokenIssueError::Unavailable)?,
        )
        .map_err(|_| TokenIssueError::Unavailable)?;
        Ok(Self {
            issuer: ViewerTokenIssuer::new(config, ttl)?,
            namespace,
        })
    }

    pub fn issue_for_camera(&self, camera: &CameraId) -> Result<ViewerAccess, TokenIssueError> {
        let identity = PublicationIdentity::for_camera(&self.namespace, camera)
            .map_err(|_| TokenIssueError::Unavailable)?;
        let viewer = ViewerIdentity::new(format!("development-viewer-{}", camera.as_str()))?;
        self.issuer.issue(&viewer, &identity.room)
    }
}

impl fmt::Debug for ViewerTokenIssuer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ViewerTokenIssuer")
            .field("endpoint", &self.endpoint)
            .field("ttl", &self.ttl)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub struct ViewerAccess {
    endpoint: String,
    room: String,
    token: String,
    expires_in_seconds: u64,
}

impl ViewerAccess {
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
    #[must_use]
    pub fn room(&self) -> &str {
        &self.room
    }
    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }
    #[must_use]
    pub const fn expires_in_seconds(&self) -> u64 {
        self.expires_in_seconds
    }
}

impl ViewerTokenIssuer {
    pub fn new(config: &LiveKitConfig, ttl: Duration) -> Result<Self, TokenIssueError> {
        if ttl < Duration::from_secs(30) || ttl > Duration::from_secs(15 * 60) {
            return Err(TokenIssueError::InvalidLifetime);
        }
        Ok(Self {
            endpoint: config.endpoint.as_str().to_owned(),
            api_key: config.credentials.api_key().to_owned(),
            api_secret: config.credentials.api_secret().to_owned(),
            ttl,
        })
    }

    pub fn issue(
        &self,
        viewer: &ViewerIdentity,
        room: &RoomName,
    ) -> Result<ViewerAccess, TokenIssueError> {
        let grants = VideoGrants {
            room_join: true,
            room: room.as_str().to_owned(),
            can_publish: false,
            can_subscribe: true,
            can_publish_data: false,
            ..VideoGrants::default()
        };
        let token = AccessToken::with_api_key(&self.api_key, &self.api_secret)
            .with_identity(&viewer.0)
            .with_grants(grants)
            .with_ttl(self.ttl)
            .to_jwt()
            .map_err(|_| TokenIssueError::Unavailable)?;
        Ok(ViewerAccess {
            endpoint: self.endpoint.clone(),
            room: room.as_str().to_owned(),
            token,
            expires_in_seconds: self.ttl.as_secs(),
        })
    }
}

/// A token for the publisher itself, as distinct from a viewer.
///
/// The publisher produces media and consumes none, so this grants publication
/// into exactly one room and nothing else. A token that could also subscribe
/// would carry authority the publisher never exercises.
#[derive(Clone)]
pub struct PublisherAccess {
    endpoint: String,
    token: String,
}

impl PublisherAccess {
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }
}

impl fmt::Debug for PublisherAccess {
    /// Deliberately omits the token: it is a bearer credential, and a `Debug`
    /// impl is exactly the accident that puts one in a log.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PublisherAccess")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

/// Mints the publisher's own room credential.
#[derive(Clone)]
pub struct PublisherTokenIssuer {
    endpoint: String,
    api_key: String,
    api_secret: String,
    ttl: Duration,
}

impl fmt::Debug for PublisherTokenIssuer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PublisherTokenIssuer")
            .field("endpoint", &self.endpoint)
            .field("ttl", &self.ttl)
            .finish_non_exhaustive()
    }
}

impl PublisherTokenIssuer {
    pub fn new(config: &LiveKitConfig, ttl: Duration) -> Result<Self, TokenIssueError> {
        if ttl < Duration::from_secs(30) || ttl > Duration::from_secs(15 * 60) {
            return Err(TokenIssueError::InvalidLifetime);
        }
        Ok(Self {
            endpoint: config.endpoint.as_str().to_owned(),
            api_key: config.credentials.api_key().to_owned(),
            api_secret: config.credentials.api_secret().to_owned(),
            ttl,
        })
    }

    /// Issues a fresh token for one connection attempt.
    ///
    /// The lifetime only has to cover joining, and a reconnect mints another,
    /// so a short-lived credential costs nothing here.
    pub fn issue(
        &self,
        identity: &PublicationIdentity,
    ) -> Result<PublisherAccess, TokenIssueError> {
        let grants = VideoGrants {
            room_join: true,
            room: identity.room.as_str().to_owned(),
            can_publish: true,
            can_subscribe: false,
            can_publish_data: false,
            ..VideoGrants::default()
        };
        let token = AccessToken::with_api_key(&self.api_key, &self.api_secret)
            .with_identity(identity.participant.as_str())
            .with_grants(grants)
            .with_ttl(self.ttl)
            .to_jwt()
            .map_err(|_| TokenIssueError::Unavailable)?;
        Ok(PublisherAccess {
            endpoint: self.endpoint.clone(),
            token,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TokenIssueError {
    InvalidViewerIdentity,
    InvalidLifetime,
    Unavailable,
}
impl fmt::Display for TokenIssueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LiveKit viewer access could not be issued")
    }
}
impl Error for TokenIssueError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{LiveKitCredentials, PublicationBackoff},
        domain::PublicationIdentity,
    };
    use livekit_api::access_token::Claims;
    use std::num::NonZeroUsize;
    use surveillance_common::types::CameraId;
    use url::Url;

    fn config() -> LiveKitConfig {
        let camera = CameraId::new("camera-01").unwrap_or_else(|error| panic!("{error}"));
        LiveKitConfig {
            decoder_program: "ffmpeg".to_owned(),
            max_publish_height: 0,
            viewer_idle_grace: Duration::from_secs(5),
            endpoint: Url::parse("ws://127.0.0.1:7880").unwrap_or_else(|error| panic!("{error}")),
            credentials: LiveKitCredentials::new(
                "devkey".to_owned(),
                "development-secret".to_owned(),
            )
            .unwrap_or_else(|error| panic!("{error}")),
            identity: PublicationIdentity::for_camera("test", &camera)
                .unwrap_or_else(|error| panic!("{error}")),
            deployment_namespace: "test".to_owned(),
            queue_items: NonZeroUsize::new(4).unwrap_or(NonZeroUsize::MIN),
            max_lag: Duration::from_secs(1),
            max_access_unit_bytes: NonZeroUsize::new(1024).unwrap_or(NonZeroUsize::MIN),
            shutdown_timeout: Duration::from_secs(1),
            backoff: PublicationBackoff::new(
                Duration::from_millis(1),
                Duration::from_secs(1),
                Duration::from_secs(2),
            )
            .unwrap_or_else(|error| panic!("{error}")),
        }
    }

    #[test]
    fn grants_are_short_lived_room_scoped_and_subscribe_only() {
        let config = config();
        let issuer = ViewerTokenIssuer::new(&config, Duration::from_secs(60))
            .unwrap_or_else(|error| panic!("{error}"));
        let viewer = ViewerIdentity::new("viewer-01").unwrap_or_else(|error| panic!("{error}"));
        let access = issuer
            .issue(&viewer, &config.identity.room)
            .unwrap_or_else(|error| panic!("{error}"));
        let claims =
            Claims::from_unverified(access.token()).unwrap_or_else(|error| panic!("{error}"));
        assert!(claims.video.room_join && claims.video.can_subscribe);
        assert!(
            !claims.video.can_publish && !claims.video.can_publish_data && !claims.video.room_admin
        );
        assert_eq!(claims.video.room, config.identity.room.as_str());
        assert!(access.expires_in_seconds() <= 60);
        assert!(!format!("{issuer:?}").contains("development-secret"));
    }
}
