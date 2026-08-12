use std::{
    num::NonZeroU32,
    path::PathBuf,
    time::{Duration, SystemTime},
};

use surveillance_common::types::{CameraId, RecordingId};
use surveillance_database::{
    DatabaseConfig, PersistCompletedOutcome, PostgresRecordingRepository, RepositoryErrorCategory,
};
use surveillance_storage::metadata::{
    CompletedRecording, MetadataProvenance, NonPlayableRecording, RecordingContainer,
    RecordingEvidenceLocator, RecordingFailureCategory, RecordingLocator, RecordingStatus,
    VideoCodec,
};
use surveillance_storage::recordings::{
    RecordingCursor, RecordingResultLimit, RecordingSearch, RecordingTimeRange,
};

fn completed(recording: &str, camera: &str, locator: &str, started: u64) -> CompletedRecording {
    CompletedRecording::new(
        RecordingId::new(recording).unwrap_or_else(|error| panic!("valid recording ID: {error}")),
        CameraId::new(camera).unwrap_or_else(|error| panic!("valid camera ID: {error}")),
        SystemTime::UNIX_EPOCH + Duration::from_secs(started),
        SystemTime::UNIX_EPOCH + Duration::from_secs(started + 60),
        RecordingLocator::new(PathBuf::from(locator))
            .unwrap_or_else(|error| panic!("valid locator: {error}")),
        RecordingContainer::FragmentedMp4,
        VideoCodec::H264,
        1920,
        1080,
        4_000_000,
        MetadataProvenance::Recorder,
    )
    .unwrap_or_else(|error| panic!("valid completed evidence: {error}"))
}

fn database_url() -> String {
    std::env::var("PHASE4_DATABASE_URL").unwrap_or_else(|_| {
        panic!("PHASE4_DATABASE_URL is set by scripts/phase4/postgres-fixture.sh")
    })
}

async fn repository(maximum: u32, timeout: Duration) -> PostgresRecordingRepository {
    let config = DatabaseConfig::new(
        &database_url(),
        NonZeroU32::new(maximum).unwrap_or(NonZeroU32::MIN),
        timeout,
    )
    .unwrap_or_else(|error| panic!("valid database configuration: {error}"));
    PostgresRecordingRepository::connect(&config)
        .await
        .unwrap_or_else(|error| panic!("repository connects: {error}"))
}

#[tokio::test]
#[ignore = "requires scripts/phase4/postgres-fixture.sh"]
async fn migrations_apply_from_empty_and_are_rerunnable() {
    let repository = repository(2, Duration::from_secs(2)).await;
    repository
        .migrate()
        .await
        .unwrap_or_else(|error| panic!("empty database migrates: {error}"));
    repository
        .migrate()
        .await
        .unwrap_or_else(|error| panic!("migration rerun succeeds: {error}"));
}

#[tokio::test]
#[ignore = "requires scripts/phase4/postgres-fixture.sh"]
async fn insert_or_confirm_is_idempotent_and_conflicts_are_typed() {
    let repository = repository(2, Duration::from_secs(2)).await;
    repository
        .migrate()
        .await
        .unwrap_or_else(|error| panic!("migration succeeds: {error}"));
    let evidence = completed(
        "repository-segment",
        "repository-camera",
        "repository-camera/segment.mp4",
        100,
    );
    assert_eq!(
        repository
            .insert_or_confirm_completed(&evidence)
            .await
            .unwrap_or_else(|error| panic!("insert succeeds: {error}")),
        PersistCompletedOutcome::Inserted
    );
    assert_eq!(
        repository
            .insert_or_confirm_completed(&evidence)
            .await
            .unwrap_or_else(|error| panic!("confirm succeeds: {error}")),
        PersistCompletedOutcome::Confirmed
    );

    let conflict = completed(
        "repository-segment",
        "repository-camera",
        "repository-camera/different.mp4",
        100,
    );
    let error = repository
        .insert_or_confirm_completed(&conflict)
        .await
        .expect_err("conflicting immutable evidence must fail");
    assert_eq!(error.category(), RepositoryErrorCategory::Conflict);
    assert!(!error.retryable());
}

