//! Real file/index/playback recovery for read-only folder selection.
use std::{
    num::{NonZeroU32, NonZeroU64},
    path::PathBuf,
    time::Duration,
};
use surveillance_common::types::{CameraId, RecordingId};
use surveillance_database::{DatabaseConfig, PostgresRecordingRepository};
use surveillance_server::{playback::RecordingVideoExtent, recording_folders::RecordingFolders};
use surveillance_storage::reconciliation::ReconciliationLimit;
use tokio::sync::watch;

#[allow(
    clippy::too_many_lines,
    reason = "one end-to-end lifetime covers indexing, owner restart and playback"
)]
#[tokio::test]
#[ignore = "requires scripts/phase4/postgres-fixture.sh"]
async fn folder_index_survives_restart_plays_original_and_leaves_media_untouched(
) -> Result<(), Box<dyn std::error::Error>> {
    let database = DatabaseConfig::new(
        &std::env::var("PHASE4_DATABASE_URL")?,
        NonZeroU32::new(2).ok_or("pool")?,
        Duration::from_secs(2),
    )?;
    let repository = PostgresRecordingRepository::connect(&database).await?;
    repository.migrate().await?;
    let base = PathBuf::from(std::env::var("PHASE4_RECORDING_FOLDERS_ROOT")?);
    let approved = base.join("archives");
    let primary = base.join("primary");
    let root = approved.join("saved");
    let path = root.join("folder-camera/2026/10/29/17-46-23--folder-fixture.mp4");
    tokio::fs::create_dir_all(path.parent().ok_or("parent")?).await?;
    tokio::fs::create_dir_all(&primary).await?;
    tokio::fs::copy(std::env::var("PHASE4_OUTAGE_MEDIA")?, &path).await?;
    let original = tokio::fs::read(&path).await?;
    let (folders, owner) = RecordingFolders::new(
        vec![approved.clone()],
        primary.clone(),
        repository.clone(),
        "ffprobe".into(),
        "ffmpeg".into(),
        NonZeroU64::new(5000).ok_or("probe timeout")?,
        ReconciliationLimit::new(1000)?,
    )
    .await?;
    let choices = folders.discover().await.map_err(std::io::Error::other)?;
    assert_eq!(choices.len(), 1);
    let (shutdown, receiver) = watch::channel(false);
    let task = tokio::spawn(owner.run(receiver));
    let mut source = None;
    for _ in 0..100 {
        match folders.index(&choices[0].id).await {
            Ok(id) => {
                source = Some(id);
                break;
            }
            Err("index_worker_starting") => tokio::time::sleep(Duration::from_millis(20)).await,
            Err(category) => return Err(category.into()),
        }
    }
    let source = source.ok_or("worker did not initialize")?;
    let mut indexed = false;
    for _ in 0..200 {
        let sources = repository.recording_sources().await?;
        if sources
            .iter()
            .any(|entry| entry.source_id == source && entry.index_state == "ready")
        {
            indexed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(indexed, "index job should complete");
    shutdown.send_replace(true);
    task.await?;
    drop(folders);
    let (restored, owner) = RecordingFolders::new(
        vec![approved],
        primary,
        repository.clone(),
        "ffprobe".into(),
        "ffmpeg".into(),
        NonZeroU64::new(5000).ok_or("timeout")?,
        ReconciliationLimit::new(1000)?,
    )
    .await?;
    drop(owner);
    let (scoped, assembler) = restored
        .resolve(&source)
        .await
        .map_err(std::io::Error::other)?;
    let camera = CameraId::new("folder-camera")?;
    let id = RecordingId::new("folder-fixture")?;
    assert!(
        repository.find_completed(&camera, &id).await?.is_none(),
        "archive must not enter primary index"
    );
    let other_camera = CameraId::new("not-in-this-source")?;
    assert_eq!(
        restored
            .resolve_for_camera(&source, &other_camera)
            .await
            .err(),
        Some("source_not_found")
    );
    assert_eq!(
        restored
            .resolve_for_camera("archive-999999", &camera)
            .await
            .err(),
        Some("source_not_found")
    );
    let row = scoped
        .find_completed(&camera, &id)
        .await?
        .ok_or("indexed recording")?;
    let mut stream = assembler
        .start_recording(&row, RecordingVideoExtent::Full)
        .await?;
    let mut bytes = 0;
    let mut buffer = [0_u8; 4096];
    loop {
        let count = stream.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        bytes += count;
    }
    stream.finish().await?;
    assert!(bytes > 0);
    assert_eq!(
        tokio::fs::read(path).await?,
        original,
        "index and playback must leave source untouched"
    );
    Ok(())
}
