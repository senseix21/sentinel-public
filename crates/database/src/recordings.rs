//! `PostgreSQL` recording metadata adapter.

use std::{error::Error, fmt, path::PathBuf, time::SystemTime};

use chrono::{DateTime, Utc};
use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use surveillance_common::types::RecordingId;
use surveillance_storage::metadata::{
    CompletedRecording, MetadataProvenance, NonPlayableRecording, RecordingContainer,
    RecordingFailureCategory, RecordingStatus, VideoCodec,
};
use surveillance_storage::recordings::RecordingSearch;

use crate::DatabaseConfig;

/// Forward migrations, owned by this crate and shared by every repository in
/// it so that one connection applies one ordered sequence.
pub(crate) static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

#[derive(Clone)]
pub struct PostgresRecordingRepository {
    pool: PgPool,
}

impl PostgresRecordingRepository {
    /// Creates a bounded pool without requiring `PostgreSQL` to be available.
    #[must_use]
    pub fn connect_lazy(config: &DatabaseConfig) -> Self {
        let pool = PgPoolOptions::new()
            .max_connections(config.maximum_connections())
            .acquire_timeout(config.acquire_timeout())
            .connect_lazy_with(config.connect_options());
        Self { pool }
    }

    /// Connects using an explicitly bounded pool.
    ///
    /// # Errors
    /// Returns a typed credential-free repository failure.
    pub async fn connect(config: &DatabaseConfig) -> Result<Self, RepositoryError> {
        let pool = PgPoolOptions::new()
            .max_connections(config.maximum_connections())
            .acquire_timeout(config.acquire_timeout())
            .connect_with(config.connect_options())
            .await
            .map_err(RepositoryError::from_sqlx)?;
        Ok(Self { pool })
    }

    #[must_use]
    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    #[must_use]
    pub fn pool_snapshot(&self) -> DatabasePoolSnapshot {
        DatabasePoolSnapshot {
            size: self.pool.size(),
            idle: u32::try_from(self.pool.num_idle()).unwrap_or(u32::MAX),
            maximum: self.pool.options().get_max_connections(),
        }
    }

    /// Probes dependency readiness through the bounded pool.
    ///
    /// # Errors
    /// Returns a classified database failure.
    pub async fn health(&self) -> Result<(), RepositoryError> {
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(RepositoryError::from_sqlx)
    }

    /// Applies repository-owned forward migrations.
    ///
    /// # Errors
    /// Returns [`RepositoryErrorCategory::Migration`] without exposing the DSN.
    pub async fn migrate(&self) -> Result<(), RepositoryError> {
        MIGRATOR
            .run(&self.pool)
            .await
            .map_err(RepositoryError::migration)
    }

