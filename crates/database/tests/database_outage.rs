//! Proves durable recording preservation and later indexing across a `PostgreSQL` outage.

use std::{num::NonZeroU32, path::PathBuf, time::Duration};

use surveillance_database::{DatabaseConfig, PostgresRecordingRepository};
use surveillance_storage::metadata::{
    CompletedRecording, MetadataProvenance, RecordingContainer, RecordingLocator, RecordingStatus,
    VideoCodec,
};

#[tokio::test]
#[ignore = "orchestrated by scripts/phase4/postgres-fixture.sh"]
async fn durable_file_survives_database_outage_and_indexes_after_recovery() {
    let database_url = std::env::var("PHASE4_DATABASE_URL")
        .unwrap_or_else(|_| panic!("fixture database URL is required"));
    let coordination = PathBuf::from(
        std::env::var("PHASE4_OUTAGE_COORDINATION")
            .unwrap_or_else(|_| panic!("coordination directory is required")),
    );
    let durable = coordination.join("camera-outage/segment.mp4");
    tokio::fs::create_dir_all(durable.parent().unwrap_or(&coordination))
        .await
        .unwrap_or_else(|error| panic!("fixture directory: {error}"));
    let config = DatabaseConfig::new(
        &database_url,
        NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN),
        Duration::from_millis(250),
    )
    .unwrap_or_else(|error| panic!("database config: {error}"));
    let repository = PostgresRecordingRepository::connect(&config)
        .await
        .unwrap_or_else(|error| panic!("initial connection: {error}"));
    repository
        .migrate()
        .await
        .unwrap_or_else(|error| panic!("migration: {error}"));
    tokio::fs::write(coordination.join("ready"), b"ready")
        .await
        .unwrap_or_else(|error| panic!("ready marker: {error}"));
    wait_for(&coordination.join("database-down")).await;

    let source_media = PathBuf::from(
        std::env::var("PHASE4_OUTAGE_MEDIA")
            .unwrap_or_else(|_| panic!("outage media fixture is required")),
    );
    tokio::fs::copy(&source_media, &durable)
        .await
        .unwrap_or_else(|error| panic!("recording continues: {error}"));
    let size = tokio::fs::metadata(&durable)
        .await
        .unwrap_or_else(|error| panic!("durable metadata: {error}"))
        .len();
    let error = repository
        .recording_status(
            &surveillance_common::types::RecordingId::new("outage-recording")
                .unwrap_or_else(|error| panic!("recording ID: {error}")),
        )
        .await
        .expect_err("database operation must fail during outage");
    assert!(error.retryable());
    assert!(tokio::fs::try_exists(&durable).await.unwrap_or(false));
    tokio::fs::write(coordination.join("outage-observed"), b"observed")
        .await
        .unwrap_or_else(|error| panic!("outage marker: {error}"));
    wait_for(&coordination.join("database-restored")).await;

    let mut recovered = false;
    for _ in 0..40 {
        if repository.migrate().await.is_ok() {
            recovered = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(recovered, "pool must reconnect after PostgreSQL returns");
    assert!(tokio::fs::try_exists(&durable).await.unwrap_or(false));
    let recording_id = surveillance_common::types::RecordingId::new("outage-recording")
        .unwrap_or_else(|error| panic!("recording ID: {error}"));
    let completed = CompletedRecording::new(
        recording_id.clone(),
        surveillance_common::types::CameraId::new("camera-outage")
            .unwrap_or_else(|error| panic!("camera ID: {error}")),
        std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000),
        std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_002),
        RecordingLocator::new(PathBuf::from("camera-outage/segment.mp4"))
            .unwrap_or_else(|error| panic!("locator: {error}")),
        RecordingContainer::FragmentedMp4,
        VideoCodec::H264,
        320,
        240,
        size,
        MetadataProvenance::Reconciliation,
    )
    .unwrap_or_else(|error| panic!("completed evidence: {error}"));
    repository
        .insert_or_confirm_completed(&completed)
        .await
        .unwrap_or_else(|error| panic!("catch-up insert: {error}"));
    assert_eq!(
        repository
            .recording_status(&recording_id)
            .await
            .unwrap_or_else(|error| panic!("status: {error}")),
        Some(RecordingStatus::Completed)
    );
}

async fn wait_for(path: &std::path::Path) {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if tokio::fs::try_exists(path).await.unwrap_or(false) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {}", path.display()));
}
