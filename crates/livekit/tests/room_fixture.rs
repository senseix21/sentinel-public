//! Qualifies publication against a real `LiveKit` room.
//!
//! Everything else about publication is provable with a test double. This is
//! not: a `Session` holds a connected `Room`, so the step where media becomes a
//! published track has no seam beneath it, and every test above it has to
//! assume that step works. This fixture removes the assumption by joining the
//! room as a subscriber — through the same viewer-token path the dashboard
//! uses — and waiting to be told the track arrived.
//!
//! Orchestrated by `scripts/phase5/room-fixture.sh`, which owns the server's
//! lifetime. Ignored by default for the same reason every other
//! external-dependency test here is.

use std::{num::NonZeroUsize, sync::Arc, time::Duration};

use livekit::{Room, RoomEvent, RoomOptions};
use surveillance_common::types::CameraId;
use surveillance_livekit::{
    config::{LiveKitConfig, LiveKitCredentials, PublicationBackoff},
    domain::PublicationIdentity,
    owner::LivePublicationOwner,
    token::DevelopmentTokenService,
    transcode::TranscodePublicationAdapter,
};
use surveillance_media::{
    distribution::{MediaDistributor, QueueCapacity},
    packet::{H264AccessUnit, H264AccessUnitAssembler, H264NalUnit, MediaTimestamp},
};
use tokio::sync::watch;
use url::Url;

/// The namespace both sides must agree on, because it is half of the room name.
const NAMESPACE: &str = "fixture";

/// High Profile 1920x1080 from libx264. Real media: the same bytes
/// `crates/media/src/sps.rs` parses, and what the published track is sized from.
const SPS_1920X1080: &[u8] = &[
    0x67, 0x64, 0x00, 0x28, 0xac, 0xd9, 0x40, 0x78, 0x02, 0x27, 0xe5, 0xc0, 0x44, 0x00, 0x00, 0x03,
    0x00, 0x04, 0x00, 0x00, 0x03, 0x00, 0xf0, 0x3c, 0x60, 0xc6, 0x58,
];
const PPS: &[u8] = &[0x68, 0xeb, 0xe3, 0xcb, 0x22, 0xc0];

fn endpoint() -> String {
    std::env::var("PHASE5_LIVEKIT_URL").unwrap_or_else(|_| "ws://127.0.0.1:7880".to_owned())
}

fn camera() -> CameraId {
    CameraId::new("room-fixture-camera").unwrap_or_else(|error| panic!("{error}"))
}

fn config() -> LiveKitConfig {
    LiveKitConfig {
        decoder_program: "ffmpeg".to_owned(),
        max_publish_height: 0,
        viewer_idle_grace: Duration::from_secs(30),
        endpoint: Url::parse(&endpoint()).unwrap_or_else(|error| panic!("fixture URL: {error}")),
        credentials: LiveKitCredentials::new(
            std::env::var("PHASE5_LIVEKIT_KEY").unwrap_or_else(|_| "devkey".to_owned()),
            std::env::var("PHASE5_LIVEKIT_SECRET")
                .unwrap_or_else(|_| "local-development-secret-not-for-deployment".to_owned()),
        )
        .unwrap_or_else(|error| panic!("{error}")),
        identity: PublicationIdentity::for_camera(NAMESPACE, &camera())
            .unwrap_or_else(|error| panic!("{error}")),
        deployment_namespace: NAMESPACE.to_owned(),
        queue_items: NonZeroUsize::new(8).unwrap_or(NonZeroUsize::MIN),
        max_lag: Duration::from_millis(500),
        max_access_unit_bytes: NonZeroUsize::new(2 * 1024 * 1024).unwrap_or(NonZeroUsize::MIN),
        shutdown_timeout: Duration::from_secs(5),
        backoff: PublicationBackoff::new(
            Duration::from_millis(50),
            Duration::from_millis(200),
            Duration::from_secs(1),
        )
        .unwrap_or_else(|error| panic!("{error}")),
    }
}

