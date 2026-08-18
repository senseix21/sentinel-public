//! Qualifies crash-safe retention ownership and protection of recent recordings.

use std::{
    num::{NonZeroU32, NonZeroU64, NonZeroUsize},
    path::PathBuf,
    time::{Duration, SystemTime},
};

use surveillance_common::types::{CameraId, RecordingId};
use surveillance_database::{DatabaseConfig, PostgresRecordingRepository};
use surveillance_server::retention_owner::RetentionOwner;
use surveillance_storage::{
    filesystem::RecordingLayout,
    metadata::{
        CompletedRecording, MetadataProvenance, RecordingContainer, RecordingLocator,
        RecordingStatus, VideoCodec,
    },
    retention::{RetentionPolicy, SafeRecordingDelete, StorageWatermarks},
};

#[tokio::test]
#[ignore = "requires scripts/phase4/postgres-fixture.sh"]
#[allow(
    clippy::too_many_lines,
    reason = "the integration scenario keeps claim, recovery, unlink, and protection visible together"
)]
async fn resumes_claimed_deletion_and_never_deletes_recent_recording() {
    let database_url = std::env::var("PHASE4_DATABASE_URL")
        .unwrap_or_else(|_| panic!("fixture database URL is required"));
    let root = PathBuf::from(
        std::env::var("PHASE4_RETENTION_ROOT")
            .unwrap_or_else(|_| panic!("fixture retention root is required")),
    );
    let database = DatabaseConfig::new(
        &database_url,
        NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN),
        Duration::from_secs(2),
    )
    .unwrap_or_else(|error| panic!("database config: {error}"));
    let repository = PostgresRecordingRepository::connect(&database)
        .await
        .unwrap_or_else(|error| panic!("database connect: {error}"));
    repository
        .migrate()
        .await
        .unwrap_or_else(|error| panic!("migration: {error}"));
    sqlx::query("DELETE FROM recording_segments")
        .execute(repository.pool())
        .await
        .unwrap_or_else(|error| panic!("fixture reset: {error}"));
    let camera =
        CameraId::new("retention-camera").unwrap_or_else(|error| panic!("camera ID: {error}"));
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
    for (recording, file, start) in [
        ("retention-old", "old.mp4", 100_u64),
        ("retention-recent", "recent.mp4", 9_995_u64),
    ] {
        let locator = RecordingLocator::new(PathBuf::from(format!("retention-camera/{file}")))
            .unwrap_or_else(|error| panic!("locator: {error}"));
        let path = root.join(locator.as_path());
        tokio::fs::create_dir_all(path.parent().unwrap_or(&root))
            .await
            .unwrap_or_else(|error| panic!("fixture directory: {error}"));
        tokio::fs::write(&path, b"recording")
            .await
            .unwrap_or_else(|error| panic!("fixture file: {error}"));
        let completed = CompletedRecording::new(
            RecordingId::new(recording).unwrap_or_else(|error| panic!("recording ID: {error}")),
            camera.clone(),
            SystemTime::UNIX_EPOCH + Duration::from_secs(start),
            SystemTime::UNIX_EPOCH + Duration::from_secs(start + 2),
            locator,
            RecordingContainer::FragmentedMp4,
            VideoCodec::H264,
            320,
            240,
            9,
            MetadataProvenance::Recorder,
        )
        .unwrap_or_else(|error| panic!("completed evidence: {error}"));
        repository
            .insert_or_confirm_completed(&completed)
            .await
            .unwrap_or_else(|error| panic!("insert: {error}"));
    }
    let claimed = repository
        .claim_retention_candidates(
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_000),
            SystemTime::UNIX_EPOCH + Duration::from_secs(9_990),
            1,
            100,
        )
        .await
        .unwrap_or_else(|error| panic!("claim: {error}"));
    assert_eq!(claimed.len(), 1);

    let layout = RecordingLayout::new(root.clone())
        .unwrap_or_else(|error| panic!("recording layout: {error}"));
    let policy = RetentionPolicy::new(
        Duration::from_secs(1_000),
        Duration::from_secs(10),
        NonZeroUsize::new(10).unwrap_or(NonZeroUsize::MIN),
        NonZeroU64::new(1_000).unwrap_or(NonZeroU64::MIN),
        StorageWatermarks::new(80, 90, 97, 2).unwrap_or_else(|error| panic!("watermarks: {error}")),
    )
    .unwrap_or_else(|error| panic!("retention policy: {error}"));
    let mut owner = RetentionOwner::new(
        repository.clone(),
        SafeRecordingDelete::new(layout),
        root.clone(),
        policy,
        Duration::from_secs(60),
    );
    let pass = owner
        .run_pass(now)
        .await
        .unwrap_or_else(|category| panic!("retention pass: {category}"));
    assert_eq!(pass.deleted, 1);
    assert!(!root.join("retention-camera/old.mp4").exists());
    assert!(root.join("retention-camera/recent.mp4").exists());
    let old =
        RecordingId::new("retention-old").unwrap_or_else(|error| panic!("recording ID: {error}"));
    let recent = RecordingId::new("retention-recent")
        .unwrap_or_else(|error| panic!("recording ID: {error}"));
    assert_eq!(
        repository
            .recording_status(&old)
            .await
            .unwrap_or_else(|error| panic!("old status: {error}")),
        Some(RecordingStatus::Deleted)
    );
    assert_eq!(
        repository
            .recording_status(&recent)
            .await
            .unwrap_or_else(|error| panic!("recent status: {error}")),
        Some(RecordingStatus::Completed)
    );
}