    /// Atomically creates the camera identity and inserts or confirms immutable
    /// completed evidence.
    ///
    /// # Errors
    /// Returns a typed conflict when the recording identity already describes
    /// different evidence, or a classified `PostgreSQL` failure.
    pub async fn insert_or_confirm_completed(
        &self,
        completed: &CompletedRecording,
    ) -> Result<PersistCompletedOutcome, RepositoryError> {
        let started_at = system_timestamp(completed.started_at());
        let ended_at = system_timestamp(completed.ended_at());
        let width = i32::try_from(completed.width()).map_err(|_| RepositoryError::invariant())?;
        let height = i32::try_from(completed.height()).map_err(|_| RepositoryError::invariant())?;
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(RepositoryError::from_sqlx)?;

        sqlx::query("INSERT INTO cameras(camera_id) VALUES ($1) ON CONFLICT DO NOTHING")
            .bind(completed.camera_id().as_str())
            .execute(&mut *transaction)
            .await
            .map_err(RepositoryError::from_sqlx)?;

        let inserted = sqlx::query(
            "INSERT INTO recording_segments (recording_id, camera_id, started_at, ended_at, storage_locator, container, video_codec, width, height, size_bytes, status, provenance) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'completed', $11::recording_provenance) ON CONFLICT (recording_id) DO NOTHING",
        )
        .bind(completed.recording_id().as_str())
        .bind(completed.camera_id().as_str())
        .bind(started_at)
        .bind(ended_at)
        .bind(completed.locator().as_str())
        .bind(container_text(completed.container()))
        .bind(codec_text(completed.codec()))
        .bind(width)
        .bind(height)
        .bind(i64::try_from(completed.size_bytes()).map_err(|_| RepositoryError::invariant())?)
        .bind(provenance_text(completed.provenance()))
        .execute(&mut *transaction)
        .await
        .map_err(RepositoryError::from_sqlx)?
        .rows_affected();

        let outcome = if inserted == 1 {
            PersistCompletedOutcome::Inserted
        } else {
            let row = sqlx::query(
                "SELECT camera_id, started_at, ended_at, storage_locator, container, video_codec, width, height, size_bytes, status::text AS status, provenance::text AS provenance FROM recording_segments WHERE recording_id = $1",
            )
            .bind(completed.recording_id().as_str())
            .fetch_one(&mut *transaction)
            .await
            .map_err(RepositoryError::from_sqlx)?;
            if row_matches(&row, completed, started_at, ended_at)? {
                PersistCompletedOutcome::Confirmed
            } else {
                return Err(RepositoryError::conflict());
            }
        };

        transaction
            .commit()
            .await
            .map_err(RepositoryError::from_sqlx)?;
        Ok(outcome)
    }

    /// Atomically inserts or confirms failed/incomplete filesystem evidence.
    ///
    /// # Errors
    /// Returns a typed conflict for mismatched replay or a classified database
    /// failure.
    pub async fn insert_or_confirm_non_playable(
        &self,
        evidence: &NonPlayableRecording,
    ) -> Result<PersistCompletedOutcome, RepositoryError> {
        let started_at = system_timestamp(evidence.started_at());
        let ended_at = evidence.ended_at().map(system_timestamp);
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(RepositoryError::from_sqlx)?;
        sqlx::query("INSERT INTO cameras(camera_id) VALUES ($1) ON CONFLICT DO NOTHING")
            .bind(evidence.camera_id().as_str())
            .execute(&mut *transaction)
            .await
            .map_err(RepositoryError::from_sqlx)?;
        let inserted = sqlx::query(
            "INSERT INTO recording_segments (recording_id, camera_id, started_at, ended_at, storage_locator, status, provenance, failure_category) VALUES ($1, $2, $3, $4, $5, $6::recording_status, $7::recording_provenance, $8) ON CONFLICT (recording_id) DO NOTHING",
        )
        .bind(evidence.recording_id().as_str())
        .bind(evidence.camera_id().as_str())
        .bind(started_at)
        .bind(ended_at)
        .bind(evidence.locator().as_str())
        .bind(status_text(evidence.status()))
        .bind(provenance_text(evidence.provenance()))
        .bind(failure_text(evidence.failure()))
        .execute(&mut *transaction)
        .await
        .map_err(RepositoryError::from_sqlx)?
        .rows_affected();
        let outcome = if inserted == 1 {
            PersistCompletedOutcome::Inserted
        } else {
            let row = sqlx::query(
                "SELECT camera_id, started_at, ended_at, storage_locator, status::text AS status, provenance::text AS provenance, failure_category FROM recording_segments WHERE recording_id = $1",
            )
            .bind(evidence.recording_id().as_str())
            .fetch_one(&mut *transaction)
            .await
            .map_err(RepositoryError::from_sqlx)?;
            let matches = row
                .try_get::<String, _>("camera_id")
                .map_err(RepositoryError::from_sqlx)?
                == evidence.camera_id().as_str()
                && row
                    .try_get::<DateTime<Utc>, _>("started_at")
                    .map_err(RepositoryError::from_sqlx)?
                    == started_at
                && row
                    .try_get::<Option<DateTime<Utc>>, _>("ended_at")
                    .map_err(RepositoryError::from_sqlx)?
                    == ended_at
                && row
                    .try_get::<String, _>("storage_locator")
                    .map_err(RepositoryError::from_sqlx)?
                    == evidence.locator().as_str()
                && row
                    .try_get::<String, _>("status")
                    .map_err(RepositoryError::from_sqlx)?
                    == status_text(evidence.status())
                && row
                    .try_get::<String, _>("provenance")
                    .map_err(RepositoryError::from_sqlx)?
                    == provenance_text(evidence.provenance())
                && row
                    .try_get::<Option<String>, _>("failure_category")
                    .map_err(RepositoryError::from_sqlx)?
                    .as_deref()
                    == Some(failure_text(evidence.failure()));
            if matches {
                PersistCompletedOutcome::Confirmed
            } else {
                return Err(RepositoryError::conflict());
            }
        };
        transaction
            .commit()
            .await
            .map_err(RepositoryError::from_sqlx)?;
        Ok(outcome)
    }