#[tokio::test]
#[ignore = "requires scripts/phase4/postgres-fixture.sh"]
async fn transaction_rolls_back_camera_when_recording_constraint_fails() {
    let repository = repository(2, Duration::from_secs(2)).await;
    repository
        .migrate()
        .await
        .unwrap_or_else(|error| panic!("migration succeeds: {error}"));
    let first = completed(
        "locator-owner",
        "existing-camera",
        "shared/segment.mp4",
        200,
    );
    repository
        .insert_or_confirm_completed(&first)
        .await
        .unwrap_or_else(|error| panic!("first insert succeeds: {error}"));
    let collision = completed(
        "locator-collision",
        "rolled-back-camera",
        "shared/segment.mp4",
        300,
    );
    let error = repository
        .insert_or_confirm_completed(&collision)
        .await
        .expect_err("unique locator collision must fail");
    assert_eq!(error.category(), RepositoryErrorCategory::Constraint);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM cameras WHERE camera_id = 'rolled-back-camera'")
            .fetch_one(repository.pool())
            .await
            .unwrap_or_else(|error| panic!("count query succeeds: {error}"));
    assert_eq!(count, 0);
}

#[tokio::test]
#[ignore = "requires scripts/phase4/postgres-fixture.sh"]
async fn pool_timeout_is_typed_and_retryable() {
    let repository = repository(1, Duration::from_millis(50)).await;
    repository
        .migrate()
        .await
        .unwrap_or_else(|error| panic!("migration succeeds: {error}"));
    let held = repository
        .pool()
        .acquire()
        .await
        .unwrap_or_else(|error| panic!("connection acquired: {error}"));
    let error = repository
        .insert_or_confirm_completed(&completed(
            "pool-timeout",
            "pool-camera",
            "pool/segment.mp4",
            400,
        ))
        .await
        .expect_err("exhausted bounded pool must time out");
    drop(held);
    assert_eq!(error.category(), RepositoryErrorCategory::PoolTimedOut);
    assert!(error.retryable());
}

#[tokio::test]
#[ignore = "requires scripts/phase4/postgres-fixture.sh"]
async fn failed_and_incomplete_replay_is_idempotent_and_never_playable() {
    let repository = repository(2, Duration::from_secs(2)).await;
    repository
        .migrate()
        .await
        .unwrap_or_else(|error| panic!("migration succeeds: {error}"));
    let evidence = NonPlayableRecording::new(
        RecordingId::new("failed-segment")
            .unwrap_or_else(|error| panic!("valid recording: {error}")),
        CameraId::new("failed-camera").unwrap_or_else(|error| panic!("valid camera: {error}")),
        SystemTime::UNIX_EPOCH + Duration::from_secs(500),
        None,
        RecordingEvidenceLocator::new(PathBuf::from("failed-camera/segment.partial.mp4"))
            .unwrap_or_else(|error| panic!("valid evidence path: {error}")),
        RecordingStatus::Incomplete,
        RecordingFailureCategory::Shutdown,
        MetadataProvenance::Reconciliation,
    )
    .unwrap_or_else(|error| panic!("valid incomplete evidence: {error}"));
    assert_eq!(
        repository
            .insert_or_confirm_non_playable(&evidence)
            .await
            .unwrap_or_else(|error| panic!("insert succeeds: {error}")),
        PersistCompletedOutcome::Inserted
    );
    assert_eq!(
        repository
            .insert_or_confirm_non_playable(&evidence)
            .await
            .unwrap_or_else(|error| panic!("replay succeeds: {error}")),
        PersistCompletedOutcome::Confirmed
    );
    let playable: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM recording_segments WHERE recording_id = 'failed-segment' AND status = 'completed'",
    )
    .fetch_one(repository.pool())
    .await
    .unwrap_or_else(|error| panic!("query succeeds: {error}"));
    assert_eq!(playable, 0);
}

