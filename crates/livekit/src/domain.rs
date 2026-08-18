#![allow(
    clippy::missing_errors_doc,
    reason = "identity constructors share one typed validation contract"
)]
use std::{error::Error, fmt};

use surveillance_common::types::CameraId;

const MAX_IDENTITY_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublicationState {
    Disabled,
    Starting,
    Connecting,
    Publishing,
    Backoff,
    Degraded,
    Terminal,
    Stopping,
    Stopped,
}

impl PublicationState {
    #[must_use]
    #[allow(
        clippy::unnested_or_patterns,
        reason = "expanded lifecycle edges remain auditable"
    )]
    pub const fn can_transition_to(self, next: Self) -> bool {
        use PublicationState as S;
        matches!(
            (self, next),
            (S::Disabled, S::Stopped)
                | (S::Starting, S::Connecting | S::Stopping | S::Terminal)
                | (
                    S::Connecting,
                    S::Publishing | S::Backoff | S::Terminal | S::Stopping
                )
                // Publishing reaches Terminal when the adapter refuses in a way
                // reconnecting cannot change. That edge did not exist while the
                // only non-retryable failures were raised at connect time; a
                // publisher that has already sent access units can still meet
                // one on the send path.
                | (
                    S::Publishing,
                    S::Degraded | S::Backoff | S::Terminal | S::Stopping
                )
                // Degraded returns to Publishing when the session resynchronises
                // within itself, which a media-timeline discontinuity does: the
                // next parameterised keyframe establishes a fresh pacing origin
                // and publication continues on the same connection.
                //
                // Without this edge Degraded is a one-way door, and the state
                // would stay Degraded for the life of the connection after the
                // first camera reconnect — a signal an operator learns to ignore
                // is no better than the `Publishing` this replaced.
                | (
                    S::Degraded,
                    S::Publishing | S::Backoff | S::Connecting | S::Terminal | S::Stopping
                )
                | (S::Backoff, S::Connecting | S::Terminal | S::Stopping)
                | (S::Terminal, S::Stopping | S::Stopped)
                | (S::Stopping, S::Stopped)
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublicationFailureCategory {
    AdapterUnavailable,
    Authentication,
    Configuration,
    UnsupportedCodec,
    UnsupportedProfile,
    Connection,
    Transport,
    InputClosed,
    MissingParameterSets,
    WaitingForKeyframe,
    TimestampDiscontinuity,
    MalformedMedia,
    OversizedMedia,
    Shutdown,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RoomName(String);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ParticipantIdentity(String);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct TrackName(String);

macro_rules! identity_impl {
    ($name:ident, $label:literal) => {
        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, IdentityError> {
                let value = value.into();
                validate_identity(&value)?;
                Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

identity_impl!(RoomName, "room");
identity_impl!(ParticipantIdentity, "participant");
identity_impl!(TrackName, "track");

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublicationIdentity {
    pub room: RoomName,
    pub participant: ParticipantIdentity,
    pub track: TrackName,
}

impl PublicationIdentity {
    pub fn for_camera(namespace: &str, camera_id: &CameraId) -> Result<Self, IdentityError> {
        validate_namespace(namespace)?;
        let stem = format!("{namespace}-{}", camera_id.as_str());
        Ok(Self {
            room: RoomName::new(format!("{stem}-live"))?,
            participant: ParticipantIdentity::new(format!("{stem}-publisher"))?,
            track: TrackName::new(format!("{stem}-video"))?,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityError {
    Empty,
    TooLong,
    InvalidCharacter,
    InvalidPlacement,
}

impl fmt::Display for IdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "LiveKit identity must not be empty",
            Self::TooLong => "LiveKit identity exceeds 128 bytes",
            Self::InvalidCharacter => "LiveKit identity contains an unsupported character",
            Self::InvalidPlacement => "LiveKit identity has invalid separator placement",
        })
    }
}

impl Error for IdentityError {}

fn validate_namespace(value: &str) -> Result<(), IdentityError> {
    validate_identity(value)
}

fn validate_identity(value: &str) -> Result<(), IdentityError> {
    if value.is_empty() {
        return Err(IdentityError::Empty);
    }
    if value.len() > MAX_IDENTITY_BYTES {
        return Err(IdentityError::TooLong);
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(IdentityError::InvalidCharacter);
    }
    if value.starts_with('-') || value.ends_with('-') || value.contains("--") {
        return Err(IdentityError::InvalidPlacement);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_identity_is_stable_scoped_and_validated() {
        let camera = CameraId::new("camera-01").unwrap_or_else(|error| panic!("{error}"));
        let identity = PublicationIdentity::for_camera("site-a", &camera)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(identity.room.as_str(), "site-a-camera-01-live");
        assert_eq!(identity.participant.as_str(), "site-a-camera-01-publisher");
        assert_eq!(identity.track.as_str(), "site-a-camera-01-video");
        assert_eq!(
            PublicationIdentity::for_camera("Site A", &camera),
            Err(IdentityError::InvalidCharacter)
        );
    }

    #[test]
    fn lifecycle_rejects_false_publishing_state() {
        assert!(PublicationState::Connecting.can_transition_to(PublicationState::Publishing));
        assert!(!PublicationState::Backoff.can_transition_to(PublicationState::Publishing));
        assert!(PublicationState::Publishing.can_transition_to(PublicationState::Degraded));
    }
}
