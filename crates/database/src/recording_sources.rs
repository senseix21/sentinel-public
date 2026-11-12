//! Durable archive roots and source-scoped recording summaries.
use crate::{PostgresRecordingRepository, RepositoryError};
use chrono::{DateTime, Utc};
use sqlx::Row;
use std::path::PathBuf;

#[derive(Clone)]
pub struct RecordingSource {
    pub source_id: String,
    pub root: PathBuf,
    pub label: String,
    pub index_state: String,
}

pub struct SourceCamera {
    pub camera_id: String,
    pub first_recorded_at: DateTime<Utc>,
    pub last_recorded_at: DateTime<Utc>,
    pub recordings: i64,
}

impl PostgresRecordingRepository {
    #[must_use]
    pub fn playback_api_prefix(&self) -> String {
        if self.source_id == "primary" {
            "/api".to_owned()
        } else {
            format!("/api/recording-sources/{}", self.source_id)
        }
    }

    /// Creates a namespaced handle sharing the existing bounded pool.
    /// # Errors
    /// Rejects invalid archive IDs; roots are resolved by the source owner.
    pub fn for_archive(&self, source_id: &str) -> Result<Self, RepositoryError> {
        surveillance_common::types::CameraId::new(source_id)
            .map_err(|_| RepositoryError::invariant())?;
        if source_id == "primary" {
            return Err(RepositoryError::invariant());
        }
        Ok(Self {
            pool: self.pool.clone(),
            source_id: source_id.to_owned(),
        })
    }

    /// Lists the bounded set of registered archive roots (never an API response).
    /// # Errors
    /// Returns a classified database failure or catalog capacity error.
    pub async fn recording_sources(&self) -> Result<Vec<RecordingSource>, RepositoryError> {
        let rows = sqlx::query("SELECT source_id, root, label, index_state FROM recording_sources WHERE read_only ORDER BY source_id LIMIT 33")
            .fetch_all(self.pool()).await.map_err(RepositoryError::from_sqlx)?;
        if rows.len() > 32 {
            return Err(RepositoryError::invariant());
        }
        rows.iter()
            .map(|row| {
                Ok(RecordingSource {
                    source_id: row
                        .try_get("source_id")
                        .map_err(RepositoryError::from_sqlx)?,
                    root: PathBuf::from(
                        row.try_get::<String, _>("root")
                            .map_err(RepositoryError::from_sqlx)?,
                    ),
                    label: row.try_get("label").map_err(RepositoryError::from_sqlx)?,
                    index_state: row
                        .try_get("index_state")
                        .map_err(RepositoryError::from_sqlx)?,
                })
            })
            .collect()
    }

    /// Registers an approved canonical root, idempotently and under a catalog lock.
    /// # Errors
    /// Returns a classified database or capacity error.
    pub async fn register_recording_source(
        &self,
        root: &std::path::Path,
        label: &str,
    ) -> Result<String, RepositoryError> {
        let root = root.to_str().ok_or_else(RepositoryError::invariant)?;
        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(RepositoryError::from_sqlx)?;
        sqlx::query("LOCK TABLE recording_sources IN SHARE ROW EXCLUSIVE MODE")
            .execute(&mut *tx)
            .await
            .map_err(RepositoryError::from_sqlx)?;
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM recording_sources WHERE read_only AND root <> $1",
        )
        .bind(root)
        .fetch_one(&mut *tx)
        .await
        .map_err(RepositoryError::from_sqlx)?;
        if count >= 32 {
            return Err(RepositoryError::invariant());
        }
        let id = sqlx::query_scalar("INSERT INTO recording_sources(root, label) VALUES ($1, $2) ON CONFLICT(root) DO UPDATE SET label = EXCLUDED.label RETURNING source_id")
            .bind(root).bind(label).fetch_one(&mut *tx).await.map_err(RepositoryError::from_sqlx)?;
        tx.commit().await.map_err(RepositoryError::from_sqlx)?;
        Ok(id)
    }

    /// Counts archive evidence that could not be made playable.
    /// # Errors
    /// Returns a classified database failure.
    pub async fn source_unplayable_count(&self) -> Result<i64, RepositoryError> {
        sqlx::query_scalar("SELECT count(*) FROM recording_segments WHERE source_id = $1 AND status <> 'completed'")
            .bind(&self.source_id).fetch_one(self.pool()).await.map_err(RepositoryError::from_sqlx)
    }

    /// Records bounded indexing status, without changing source media.
    /// # Errors
    /// Returns a classified database failure.
    pub async fn source_index_state(&self, id: &str, state: &str) -> Result<(), RepositoryError> {
        sqlx::query(
            "UPDATE recording_sources SET index_state = $2 WHERE source_id = $1 AND read_only",
        )
        .bind(id)
        .bind(state)
        .execute(self.pool())
        .await
        .map_err(RepositoryError::from_sqlx)?;
        Ok(())
    }

    /// Marks indexing interrupted by process exit as retryable operator work.
    /// # Errors
    /// Returns a classified database failure.
    pub async fn interrupt_archive_indexes(&self) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE recording_sources SET index_state = 'interrupted' WHERE index_state = 'indexing' AND read_only")
            .execute(self.pool()).await.map_err(RepositoryError::from_sqlx)?;
        Ok(())
    }

    /// Reads indexed camera coverage for this source, with scope applied in SQL.
    /// # Errors
    /// Returns a classified database or capacity error.
    pub async fn source_cameras(
        &self,
        scope: Option<Vec<String>>,
    ) -> Result<Vec<SourceCamera>, RepositoryError> {
        let rows = sqlx::query("SELECT camera_id, min(started_at) AS first_recorded_at, max(ended_at) AS last_recorded_at, count(*) AS recordings FROM recording_segments WHERE source_id = $1 AND status = 'completed' AND ($2::text[] IS NULL OR camera_id = ANY($2)) GROUP BY camera_id ORDER BY camera_id LIMIT 1025")
            .bind(&self.source_id).bind(scope).fetch_all(self.pool()).await.map_err(RepositoryError::from_sqlx)?;
        if rows.len() > 1024 {
            return Err(RepositoryError::invariant());
        }
        rows.iter()
            .map(|row| {
                Ok(SourceCamera {
                    camera_id: row
                        .try_get("camera_id")
                        .map_err(RepositoryError::from_sqlx)?,
                    first_recorded_at: row
                        .try_get("first_recorded_at")
                        .map_err(RepositoryError::from_sqlx)?,
                    last_recorded_at: row
                        .try_get("last_recorded_at")
                        .map_err(RepositoryError::from_sqlx)?,
                    recordings: row
                        .try_get("recordings")
                        .map_err(RepositoryError::from_sqlx)?,
                })
            })
            .collect()
    }
}
