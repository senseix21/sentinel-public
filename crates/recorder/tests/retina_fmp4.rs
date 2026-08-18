//! Qualifies Retina ingest through probeable, decodable fragmented-MP4 segmentation.

use std::{path::PathBuf, process::Command, time::Duration};

use surveillance_camera::rtsp::{
    connect_h264, RtspCredentials, RtspEndpoint, RtspFrameLimits, RtspReadOutcome, RtspSourceConfig,
};
use surveillance_common::types::CameraId;
use surveillance_media::distribution::{MediaDistributor, PublishOutcome, QueueCapacity};
use surveillance_recorder::{
    clock::{ClockDriftTolerance, MediaWallClock},
    fmp4::{Fmp4SegmentSink, Fmp4SinkConfig, SegmentDuration},
    runtime::{run_segment_recorder, RecordingLifecycleEvent},
};
use surveillance_storage::{filesystem::RecordingLayout, segment_store::SegmentFileStore};

fn source_config() -> RtspSourceConfig {
    let endpoint = RtspEndpoint::parse("rtsp://127.0.0.1:18554/retina-fixture")
        .unwrap_or_else(|error| panic!("fixture endpoint must be valid: {error}"));
    let credentials = RtspCredentials::new(
        "fixture-reader".to_owned(),
        "fixture-reader-pass".to_owned(),
    )
    .unwrap_or_else(|error| panic!("fixture credentials must be valid: {error}"));
    let limits = RtspFrameLimits::new(2 * 1024 * 1024, 128)
        .unwrap_or_else(|error| panic!("fixture limits must be valid: {error}"));
    RtspSourceConfig::new(endpoint, Some(credentials), Duration::from_secs(5), limits)
        .unwrap_or_else(|error| panic!("fixture source must be valid: {error}"))
}

#[tokio::test]
#[ignore = "requires the accelerated repository-owned RTSP fixture"]
async fn writes_three_probeable_and_decodable_thirty_second_segments() {
    let output_dir = std::env::var_os("RETINA_RECORDING_OUTPUT_DIR").map_or_else(
        || panic!("recording fixture output directory must be provided"),
        PathBuf::from,
    );
    let config = Fmp4SinkConfig::new(
        640,
        360,
        SegmentDuration::new(Duration::from_secs(30))
            .unwrap_or_else(|error| panic!("fixture duration must be valid: {error}")),
        64 * 1024 * 1024,
        10_000,
    )
    .unwrap_or_else(|error| panic!("fixture sink config must be valid: {error}"));
    let sink = Fmp4SegmentSink::new(config);
    let layout = RecordingLayout::new(output_dir)
        .unwrap_or_else(|error| panic!("fixture layout must be valid: {error}"));
    let store = SegmentFileStore::new(layout.clone(), PathBuf::from("ffprobe"));
    let camera_id = CameraId::new("fixture-camera")
        .unwrap_or_else(|error| panic!("fixture camera ID must be valid: {error}"));
    let capacity = QueueCapacity::new(256)
        .unwrap_or_else(|error| panic!("fixture queue capacity must be valid: {error}"));
    let (distributor, subscriptions) = MediaDistributor::new(capacity, capacity);
    drop(subscriptions.live);
    let clock = MediaWallClock::new(
        ClockDriftTolerance::new(Duration::from_secs(3_600))
            .unwrap_or_else(|error| panic!("fixture clock tolerance must be valid: {error}")),
    );
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(4);
    let (_recorder_shutdown_tx, recorder_shutdown_rx) = tokio::sync::watch::channel(false);
    let recorder = run_segment_recorder(
        &camera_id,
        subscriptions.recorder,
        sink,
        &layout,
        &store,
        clock,
        &event_tx,
        recorder_shutdown_rx,
    );
    tokio::pin!(recorder);
    let mut session = connect_h264(&source_config())
        .await
        .unwrap_or_else(|error| panic!("fixture should connect: {error}"));

    let mut paths = Vec::new();
    tokio::time::timeout(Duration::from_secs(60), async {
        while paths.len() < 3 {
            tokio::select! {
                outcome = session.next() => match outcome
                    .unwrap_or_else(|error| panic!("fixture media read failed: {error}")) {
                    RtspReadOutcome::AccessUnit(received) => {
                        assert_ne!(
                            distributor.publish(received.access_unit).recorder,
                            PublishOutcome::Full
                        );
                    }
                    RtspReadOutcome::PacketLoss { lost_packets } => {
                        panic!("TCP recording fixture lost {lost_packets} packets")
                    }
                    RtspReadOutcome::EndOfStream => panic!("fixture ended before three segments"),
                },
                event = event_rx.recv() => match event {
                    Some(RecordingLifecycleEvent::Completed { segment, .. }) => {
                        paths.push(segment.path().to_path_buf());
                    }
                    Some(RecordingLifecycleEvent::Failed { segment, .. }) => {
                        panic!("recording lifecycle failed: {:?}", segment.failure())
                    }
                    None => panic!("recording lifecycle channel closed"),
                },
                result = &mut recorder => panic!("recorder exited early: {result:?}"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("three accelerated segments were not produced in 60 seconds"));

    drop(distributor);
    let recorder_exit = tokio::time::timeout(Duration::from_secs(5), recorder)
        .await
        .unwrap_or_else(|_| panic!("recorder did not drain and stop"))
        .unwrap_or_else(|error| panic!("recorder runtime failed: {error}"));
    assert!(recorder_exit.counters.completed >= 3);

    session
        .shutdown(Duration::from_secs(3))
        .await
        .unwrap_or_else(|error| panic!("fixture teardown should complete: {error}"));

    validate_files(&paths);
}

fn validate_files(paths: &[PathBuf]) {
    for path in paths {
        let probe = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=codec_name,width,height",
                "-of",
                "csv=p=0",
            ])
            .arg(path)
            .output()
            .unwrap_or_else(|error| panic!("ffprobe must execute: {error}"));
        assert!(
            probe.status.success(),
            "ffprobe rejected {}",
            path.display()
        );
        let summary = String::from_utf8(probe.stdout)
            .unwrap_or_else(|error| panic!("ffprobe output must be UTF-8: {error}"));
        assert_eq!(summary.trim(), "h264,640,360");

        let decode = Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(path)
            .args(["-map", "0:v:0", "-f", "null", "-"])
            .status()
            .unwrap_or_else(|error| panic!("ffmpeg must execute: {error}"));
        assert!(decode.success(), "ffmpeg rejected {}", path.display());
    }
}
