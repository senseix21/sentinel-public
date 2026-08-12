use std::{
    num::NonZeroU32,
    path::PathBuf,
    time::{Duration, SystemTime},
};

use surveillance_common::types::{CameraId, RecordingId};
use surveillance_database::{DatabaseConfig, PostgresRecordingRepository};
use surveillance_server::playback::PlaybackAssembler;
use surveillance_storage::{
    metadata::{
        CompletedRecording, MetadataProvenance, RecordingContainer, RecordingLocator, VideoCodec,
    },
    recordings::{PlaybackPlan, RecordingResultLimit, RecordingSearch, RecordingTimeRange},
};
use tokio::process::Command;

#[tokio::test]
#[ignore = "requires scripts/phase4/postgres-fixture.sh"]
async fn three_segment_stream_copy_fully_decodes() {
    let database_url = std::env::var("PHASE4_DATABASE_URL")
        .unwrap_or_else(|_| panic!("fixture database URL is required"));
    let root = PathBuf::from(
        std::env::var("PHASE4_PLAYBACK_ROOT")
            .unwrap_or_else(|_| panic!("fixture playback root is required")),
    );
    let config = DatabaseConfig::new(
        &database_url,
        NonZeroU32::new(2).unwrap_or(NonZeroU32::MIN),
        Duration::from_secs(2),
    )
    .unwrap_or_else(|error| panic!("database config: {error}"));
    let repository = PostgresRecordingRepository::connect(&config)
        .await
        .unwrap_or_else(|error| panic!("database connect: {error}"));
    repository
        .migrate()
        .await
        .unwrap_or_else(|error| panic!("migration: {error}"));
    for (index, start) in [(1_u64, 100_u64), (2, 102), (3, 104)] {
        let locator = format!("playback-camera/segment-{index}.mp4");
        let size = tokio::fs::metadata(root.join(&locator))
            .await
            .unwrap_or_else(|error| panic!("fixture metadata: {error}"))
            .len();
        let completed = CompletedRecording::new(
            RecordingId::new(format!("playback-{index}"))
                .unwrap_or_else(|error| panic!("recording ID: {error}")),
            CameraId::new("playback-camera").unwrap_or_else(|error| panic!("camera ID: {error}")),
            SystemTime::UNIX_EPOCH + Duration::from_secs(start),
            SystemTime::UNIX_EPOCH + Duration::from_secs(start + 2),
            RecordingLocator::new(PathBuf::from(locator))
                .unwrap_or_else(|error| panic!("locator: {error}")),
            RecordingContainer::FragmentedMp4,
            VideoCodec::H264,
            320,
            240,
            size,
            MetadataProvenance::Recorder,
        )
        .unwrap_or_else(|error| panic!("completed evidence: {error}"));
        repository
            .insert_or_confirm_completed(&completed)
            .await
            .unwrap_or_else(|error| panic!("insert: {error}"));
    }
    let range = RecordingTimeRange::new(
        SystemTime::UNIX_EPOCH + Duration::from_secs(101),
        SystemTime::UNIX_EPOCH + Duration::from_secs(105),
        Duration::from_secs(60),
    )
    .unwrap_or_else(|error| panic!("range: {error}"));
    let rows = repository
        .find_overlapping_completed(&RecordingSearch::new(
            CameraId::new("playback-camera").unwrap_or_else(|error| panic!("camera: {error}")),
            range,
            RecordingResultLimit::new(10).unwrap_or_else(|error| panic!("limit: {error}")),
            None,
        ))
        .await
        .unwrap_or_else(|error| panic!("query: {error}"));
    let plan = PlaybackPlan::new(range, rows).unwrap_or_else(|error| panic!("plan: {error}"));
    let assembler = PlaybackAssembler::new(
        root.clone(),
        PathBuf::from("ffmpeg"),
        Duration::from_secs(5),
    );
    let mut playback = assembler
        .start(&plan)
        .await
        .unwrap_or_else(|error| panic!("start: {error}"));
    let output_path = root.join("assembled.mp4");
    let mut output = tokio::fs::File::create(&output_path)
        .await
        .unwrap_or_else(|error| panic!("output: {error}"));
    tokio::io::copy(playback.stdout_mut(), &mut output)
        .await
        .unwrap_or_else(|error| panic!("copy: {error}"));
    playback
        .finish()
        .await
        .unwrap_or_else(|error| panic!("finish: {error}"));
    let status = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&output_path)
        .args(["-map", "0:v:0", "-f", "null", "-"])
        .status()
        .await
        .unwrap_or_else(|error| panic!("decode process: {error}"));
    assert!(status.success());
}
