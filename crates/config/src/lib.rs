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
pub mod secret;
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
