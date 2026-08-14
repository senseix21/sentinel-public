//! The settings store contract.
//!
//! The contract lives here and its `PostgreSQL` implementation lives in
//! `crates/database`, so the dependency points inward: persistence knows about
//! the configuration domain, and the configuration domain knows nothing about
//! the store.

use std::{error::Error, fmt};

use surveillance_common::types::CameraId;

use crate::{
    classification::{Origin, SettingId},
    resolution::{Layer, SettingKey},
    value::SettingValue,
};

/// Who made a change, and under what action.
///
/// Phase 7 records the loopback origin and a placeholder identity; ADR 008
/// defers the identity model to Phase 8. The shape does not change then.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Actor(String);

impl Actor {
    /// The Phase 7 placeholder: a request that reached the loopback listener.
    #[must_use]
    pub fn loopback() -> Self {
        Self("loopback".to_owned())
    }

    /// # Errors
    /// Returns [`ActorError::Empty`] for an empty or whitespace-only identity,
    /// and [`ActorError::TooLong`] beyond the column bound.
    pub fn new(identity: &str) -> Result<Self, ActorError> {
        let trimmed = identity.trim();
        if trimmed.is_empty() {
            return Err(ActorError::Empty);
        }
        if trimmed.len() > 128 {
            return Err(ActorError::TooLong);
        }
        Ok(Self(trimmed.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An identity that cannot be recorded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActorError {
    Empty,
    TooLong,
}

impl fmt::Display for ActorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("actor identity is empty"),
            Self::TooLong => formatter.write_str("actor identity exceeds 128 bytes"),
        }
    }
}

impl Error for ActorError {}

/// What an audit row records.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditAction {
    Set,
    Clear,
}

impl AuditAction {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Set => "set",
            Self::Clear => "clear",
        }
    }
}

/// One row of the audit trail.
///
/// Values are carried as the representation the schema expects: either a
/// plaintext value or a fingerprint, never both. See
/// [`SettingValue::audit_representation`].
pub struct AuditRecord {
    pub key: SettingKey,
    pub actor: Actor,
    pub action: AuditAction,
    pub origin: Origin,
    pub outcome: &'static str,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
    pub old_fingerprint: Option<String>,
    pub new_fingerprint: Option<String>,
}

impl fmt::Debug for AuditRecord {
    /// Renders keys and representations, never a plaintext secret — a secret
    /// side is always a fingerprint by construction.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuditRecord")
            .field("key", &self.key.to_string())
            .field("actor", &self.actor)
            .field("action", &self.action)
            .field("outcome", &self.outcome)
            .finish_non_exhaustive()
    }
}

/// A batch to write in one transaction.
pub struct SettingsBatch {
    pub entries: Vec<(SettingKey, SettingValue)>,
    pub audit: Vec<AuditRecord>,
}

/// What a load returned.
pub struct PersistedSettings {
    /// The persisted layer, ready to resolve.
    pub layer: Layer,
    /// The highest generation observed, or zero when the store is empty.
    pub generation: u64,
    /// Rows the store held that no longer name a setting, or whose value no
    /// longer parses. Surfaced rather than dropped: an operator needs to know
    /// their stored configuration is partly unreadable.
    pub rejected: Vec<RejectedRow>,
}

/// A stored row that could not be turned into a setting.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RejectedRow {
    pub key: String,
    pub camera: Option<CameraId>,
    pub reason: RejectionReason,
}

/// Why a stored row was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RejectionReason {
    /// The key no longer names a setting, typically after a rename.
    UnknownKey,
    /// The value no longer parses for the setting's declared kind.
    Unparseable,
    /// The row carries a setting that must never be persisted.
    NotPersistable(SettingId),
    /// The camera identifier stored alongside the row is not valid.
    InvalidCamera,
    /// A camera scope on a global setting, or the reverse.
    WrongScope(SettingId),
}

impl fmt::Display for RejectionReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownKey => formatter.write_str("key does not name a setting"),
            Self::Unparseable => formatter.write_str("value does not parse for this setting"),
            Self::NotPersistable(id) => write!(formatter, "setting {id} must not be persisted"),
            Self::InvalidCamera => formatter.write_str("invalid camera identifier"),
            Self::WrongScope(id) => write!(formatter, "setting {id} has the wrong scope"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Actor, ActorError, AuditAction};

    #[test]
    fn the_loopback_actor_is_recordable() {
        assert_eq!(Actor::loopback().as_str(), "loopback");
    }

    #[test]
    fn an_empty_actor_is_refused() {
        assert_eq!(Actor::new("   "), Err(ActorError::Empty));
    }

    #[test]
    fn an_oversized_actor_is_refused() {
        assert_eq!(
            Actor::new(&"a".repeat(129)),
            Err(ActorError::TooLong),
            "the bound matches the column constraint"
        );
    }

    #[test]
    fn actor_identities_are_trimmed() {
        assert_eq!(
            Actor::new("  operator  ").map(|actor| actor.as_str().to_owned()),
            Ok("operator".to_owned())
        );
    }

    #[test]
    fn audit_action_labels_match_the_database_enum() {
        assert_eq!(AuditAction::Set.as_str(), "set");
        assert_eq!(AuditAction::Clear.as_str(), "clear");
    }
}
