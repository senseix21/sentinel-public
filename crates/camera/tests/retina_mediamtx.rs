use std::time::Duration;

use surveillance_camera::{
    connection::TerminalCameraFailure,
    rtsp::{
        connect_h264, RtspConnectError, RtspCredentials, RtspEndpoint, RtspFrameLimits,
        RtspReadOutcome, RtspSourceConfig,
    },
    supervisor::{run_h264_camera, JitterSource, ReconnectPolicy, SupervisorStopReason},
};
use surveillance_common::types::CameraId;
use surveillance_media::distribution::{MediaDistributor, QueueCapacity, ReceiveOutcome};

struct MidpointJitter;

impl JitterSource for MidpointJitter {
    fn sample_basis_points(&mut self) -> u16 {
        5_000
    }
}

fn reconnect_policy() -> ReconnectPolicy {
    ReconnectPolicy::new(
        Duration::from_millis(50),
        Duration::from_millis(500),
        2_000,
        Duration::from_secs(2),
        Duration::from_secs(30),
    )
    .unwrap_or_else(|error| panic!("fixture reconnect policy must be valid: {error}"))
}

fn source_config(password: &str) -> RtspSourceConfig {
    let endpoint = RtspEndpoint::parse("rtsp://127.0.0.1:18554/retina-fixture")
        .unwrap_or_else(|error| panic!("fixture endpoint must be valid: {error}"));
    let credentials = RtspCredentials::new("fixture-reader".to_owned(), password.to_owned())
        .unwrap_or_else(|error| panic!("fixture credentials must be valid: {error}"));
    let limits = RtspFrameLimits::new(2 * 1024 * 1024, 128)
        .unwrap_or_else(|error| panic!("fixture limits must be valid: {error}"));
    RtspSourceConfig::new(endpoint, Some(credentials), Duration::from_secs(5), limits)
        .unwrap_or_else(|error| panic!("fixture source config must be valid: {error}"))
}

#[tokio::test]
#[ignore = "requires the repository-owned MediaMTX fixture script"]
async fn authenticated_tcp_session_yields_bounded_h264_access_units() {
    let mut session = connect_h264(&source_config("fixture-reader-pass"))
        .await
        .unwrap_or_else(|error| panic!("fixture should connect: {error}"));

    let parameters = session
        .video_parameters()
        .unwrap_or_else(|| panic!("fixture should expose video parameters"));
    assert_eq!((parameters.width, parameters.height), (640, 360));
    assert!(parameters.rfc6381_codec.starts_with("avc1."));

    let mut access_units = 0_usize;
    let mut saw_keyframe = false;
    let mut previous_ticks = None;
    while access_units < 15 {
        let outcome = tokio::time::timeout(Duration::from_secs(2), session.next())
            .await
            .unwrap_or_else(|_| panic!("fixture media read timed out"))
            .unwrap_or_else(|error| panic!("fixture media read failed: {error}"));
        match outcome {
            RtspReadOutcome::AccessUnit(received) => {
                let timestamp = received.access_unit.timestamp();
                assert_eq!(timestamp.timebase_hz(), 90_000);
                if let Some(previous_ticks) = previous_ticks {
                    assert!(timestamp.ticks() > previous_ticks);
                }
                previous_ticks = Some(timestamp.ticks());
                saw_keyframe |= received.access_unit.is_keyframe();
                access_units += 1;
            }
            RtspReadOutcome::PacketLoss { lost_packets } => {
                panic!("TCP fixture reported {lost_packets} lost packets")
            }
            RtspReadOutcome::EndOfStream => panic!("fixture ended before 15 access units"),
        }
    }
    assert!(saw_keyframe, "fixture did not yield a keyframe");

    session
        .shutdown(Duration::from_secs(3))
        .await
        .unwrap_or_else(|error| panic!("fixture teardown should complete: {error}"));
}

#[tokio::test]
#[ignore = "requires the repository-owned MediaMTX fixture script"]
async fn rejected_credentials_are_terminal() {
    let error = connect_h264(&source_config("wrong-password"))
        .await
        .err()
        .unwrap_or_else(|| panic!("invalid credentials must be rejected"));
    assert!(matches!(
        error,
        RtspConnectError::Terminal {
            failure: TerminalCameraFailure::AuthenticationRejected,
            ..
        }
    ));
}