    /// Selects one deterministic bounded page using the normative indexed
    /// half-open overlap predicate.
    ///
    /// # Errors
    /// Returns typed repository or row-invariant failures.
    pub async fn find_overlapping_completed(
        &self,
        search: &RecordingSearch,
    ) -> Result<Vec<CompletedRecording>, RepositoryError> {
        let range_start = system_timestamp(search.range().start());
        let range_end = system_timestamp(search.range().end());
        let cursor_time = search
            .cursor()
            .map(|cursor| system_timestamp(cursor.started_at()));
        let cursor_id = search.cursor().map(|cursor| cursor.recording_id().as_str());
        let limit =
            i64::try_from(search.limit().get()).map_err(|_| RepositoryError::invariant())?;
        let rows = sqlx::query(
            "SELECT recording_id, camera_id, started_at, ended_at, storage_locator, container, video_codec, width, height, size_bytes, provenance::text AS provenance FROM recording_segments WHERE camera_id = $1 AND status = 'completed' AND started_at < $2 AND ended_at > $3 AND ($4::timestamptz IS NULL OR (started_at, recording_id) > ($4, $5::text)) ORDER BY started_at, recording_id LIMIT $6",
        )
        .bind(search.camera_id().as_str())
        .bind(range_end)
        .bind(range_start)
        .bind(cursor_time)
        .bind(cursor_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(RepositoryError::from_sqlx)?;
        rows.iter().map(completed_from_row).collect()
    }

    /// Finds one camera-scoped completed recording without exposing its locator
    /// outside the application boundary.
    ///
    /// # Errors
    /// Returns a classified database or row-invariant failure.
    pub async fn find_completed(
        &self,
        camera_id: &surveillance_common::types::CameraId,
        recording_id: &RecordingId,
    ) -> Result<Option<CompletedRecording>, RepositoryError> {
        let row = sqlx::query(
            "SELECT recording_id, camera_id, started_at, ended_at, storage_locator, container, video_codec, width, height, size_bytes, provenance::text AS provenance FROM recording_segments WHERE camera_id = $1 AND recording_id = $2 AND status = 'completed'",
        )
        .bind(camera_id.as_str())
        .bind(recording_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(RepositoryError::from_sqlx)?;
        row.as_ref().map(completed_from_row).transpose()
    }

    /// Reads lifecycle state for idempotent reconciliation.
    ///
    /// # Errors
    /// Returns a classified database or invariant failure.
    pub async fn recording_status(
        &self,
        recording_id: &RecordingId,
    ) -> Result<Option<RecordingStatus>, RepositoryError> {
        let value = sqlx::query_scalar::<_, String>(
            "SELECT status::text FROM recording_segments WHERE recording_id = $1",
        )
        .bind(recording_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(RepositoryError::from_sqlx)?;
        value.map(|value| parse_status(&value)).transpose()
    }

    /// Returns a bounded deterministic page of completed file evidence.
    ///
    /// # Errors
    /// Returns a classified database failure or rejects an invalid page bound.
    pub async fn completed_evidence_page(
        &self,
        after: Option<&RecordingId>,
        limit: usize,
    ) -> Result<Vec<IndexedCompletedEvidence>, RepositoryError> {
        if limit == 0 || limit > 1_000 {
            return Err(RepositoryError::invariant());
        }
        let rows = sqlx::query(
            "SELECT recording_id, storage_locator FROM recording_segments WHERE status = 'completed' AND ($1::text IS NULL OR recording_id > $1) ORDER BY recording_id LIMIT $2",
        )
        .bind(after.map(RecordingId::as_str))
        .bind(i64::try_from(limit).map_err(|_| RepositoryError::invariant())?)
        .fetch_all(&self.pool)
        .await
        .map_err(RepositoryError::from_sqlx)?;
        rows.iter()
            .map(|row| {
                let recording_id = RecordingId::new(
                    row.try_get::<String, _>("recording_id")
                        .map_err(RepositoryError::from_sqlx)?,
                )
                .map_err(|_| RepositoryError::invariant())?;
                let locator = surveillance_storage::metadata::RecordingLocator::new(PathBuf::from(
                    row.try_get::<String, _>("storage_locator")
                        .map_err(RepositoryError::from_sqlx)?,
                ))
                .map_err(|_| RepositoryError::invariant())?;
                Ok(IndexedCompletedEvidence {
                    recording_id,
                    locator,
                })
            })
            .collect()
    }

    /// Makes a completed row non-playable after missing/corrupt evidence.
    ///
    /// # Errors
    /// Returns a classified database failure.
    pub async fn mark_inconsistent(
        &self,
        recording_id: &RecordingId,
    ) -> Result<bool, RepositoryError> {
        let changed = sqlx::query(
            "UPDATE recording_segments SET status = 'inconsistent', status_changed_at = clock_timestamp(), failure_category = 'reconciliation' WHERE recording_id = $1 AND status = 'completed'",
        )
        .bind(recording_id.as_str())
        .execute(&self.pool)
        .await
        .map_err(RepositoryError::from_sqlx)?
        .rows_affected();
        Ok(changed == 1)
    }

    /// Claims a bounded oldest-first age-policy batch before any unlink.
    ///
    /// # Errors
    /// Returns a classified database or invariant failure.
    pub async fn claim_retention_candidates(
        &self,
        age_cutoff: SystemTime,
        protected_cutoff: SystemTime,
        maximum_files: usize,
        maximum_bytes: u64,
    ) -> Result<Vec<RetentionCandidate>, RepositoryError> {
        if maximum_files == 0 || maximum_files > 1_000 || maximum_bytes == 0 {
            return Err(RepositoryError::invariant());
        }
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(RepositoryError::from_sqlx)?;
        let rows = sqlx::query(
            "SELECT recording_id, storage_locator, size_bytes FROM recording_segments WHERE status = 'completed' AND ended_at <= $1 AND ended_at < $2 ORDER BY ended_at, recording_id FOR UPDATE SKIP LOCKED LIMIT $3",
        )
        .bind(system_timestamp(age_cutoff))
        .bind(system_timestamp(protected_cutoff))
        .bind(i64::try_from(maximum_files).map_err(|_| RepositoryError::invariant())?)
        .fetch_all(&mut *transaction)
        .await
        .map_err(RepositoryError::from_sqlx)?;
        let mut candidates = Vec::with_capacity(rows.len());
        let mut claimed_bytes = 0_u64;
        for row in rows {
            let candidate = retention_candidate_from_row(&row)?;
            let Some(next_bytes) = claimed_bytes.checked_add(candidate.size_bytes) else {
                break;
            };
            if next_bytes > maximum_bytes {
                break;
            }
            sqlx::query("UPDATE recording_segments SET status = 'deletion_pending', status_changed_at = clock_timestamp() WHERE recording_id = $1 AND status = 'completed'")
                .bind(candidate.recording_id.as_str())
                .execute(&mut *transaction).await.map_err(RepositoryError::from_sqlx)?;
            claimed_bytes = next_bytes;
            candidates.push(candidate);
        }
        transaction
            .commit()
            .await
            .map_err(RepositoryError::from_sqlx)?;
        Ok(candidates)
    }

    /// Returns interrupted deletion claims for idempotent recovery.
    ///
    /// # Errors
    /// Returns a classified database failure or rejects an invalid bound.
    pub async fn deletion_pending_page(
        &self,
        limit: usize,
    ) -> Result<Vec<RetentionCandidate>, RepositoryError> {
        if limit == 0 || limit > 1_000 {
            return Err(RepositoryError::invariant());
        }
        let rows = sqlx::query("SELECT recording_id, storage_locator, size_bytes FROM recording_segments WHERE status = 'deletion_pending' ORDER BY status_changed_at, recording_id LIMIT $1")
            .bind(i64::try_from(limit).map_err(|_| RepositoryError::invariant())?)
            .fetch_all(&self.pool).await.map_err(RepositoryError::from_sqlx)?;
        rows.iter().map(retention_candidate_from_row).collect()
    }

    /// Finalizes an idempotent deletion transition after unlink and directory sync.
    ///
    /// # Errors
    /// Returns a classified database failure.
    pub async fn finalize_deletion(
        &self,
        recording_id: &RecordingId,
    ) -> Result<bool, RepositoryError> {
        let changed = sqlx::query("UPDATE recording_segments SET status = 'deleted', status_changed_at = clock_timestamp() WHERE recording_id = $1 AND status = 'deletion_pending'")
            .bind(recording_id.as_str()).execute(&self.pool).await.map_err(RepositoryError::from_sqlx)?.rows_affected();
        Ok(changed == 1)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DatabasePoolSnapshot {
    pub size: u32,
    pub idle: u32,
    pub maximum: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetentionCandidate {
    pub recording_id: RecordingId,
    pub locator: surveillance_storage::metadata::RecordingLocator,
    pub size_bytes: u64,
}

fn retention_candidate_from_row(
    row: &sqlx::postgres::PgRow,
) -> Result<RetentionCandidate, RepositoryError> {
    let recording_id = RecordingId::new(
        row.try_get::<String, _>("recording_id")
            .map_err(RepositoryError::from_sqlx)?,
    )
    .map_err(|_| RepositoryError::invariant())?;
    let locator = surveillance_storage::metadata::RecordingLocator::new(PathBuf::from(
        row.try_get::<String, _>("storage_locator")
            .map_err(RepositoryError::from_sqlx)?,
    ))
    .map_err(|_| RepositoryError::invariant())?;
    let size_bytes = row
        .try_get::<Option<i64>, _>("size_bytes")
        .map_err(RepositoryError::from_sqlx)?
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(RepositoryError::invariant)?;
    Ok(RetentionCandidate {
        recording_id,
        locator,
        size_bytes,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexedCompletedEvidence {
    pub recording_id: RecordingId,
    pub locator: surveillance_storage::metadata::RecordingLocator,
}

fn parse_status(value: &str) -> Result<RecordingStatus, RepositoryError> {
    match value {
        "open" => Ok(RecordingStatus::Open),
        "finalizing" => Ok(RecordingStatus::Finalizing),
        "completed" => Ok(RecordingStatus::Completed),
        "failed" => Ok(RecordingStatus::Failed),
        "incomplete" => Ok(RecordingStatus::Incomplete),
        "deletion_pending" => Ok(RecordingStatus::DeletionPending),
        "deleted" => Ok(RecordingStatus::Deleted),
        "quarantined" => Ok(RecordingStatus::Quarantined),
        "inconsistent" => Ok(RecordingStatus::Inconsistent),
        _ => Err(RepositoryError::invariant()),
    }
}

fn completed_from_row(row: &sqlx::postgres::PgRow) -> Result<CompletedRecording, RepositoryError> {
    let recording_id = surveillance_common::types::RecordingId::new(
        row.try_get::<String, _>("recording_id")
            .map_err(RepositoryError::from_sqlx)?,
    )
    .map_err(|_| RepositoryError::invariant())?;
    let camera_id = surveillance_common::types::CameraId::new(
        row.try_get::<String, _>("camera_id")
            .map_err(RepositoryError::from_sqlx)?,
    )
    .map_err(|_| RepositoryError::invariant())?;
    let started_at: DateTime<Utc> = row
        .try_get("started_at")
        .map_err(RepositoryError::from_sqlx)?;
    let ended_at: DateTime<Utc> = row
        .try_get("ended_at")
        .map_err(RepositoryError::from_sqlx)?;
    let locator = surveillance_storage::metadata::RecordingLocator::new(
        row.try_get::<String, _>("storage_locator")
            .map(PathBuf::from)
            .map_err(RepositoryError::from_sqlx)?,
    )
    .map_err(|_| RepositoryError::invariant())?;
    let container = match row
        .try_get::<Option<String>, _>("container")
        .map_err(RepositoryError::from_sqlx)?
        .as_deref()
    {
        Some("fragmented_mp4") => RecordingContainer::FragmentedMp4,
        _ => return Err(RepositoryError::invariant()),
    };
    let codec = match row
        .try_get::<Option<String>, _>("video_codec")
        .map_err(RepositoryError::from_sqlx)?
        .as_deref()
    {
        Some("h264") => VideoCodec::H264,
        _ => return Err(RepositoryError::invariant()),
    };
    let provenance = match row
        .try_get::<String, _>("provenance")
        .map_err(RepositoryError::from_sqlx)?
        .as_str()
    {
        "recorder" => MetadataProvenance::Recorder,
        "reconciliation" => MetadataProvenance::Reconciliation,
        _ => return Err(RepositoryError::invariant()),
    };
    let width = row
        .try_get::<Option<i32>, _>("width")
        .map_err(RepositoryError::from_sqlx)?
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(RepositoryError::invariant)?;
    let height = row
        .try_get::<Option<i32>, _>("height")
        .map_err(RepositoryError::from_sqlx)?
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(RepositoryError::invariant)?;
    let size = row
        .try_get::<Option<i64>, _>("size_bytes")
        .map_err(RepositoryError::from_sqlx)?
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(RepositoryError::invariant)?;
    CompletedRecording::new(
        recording_id,
        camera_id,
        started_at.into(),
        ended_at.into(),
        locator,
        container,
        codec,
        width,
        height,
        size,
        provenance,
    )
    .map_err(|_| RepositoryError::invariant())
}

fn row_matches(
    row: &sqlx::postgres::PgRow,
    completed: &CompletedRecording,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
) -> Result<bool, RepositoryError> {
    Ok(row
        .try_get::<String, _>("camera_id")
        .map_err(RepositoryError::from_sqlx)?
        == completed.camera_id().as_str()
        && row
            .try_get::<DateTime<Utc>, _>("started_at")
            .map_err(RepositoryError::from_sqlx)?
            == started_at
        && row
            .try_get::<Option<DateTime<Utc>>, _>("ended_at")
            .map_err(RepositoryError::from_sqlx)?
            == Some(ended_at)
        && row
            .try_get::<String, _>("storage_locator")
            .map_err(RepositoryError::from_sqlx)?
            == completed.locator().as_str()
        && row
            .try_get::<Option<String>, _>("container")
            .map_err(RepositoryError::from_sqlx)?
            .as_deref()
            == Some(container_text(completed.container()))
        && row
            .try_get::<Option<String>, _>("video_codec")
            .map_err(RepositoryError::from_sqlx)?
            .as_deref()
            == Some(codec_text(completed.codec()))
        && row
            .try_get::<Option<i32>, _>("width")
            .map_err(RepositoryError::from_sqlx)?
            == i32::try_from(completed.width()).ok()
        && row
            .try_get::<Option<i32>, _>("height")
            .map_err(RepositoryError::from_sqlx)?
            == i32::try_from(completed.height()).ok()
        && row
            .try_get::<Option<i64>, _>("size_bytes")
            .map_err(RepositoryError::from_sqlx)?
            == i64::try_from(completed.size_bytes()).ok()
        && row
            .try_get::<String, _>("status")
            .map_err(RepositoryError::from_sqlx)?
            == "completed"
        && row
            .try_get::<String, _>("provenance")
            .map_err(RepositoryError::from_sqlx)?
            == provenance_text(completed.provenance()))
}

fn system_timestamp(value: SystemTime) -> DateTime<Utc> {
    DateTime::<Utc>::from(value)
}

const fn container_text(value: RecordingContainer) -> &'static str {
    match value {
        RecordingContainer::FragmentedMp4 => "fragmented_mp4",
    }
}

const fn codec_text(value: VideoCodec) -> &'static str {
    match value {
        VideoCodec::H264 => "h264",
    }
}

const fn provenance_text(value: MetadataProvenance) -> &'static str {
    match value {
        MetadataProvenance::Recorder => "recorder",
        MetadataProvenance::Reconciliation => "reconciliation",
    }
}

const fn status_text(value: RecordingStatus) -> &'static str {
    match value {
        RecordingStatus::Failed => "failed",
        RecordingStatus::Incomplete => "incomplete",
        RecordingStatus::Open => "open",
        RecordingStatus::Finalizing => "finalizing",
        RecordingStatus::Completed => "completed",
        RecordingStatus::DeletionPending => "deletion_pending",
        RecordingStatus::Deleted => "deleted",
        RecordingStatus::Quarantined => "quarantined",
        RecordingStatus::Inconsistent => "inconsistent",
    }
}

const fn failure_text(value: RecordingFailureCategory) -> &'static str {
    match value {
        RecordingFailureCategory::Muxer => "muxer",
        RecordingFailureCategory::Storage => "storage",
        RecordingFailureCategory::Validation => "validation",
        RecordingFailureCategory::Shutdown => "shutdown",
        RecordingFailureCategory::Reconciliation => "reconciliation",
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersistCompletedOutcome {
    Inserted,
    Confirmed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepositoryErrorCategory {
    Unavailable,
    PoolTimedOut,
    Constraint,
    Conflict,
    Migration,
    Decode,
    Invariant,
}

pub struct RepositoryError {
    category: RepositoryErrorCategory,
    source: Option<Box<dyn Error + Send + Sync>>,
}

impl RepositoryError {
    #[must_use]
    pub const fn category(&self) -> RepositoryErrorCategory {
        self.category
    }

    #[must_use]
    pub const fn retryable(&self) -> bool {
        matches!(
            self.category,
            RepositoryErrorCategory::Unavailable | RepositoryErrorCategory::PoolTimedOut
        )
    }

    pub(crate) fn from_sqlx(error: sqlx::Error) -> Self {
        let category = match &error {
            sqlx::Error::PoolTimedOut => RepositoryErrorCategory::PoolTimedOut,
            sqlx::Error::PoolClosed | sqlx::Error::Io(_) | sqlx::Error::Tls(_) => {
                RepositoryErrorCategory::Unavailable
            }
            sqlx::Error::Database(database)
                if database.is_unique_violation()
                    || database.is_check_violation()
                    || database.is_foreign_key_violation() =>
            {
                RepositoryErrorCategory::Constraint
            }
            sqlx::Error::ColumnDecode { .. }
            | sqlx::Error::Decode(_)
            | sqlx::Error::ColumnNotFound(_)
            | sqlx::Error::TypeNotFound { .. } => RepositoryErrorCategory::Decode,
            _ => RepositoryErrorCategory::Unavailable,
        };
        Self {
            category,
            source: Some(Box::new(error)),
        }
    }

    pub(crate) fn migration(error: sqlx::migrate::MigrateError) -> Self {
        Self {
            category: RepositoryErrorCategory::Migration,
            source: Some(Box::new(error)),
        }
    }

    const fn conflict() -> Self {
        Self {
            category: RepositoryErrorCategory::Conflict,
            source: None,
        }
    }
    pub(crate) const fn invariant() -> Self {
        Self {
            category: RepositoryErrorCategory::Invariant,
            source: None,
        }
    }
}

impl fmt::Debug for RepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RepositoryError")
            .field("category", &self.category)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for RepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "recording repository operation failed: {:?}",
            self.category
        )
    }
}

impl Error for RepositoryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn Error + 'static))
    }
}
