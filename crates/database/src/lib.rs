//! Persistence adapters and migrations; business policy lives elsewhere.

pub mod access_audit;
pub mod auth;
pub mod cameras;
pub mod config;
pub mod events;
pub mod recordings;
pub mod rois;
pub mod settings;

pub use access_audit::{
    AccessAuditRecord, AuditCategory, AuditOutcome, ExportExtent, PostgresAccessAuditRepository,
};
pub use auth::{PostgresAuthRepository, PrincipalSummary, TokenSummary};
pub use config::{DatabaseConfig, DatabaseConfigError};
pub use recordings::{
    DatabasePoolSnapshot, IndexedCompletedEvidence, PersistCompletedOutcome,
    PostgresRecordingRepository, RepositoryError, RepositoryErrorCategory, RetentionCandidate,
};
pub use settings::{audit_records, PostgresSettingsRepository};