fn keyframe(ticks: i64) -> Arc<H264AccessUnit> {
    let mut assembler = H264AccessUnitAssembler::new();
    let timestamp =
        MediaTimestamp::new(ticks, 90_000).unwrap_or_else(|error| panic!("timestamp: {error}"));
    let nals: [&[u8]; 3] = [SPS_1920X1080, PPS, &[0x65, 0x88, 0x84, 0x00]];
    let mut completed = None;
    for (index, bytes) in nals.iter().enumerate() {
        completed = assembler
            .push(
                timestamp,
                H264NalUnit::from_ebsp(Arc::from(*bytes))
                    .unwrap_or_else(|error| panic!("fixture NAL: {error}")),
                index + 1 == nals.len(),
            )
            .unwrap_or_else(|error| panic!("fixture access unit: {error}"));
    }
    Arc::new(completed.unwrap_or_else(|| panic!("fixture access unit must complete")))
}

/// Joins the room the publisher will publish into, as a viewer would.
///
/// The token comes from `DevelopmentTokenService`, which is the service the
/// dashboard calls, so a change that breaks viewer tokens breaks this too
/// rather than being papered over by a token minted for the test.
async fn subscriber() -> (Room, tokio::sync::mpsc::UnboundedReceiver<RoomEvent>) {
    let service =
        DevelopmentTokenService::new(&config(), NAMESPACE.to_owned(), Duration::from_secs(300))
            .unwrap_or_else(|error| panic!("token service: {error}"));
    let access = service
        .issue_for_camera(&camera())
        .unwrap_or_else(|error| panic!("viewer token: {error}"));
    Room::connect(access.endpoint(), access.token(), RoomOptions::default())
        .await
        .unwrap_or_else(|error| panic!("subscriber could not join {}: {error}", access.room()))
}

#[tokio::test]
#[ignore = "orchestrated by scripts/phase5/room-fixture.sh"]
async fn a_subscriber_receives_the_track_a_camera_publishes() {
    // The subscriber joins first. The adapter seeds its viewer count from the
    // room at connect, and publishes nothing while nobody is watching, so a
    // subscriber arriving later would be testing the join event instead of the
    // publication.
    let (_viewer, mut events) = subscriber().await;

    let capacity = QueueCapacity::new(8).unwrap_or_else(|error| panic!("{error}"));
    let (distributor, subscriptions) = MediaDistributor::new(capacity, capacity);
    drop(subscriptions.recorder);
    let adapter = TranscodePublicationAdapter::new(&config(), Duration::from_secs(300))
        .unwrap_or_else(|error| panic!("adapter: {error:?}"));
    let (owner, health) = LivePublicationOwner::new(config(), adapter, subscriptions.live);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(owner.run(shutdown_rx));

    // Keyframes rather than one: the owner connects asynchronously, and a unit
    // published before it is listening is dropped by a bounded queue that
    // prefers recent frames, which is the behaviour under test elsewhere.
    let feeding = tokio::spawn(async move {
        for tick in 0..200 {
            distributor.publish(keyframe(tick * 3_000));
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        distributor
    });

    let subscribed = tokio::time::timeout(Duration::from_secs(30), async {
        while let Some(event) = events.recv().await {
            if let RoomEvent::TrackSubscribed {
                track, participant, ..
            } = event
            {
                return Some((track.name(), participant.identity().to_string()));
            }
        }
        None
    })
    .await;

    shutdown_tx.send_replace(true);
    let exit = task.await.unwrap_or_else(|error| panic!("{error}"));
    feeding.abort();

    let (track_name, publisher) = subscribed
        .unwrap_or_else(|_| {
            panic!(
                "no track reached the subscriber within 30s; publication state {:?}, \
                 last failure {:?}, connection attempts {}",
                health.snapshot().state,
                health.snapshot().last_failure,
                health.snapshot().counters.connection_attempts
            )
        })
        .unwrap_or_else(|| panic!("the room closed before publishing a track"));

    let identity = PublicationIdentity::for_camera(NAMESPACE, &camera())
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        publisher,
        identity.participant.as_str(),
        "the track must come from this camera's publisher identity"
    );
    assert_eq!(
        track_name,
        identity.track.as_str(),
        "and carry the track name the identity defines"
    );
    assert!(
        exit.snapshot.counters.access_units > 0,
        "the publisher must have accepted media, not merely connected"
    );
}
