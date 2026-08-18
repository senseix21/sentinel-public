//! Validated, persisted operational configuration.

pub mod assemble;
pub mod cache;
pub mod change;
pub mod classification;
pub mod defaults;
pub mod environment;
pub mod file;
pub mod repository;
pub mod resolution;
/// Re-exported from `surveillance-common`.
///
/// Secret carriage became cross-domain in Phase 8: identity (`surveillance-auth`)
/// and the secret store need it as much as configuration does, and pointing the
/// identity domain at the whole configuration domain to reach it would invert
/// the dependency direction. The module is re-exported here so existing
/// `surveillance_config::secret::*` paths keep resolving.
pub use surveillance_common::secret;
pub mod secret_store;
pub mod settings;
pub mod validation;
pub mod value;

pub use classification::{
    ApplicationBoundary, Origin, Scope, Secrecy, SettingClass, SettingId, ValidationGroup,
    ValueKind,
};
pub use resolution::{Layer, LayerError, Resolved, ResolvedSettings, ScopeError, SettingKey};
pub use secret::{Fingerprint, Secret, REDACTED};
pub use value::{AuditRepresentation, SettingValue, ValueParseError};