#[tokio::test]
#[ignore = "requires the repository-owned MediaMTX fixture script"]
async fn supervisor_owns_publication_and_graceful_shutdown() {
    let capacity = QueueCapacity::new(32)
        .unwrap_or_else(|error| panic!("fixture capacity must be valid: {error}"));
    let (distributor, mut subscriptions) = MediaDistributor::new(capacity, capacity);
    let camera_id = CameraId::new("fixture-camera")
        .unwrap_or_else(|error| panic!("fixture camera ID must be valid: {error}"));
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let mut jitter = MidpointJitter;
    let source = source_config("fixture-reader-pass");
    let supervisor = run_h264_camera(
        camera_id,
        &source,
        distributor,
        reconnect_policy(),
        &mut jitter,
        shutdown_rx,
    );
    tokio::pin!(supervisor);

    let item = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::select! {
            item = subscriptions.recorder.receive() => item,
            exit = &mut supervisor => panic!("supervisor exited before publication: {:?}", exit.reason),
        }
    })
    .await
    .unwrap_or_else(|_| panic!("supervisor did not publish within five seconds"));
    assert!(matches!(item, ReceiveOutcome::Item(_)));
    shutdown_tx
        .send(true)
        .unwrap_or_else(|error| panic!("shutdown receiver must remain owned: {error}"));
    let exit = tokio::time::timeout(Duration::from_secs(5), supervisor)
        .await
        .unwrap_or_else(|_| panic!("supervisor did not stop within five seconds"));
    assert_eq!(exit.reason, SupervisorStopReason::Requested);
    assert!(exit.counters.access_units >= 1);
    assert_eq!(exit.snapshot.counters().clean_stops, 1);
}

#[tokio::test]
#[ignore = "requires the repository-owned MediaMTX fixture script"]
async fn recorder_saturation_stops_camera_visibly() {
    let capacity = QueueCapacity::new(1)
        .unwrap_or_else(|error| panic!("fixture capacity must be valid: {error}"));
    let (distributor, _subscriptions) = MediaDistributor::new(capacity, capacity);
    let camera_id = CameraId::new("saturated-camera")
        .unwrap_or_else(|error| panic!("fixture camera ID must be valid: {error}"));
    let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let mut jitter = MidpointJitter;
    let source = source_config("fixture-reader-pass");
    let exit = tokio::time::timeout(
        Duration::from_secs(5),
        run_h264_camera(
            camera_id,
            &source,
            distributor,
            reconnect_policy(),
            &mut jitter,
            shutdown_rx,
        ),
    )
    .await
    .unwrap_or_else(|_| panic!("saturated supervisor did not stop visibly"));
    assert_eq!(exit.reason, SupervisorStopReason::RecorderBackpressure);
    assert_eq!(exit.distributor.recorder_metrics().rejected_full, 1);
}

#[tokio::test]
#[ignore = "requires the repository-owned MediaMTX recovery controller"]
async fn supervisor_recovers_after_publisher_restart_with_fresh_epoch() {
    let control_dir = std::env::var_os("RETINA_FIXTURE_CONTROL_DIR").map_or_else(
        || panic!("fixture control directory must be provided"),
        std::path::PathBuf::from,
    );
    let capacity = QueueCapacity::new(256)
        .unwrap_or_else(|error| panic!("fixture capacity must be valid: {error}"));
    let (distributor, mut subscriptions) = MediaDistributor::new(capacity, capacity);
    let camera_id = CameraId::new("recovery-camera")
        .unwrap_or_else(|error| panic!("fixture camera ID must be valid: {error}"));
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let mut jitter = MidpointJitter;
    let source = source_config("fixture-reader-pass");
    let supervisor = run_h264_camera(
        camera_id,
        &source,
        distributor,
        reconnect_policy(),
        &mut jitter,
        shutdown_rx,
    );
    tokio::pin!(supervisor);

    for _ in 0..3 {
        tokio::select! {
            item = subscriptions.recorder.receive() => assert!(matches!(item, ReceiveOutcome::Item(_))),
            exit = &mut supervisor => panic!("supervisor exited before outage: {:?}", exit.reason),
        }
    }
    std::fs::write(control_dir.join("ready-for-outage"), b"ready")
        .unwrap_or_else(|error| panic!("fixture outage marker must be written: {error}"));

    tokio::time::timeout(Duration::from_secs(12), async {
        loop {
            tokio::select! {
                item = subscriptions.recorder.receive() => {
                    assert!(!matches!(item, ReceiveOutcome::Closed));
                    if control_dir.join("publisher-restarted").is_file() {
                        break;
                    }
                }
                exit = &mut supervisor => panic!("supervisor exited during outage: {:?}", exit.reason),
                () = tokio::time::sleep(Duration::from_millis(25)) => {
                    if control_dir.join("publisher-restarted").is_file() {
                        break;
                    }
                }
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("fixture publisher did not restart"));

    for _ in 0..10 {
        tokio::select! {
            item = subscriptions.recorder.receive() => assert!(matches!(item, ReceiveOutcome::Item(_))),
            exit = &mut supervisor => panic!("supervisor exited before recovery: {:?}", exit.reason),
        }
    }
    shutdown_tx
        .send(true)
        .unwrap_or_else(|error| panic!("shutdown receiver must remain owned: {error}"));
    let exit = tokio::time::timeout(Duration::from_secs(5), supervisor)
        .await
        .unwrap_or_else(|_| panic!("recovered supervisor did not stop"));
    assert_eq!(exit.reason, SupervisorStopReason::Requested);
    assert!(exit.snapshot.counters().connected >= 2);
    assert!(exit.snapshot.counters().reconnects >= 1);
}
