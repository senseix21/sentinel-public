//! Qualifies MP4 playback assembly against the Phase 4 `PostgreSQL` fixture.
//!
//! Both routes are covered: multi-segment time-range assembly, and the
//! recording-video subresource in its full and leading-duration forms.

use std::{
    num::NonZeroU32,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use surveillance_common::types::{CameraId, RecordingId};
use surveillance_database::{DatabaseConfig, PostgresRecordingRepository};
use surveillance_server::playback::{
    ClipDuration, PlaybackAssembler, PlaybackStream, RecordingVideoExtent,
};
use surveillance_storage::{
    metadata::{
        CompletedRecording, MetadataProvenance, RecordingContainer, RecordingLocator, VideoCodec,
    },
    recordings::{PlaybackPlan, RecordingResultLimit, RecordingSearch, RecordingTimeRange},
};
use tokio::{io::AsyncWriteExt, process::Command};

const CAMERA: &str = "playback-camera";

fn camera() -> CameraId {
    CameraId::new(CAMERA).unwrap_or_else(|error| panic!("camera ID: {error}"))
}

fn fixture_root() -> PathBuf {
    PathBuf::from(
        std::env::var("PHASE4_PLAYBACK_ROOT")
            .unwrap_or_else(|_| panic!("fixture playback root is required")),
    )
}

async fn fixture_repository() -> PostgresRecordingRepository {
    let database_url = std::env::var("PHASE4_DATABASE_URL")
        .unwrap_or_else(|_| panic!("fixture database URL is required"));
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
    repository
}

async fn insert_segment(
    repository: &PostgresRecordingRepository,
    root: &Path,
    index: u64,
    start: u64,
) -> CompletedRecording {
    let locator = format!("{CAMERA}/segment-{index}.mp4");
    let size = tokio::fs::metadata(root.join(&locator))
        .await
        .unwrap_or_else(|error| panic!("fixture metadata: {error}"))
        .len();
    let completed = CompletedRecording::new(
        RecordingId::new(format!("playback-{index}"))
            .unwrap_or_else(|error| panic!("recording ID: {error}")),
        camera(),
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
    completed
}

/// Drains assembled media through the public reader.
///
/// Deliberately uses `PlaybackStream::read` rather than the child's pipe: the
/// reader replays the readiness prefetch, and a test that bypassed it would not
/// exercise the path the handlers use.
async fn drain_to(playback: &mut PlaybackStream, path: &Path) {
    let mut output = tokio::fs::File::create(path)
        .await
        .unwrap_or_else(|error| panic!("output: {error}"));
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = playback
            .read(&mut buffer)
            .await
            .unwrap_or_else(|error| panic!("read: {error}"));
        if read == 0 {
            break;
        }
        output
            .write_all(&buffer[..read])
            .await
            .unwrap_or_else(|error| panic!("write: {error}"));
    }
    output
        .flush()
        .await
        .unwrap_or_else(|error| panic!("flush: {error}"));
}

async fn assert_decodes(path: &Path) {
    let status = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-map", "0:v:0", "-f", "null", "-"])
        .status()
        .await
        .unwrap_or_else(|error| panic!("decode process: {error}"));
    assert!(status.success(), "{} did not fully decode", path.display());
}

fn assembler(root: PathBuf) -> PlaybackAssembler {
    PlaybackAssembler::new(root, PathBuf::from("ffmpeg"), Duration::from_secs(5))
}

#[tokio::test]
#[ignore = "requires scripts/phase4/postgres-fixture.sh"]
async fn three_segment_stream_copy_fully_decodes() {
    let root = fixture_root();
    let repository = fixture_repository().await;
    for (index, start) in [(1_u64, 100_u64), (2, 102), (3, 104)] {
        insert_segment(&repository, &root, index, start).await;
    }
    let range = RecordingTimeRange::new(
        SystemTime::UNIX_EPOCH + Duration::from_secs(101),
        SystemTime::UNIX_EPOCH + Duration::from_secs(105),
        Duration::from_secs(60),
    )
    .unwrap_or_else(|error| panic!("range: {error}"));
    let rows = repository
        .find_overlapping_completed(&RecordingSearch::new(
            camera(),
            range,
            RecordingResultLimit::new(10).unwrap_or_else(|error| panic!("limit: {error}")),
            None,
        ))
        .await
        .unwrap_or_else(|error| panic!("query: {error}"));
    let plan = PlaybackPlan::new(range, rows).unwrap_or_else(|error| panic!("plan: {error}"));
    let mut playback = assembler(root.clone())
        .start(&plan)
        .await
        .unwrap_or_else(|error| panic!("start: {error}"));
    let output_path = root.join("assembled.mp4");
    drain_to(&mut playback, &output_path).await;
    playback
        .finish()
        .await
        .unwrap_or_else(|error| panic!("finish: {error}"));
    assert_decodes(&output_path).await;
}

#[tokio::test]
#[ignore = "requires scripts/phase4/postgres-fixture.sh"]
async fn recording_video_full_and_leading_both_decode() {
    let root = fixture_root();
    let repository = fixture_repository().await;
    let recording = insert_segment(&repository, &root, 1, 100).await;
    let built = assembler(root.clone());

    for (extent, name) in [
        (RecordingVideoExtent::Full, "recording-full.mp4"),
        (
            RecordingVideoExtent::Leading(
                ClipDuration::new(1).unwrap_or_else(|_| panic!("one second is valid")),
            ),
            "recording-leading.mp4",
        ),
    ] {
        let mut playback = built
            .start_recording(&recording, extent)
            .await
            .unwrap_or_else(|error| panic!("start_recording {name}: {error}"));
        let output_path = root.join(name);
        drain_to(&mut playback, &output_path).await;
        playback
            .finish()
            .await
            .unwrap_or_else(|error| panic!("finish {name}: {error}"));
        assert_decodes(&output_path).await;
    }
}

#[tokio::test]
#[ignore = "requires scripts/phase4/postgres-fixture.sh"]
async fn an_unreadable_recording_is_refused_before_any_media() {
    let root = fixture_root();
    let repository = fixture_repository().await;
    let recording = insert_segment(&repository, &root, 1, 100).await;
    let truncated = root.join(CAMERA).join("unreadable.mp4");
    let invalid_media = b"not an MP4 file";
    tokio::fs::write(&truncated, invalid_media)
        .await
        .unwrap_or_else(|error| panic!("write unreadable fixture: {error}"));

    // A durable locator and nonzero size pass the metadata contract, so this
    // reaches the readiness gate on existing bytes FFmpeg cannot remux. A
    // successful spawn must not be reported as a ready media stream.
    let empty = CompletedRecording::new(
        RecordingId::new("playback-empty").unwrap_or_else(|error| panic!("recording ID: {error}")),
        camera(),
        SystemTime::UNIX_EPOCH + Duration::from_secs(200),
        SystemTime::UNIX_EPOCH + Duration::from_secs(202),
        RecordingLocator::new(PathBuf::from(format!("{CAMERA}/unreadable.mp4")))
            .unwrap_or_else(|error| panic!("locator: {error}")),
        RecordingContainer::FragmentedMp4,
        VideoCodec::H264,
        320,
        240,
        invalid_media.len() as u64,
        MetadataProvenance::Recorder,
    )
    .unwrap_or_else(|error| panic!("completed evidence: {error}"));
    drop(recording);

    let outcome = assembler(root)
        .start_recording(&empty, RecordingVideoExtent::Full)
        .await;
    assert!(
        outcome.is_err(),
        "a recording FFmpeg cannot open must fail before the handler writes 200 OK"
    );
}