#[tokio::test]
#[ignore = "requires scripts/phase4/postgres-fixture.sh"]
async fn overlap_query_is_camera_scoped_half_open_ordered_and_paginated() {
    let repository = repository(2, Duration::from_secs(2)).await;
    repository
        .migrate()
        .await
        .unwrap_or_else(|error| panic!("migration succeeds: {error}"));
    for (id, camera, start) in [
        ("query-1", "query-camera", 600),
        ("query-2", "query-camera", 660),
        ("query-3", "query-camera", 720),
        ("query-other", "other-camera", 660),
    ] {
        repository
            .insert_or_confirm_completed(&completed(
                id,
                camera,
                &format!("{camera}/{id}.mp4"),
                start,
            ))
            .await
            .unwrap_or_else(|error| panic!("fixture insert succeeds: {error}"));
    }
    let range = RecordingTimeRange::new(
        SystemTime::UNIX_EPOCH + Duration::from_secs(630),
        SystemTime::UNIX_EPOCH + Duration::from_secs(750),
        Duration::from_secs(3_600),
    )
    .unwrap_or_else(|error| panic!("valid query range: {error}"));
    let first = repository
        .find_overlapping_completed(&RecordingSearch::new(
            CameraId::new("query-camera").unwrap_or_else(|error| panic!("valid camera: {error}")),
            range,
            RecordingResultLimit::new(2).unwrap_or_else(|error| panic!("valid limit: {error}")),
            None,
        ))
        .await
        .unwrap_or_else(|error| panic!("query succeeds: {error}"));
    assert_eq!(
        first
            .iter()
            .map(|row| row.recording_id().as_str())
            .collect::<Vec<_>>(),
        vec!["query-1", "query-2"]
    );
    let cursor = RecordingCursor::new(first[1].started_at(), first[1].recording_id().clone());
    let second = repository
        .find_overlapping_completed(&RecordingSearch::new(
            CameraId::new("query-camera").unwrap_or_else(|error| panic!("valid camera: {error}")),
            range,
            RecordingResultLimit::new(2).unwrap_or_else(|error| panic!("valid limit: {error}")),
            Some(cursor),
        ))
        .await
        .unwrap_or_else(|error| panic!("query succeeds: {error}"));
    assert_eq!(
        second
            .iter()
            .map(|row| row.recording_id().as_str())
            .collect::<Vec<_>>(),
        vec!["query-3"]
    );

    let boundary = RecordingTimeRange::new(
        SystemTime::UNIX_EPOCH + Duration::from_secs(660),
        SystemTime::UNIX_EPOCH + Duration::from_secs(720),
        Duration::from_secs(3_600),
    )
    .unwrap_or_else(|error| panic!("valid boundary range: {error}"));
    let touching = repository
        .find_overlapping_completed(&RecordingSearch::new(
            CameraId::new("query-camera").unwrap_or_else(|error| panic!("valid camera: {error}")),
            boundary,
            RecordingResultLimit::new(10).unwrap_or_else(|error| panic!("valid limit: {error}")),
            None,
        ))
        .await
        .unwrap_or_else(|error| panic!("boundary query succeeds: {error}"));
    assert_eq!(
        touching
            .iter()
            .map(|row| row.recording_id().as_str())
            .collect::<Vec<_>>(),
        vec!["query-2"]
    );
}

#[tokio::test]
#[ignore = "requires scripts/phase4/postgres-fixture.sh"]
async fn retention_claim_is_oldest_first_bounded_and_recoverable() {
    let repository = repository(2, Duration::from_secs(2)).await;
    repository
        .migrate()
        .await
        .unwrap_or_else(|error| panic!("migration: {error}"));
    sqlx::query("DELETE FROM recording_segments")
        .execute(repository.pool())
        .await
        .unwrap_or_else(|error| panic!("fixture reset: {error}"));
    for (id, start) in [
        ("retain-old-1", 1_000),
        ("retain-old-2", 1_100),
        ("retain-recent", 2_000),
    ] {
        repository
            .insert_or_confirm_completed(&completed(
                id,
                "retention-camera",
                &format!("retention-camera/{id}.mp4"),
                start,
            ))
            .await
            .unwrap_or_else(|error| panic!("insert: {error}"));
    }
    let claimed = repository
        .claim_retention_candidates(
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_500),
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_900),
            1,
            5_000_000,
        )
        .await
        .unwrap_or_else(|error| panic!("claim: {error}"));
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].recording_id.as_str(), "retain-old-1");
    assert_eq!(
        repository
            .recording_status(&claimed[0].recording_id)
            .await
            .unwrap_or_else(|error| panic!("status: {error}")),
        Some(RecordingStatus::DeletionPending)
    );
    let resumed = repository
        .deletion_pending_page(10)
        .await
        .unwrap_or_else(|error| panic!("pending: {error}"));
    assert!(resumed
        .iter()
        .any(|item| item.recording_id == claimed[0].recording_id));
    assert!(repository
        .finalize_deletion(&claimed[0].recording_id)
        .await
        .unwrap_or_else(|error| panic!("finalize: {error}")));
    assert_eq!(
        repository
            .recording_status(&claimed[0].recording_id)
            .await
            .unwrap_or_else(|error| panic!("status: {error}")),
        Some(RecordingStatus::Deleted)
    );

    let none = repository
        .claim_retention_candidates(
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_500),
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_900),
            10,
            1,
        )
        .await
        .unwrap_or_else(|error| panic!("bounded claim: {error}"));
    assert!(none.is_empty());
    assert_eq!(
        repository
            .recording_status(
                &RecordingId::new("retain-recent").unwrap_or_else(|error| panic!("id: {error}"))
            )
            .await
            .unwrap_or_else(|error| panic!("status: {error}")),
        Some(RecordingStatus::Completed)
    );
}
