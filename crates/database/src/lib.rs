//! Persistence adapters and migrations; business policy lives elsewhere.

pub mod cameras;
pub mod config;
pub mod events;
pub mod recordings;
pub mod rois;
pub mod settings;

pub use config::{DatabaseConfig, DatabaseConfigError};
pub use recordings::{
    DatabasePoolSnapshot, IndexedCompletedEvidence, PersistCompletedOutcome,
    PostgresRecordingRepository, RepositoryError, RepositoryErrorCategory, RetentionCandidate,
};
