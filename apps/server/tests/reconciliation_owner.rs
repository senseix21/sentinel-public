//! Qualifies owned reconciliation of filesystem and recording-index divergence.

use std::{num::NonZeroU32, path::PathBuf, time::Duration};

use surveillance_common::types::{CameraId, RecordingId};
use surveillance_database::{DatabaseConfig, PostgresRecordingRepository};
use surveillance_server::reconciliation_owner::ReconciliationOwner;
use surveillance_storage::{
    filesystem::RecordingLayout,
    metadata::{
        CompletedRecording, MetadataProvenance, RecordingContainer, RecordingLocator,
        RecordingStatus, VideoCodec,
    },
    reconciliation::ReconciliationLimit,
    segment_store::SegmentFileStore,
};

fn required_environment(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is set by the Phase 4 fixture"))
}

#[tokio::test]
#[ignore = "requires scripts/phase4/postgres-fixture.sh"]
#[allow(
    clippy::too_many_lines,
    reason = "the integration scenario keeps the four divergence cases visible together"
)]
async fn repairs_filesystem_database_divergence_idempotently() {
    let database = DatabaseConfig::new(
        &required_environment("PHASE4_DATABASE_URL"),
        NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN),
        Duration::from_secs(2),
    )
    .unwrap_or_else(|error| panic!("database configuration: {error}"));
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

    let root = PathBuf::from(required_environment("PHASE4_RECONCILIATION_ROOT"));
    let source = PathBuf::from(required_environment("PHASE4_OUTAGE_MEDIA"));
    let directory = root.join("reconcile-camera/1970/01/01");
    tokio::fs::create_dir_all(&directory)
        .await
        .unwrap_or_else(|error| panic!("fixture directory: {error}"));
    let orphan = directory.join("00-16-40--reconcile-orphan.mp4");
    tokio::fs::copy(&source, &orphan)
        .await
        .unwrap_or_else(|error| panic!("orphan copy: {error}"));
    let partial = directory.join("00-16-41--reconcile-partial.partial.mp4");
    tokio::fs::write(&partial, b"durable crash evidence")
        .await
        .unwrap_or_else(|error| panic!("partial evidence: {error}"));
    let corrupt = directory.join("00-16-42--reconcile-corrupt.mp4");
    tokio::fs::write(&corrupt, b"not an mp4")
        .await
        .unwrap_or_else(|error| panic!("corrupt evidence: {error}"));

    let missing = CompletedRecording::new(
        RecordingId::new("reconcile-missing")
            .unwrap_or_else(|error| panic!("recording ID: {error}")),
        CameraId::new("reconcile-camera").unwrap_or_else(|error| panic!("camera ID: {error}")),
        std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_003),
        std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_005),
        RecordingLocator::new(PathBuf::from(
            "reconcile-camera/1970/01/01/00-16-43--reconcile-missing.mp4",
        ))
        .unwrap_or_else(|error| panic!("locator: {error}")),
        RecordingContainer::FragmentedMp4,
        VideoCodec::H264,
        320,
        240,
        1,
        MetadataProvenance::Recorder,
    )
    .unwrap_or_else(|error| panic!("missing evidence: {error}"));
    repository
        .insert_or_confirm_completed(&missing)
        .await
        .unwrap_or_else(|error| panic!("missing row insert: {error}"));

    let layout =
        RecordingLayout::new(root).unwrap_or_else(|error| panic!("recording layout: {error}"));
    let store = SegmentFileStore::new(layout.clone(), PathBuf::from("ffprobe"));
    let mut owner = ReconciliationOwner::new(
        repository.clone(),
        layout,
        store,
        ReconciliationLimit::new(32).unwrap_or_else(|error| panic!("scan limit: {error}")),
        32,
        Duration::ZERO,
        Duration::from_secs(60),
    );
    let first = owner
        .run_pass()
        .await
        .unwrap_or_else(|category| panic!("first pass: {category}"));
    assert_eq!(first.imported, 1);
    assert_eq!(first.incomplete, 1);
    assert_eq!(first.inconsistent, 2);
    assert_eq!(
        repository
            .recording_status(
                &RecordingId::new("reconcile-orphan")
                    .unwrap_or_else(|error| panic!("recording ID: {error}"))
            )
            .await
            .unwrap_or_else(|error| panic!("orphan status: {error}")),
        Some(RecordingStatus::Completed)
    );
    for (recording, expected) in [
        ("reconcile-partial", RecordingStatus::Incomplete),
        ("reconcile-corrupt", RecordingStatus::Inconsistent),
        ("reconcile-missing", RecordingStatus::Inconsistent),
    ] {
        let id =
            RecordingId::new(recording).unwrap_or_else(|error| panic!("recording ID: {error}"));
        assert_eq!(
            repository
                .recording_status(&id)
                .await
                .unwrap_or_else(|error| panic!("recording status: {error}")),
            Some(expected)
        );
    }
    // Once the orphan is indexed, a second repair pass must not need ffprobe
    // to validate immutable path-and-size evidence. A deliberately missing
    // executable makes a redundant probe fail the pass instead of hiding in
    // timing data.
    let layout = RecordingLayout::new(PathBuf::from(required_environment(
        "PHASE4_RECONCILIATION_ROOT",
    )))
    .unwrap_or_else(|error| panic!("recording layout: {error}"));
    let mut owner = ReconciliationOwner::new(
        repository.clone(),
        layout.clone(),
        SegmentFileStore::new(layout, PathBuf::from("missing-ffprobe-by-design")),
        ReconciliationLimit::new(32).unwrap_or_else(|error| panic!("scan limit: {error}")),
        32,
        Duration::ZERO,
        Duration::from_secs(60),
    );
    let second = owner
        .run_pass()
        .await
        .unwrap_or_else(|category| panic!("second pass: {category}"));
    assert_eq!(second.imported, 0);
    assert_eq!(second.incomplete, 0);
    assert_eq!(second.skipped_indexed, 2);
}
