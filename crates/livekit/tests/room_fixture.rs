//! Qualifies publication against a real `LiveKit` room.
//!
//! Everything else about publication is provable with a test double. This is
//! not: a `Session` holds a connected `Room`, so the step where media becomes a
//! published track has no seam beneath it, and every test above it has to
//! assume that step works. This fixture removes the assumption by joining the
//! room as a subscriber — through the same viewer-token path the dashboard
//! uses — and looking at what arrives.
//!
//! It asserts pictures rather than a track, and the difference is the point. A
//! track is published before the decoder starts and its source emits black
//! keepalive frames until the first real picture, so a subscriber receiving
//! video proves delivery and not content. The media is colour bars; the
//! assertion is that something lit arrives. Measured here: thirty keepalive
//! frames reach the subscriber before the first real one, so a check that
//! accepted any frame would pass without a decoder in the path at all.
//!
//! Orchestrated by `scripts/phase5/room-fixture.sh`, which owns the server's
//! lifetime. Ignored by default for the same reason every other
//! external-dependency test here is.

use std::{
    num::NonZeroUsize,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

use futures_util::StreamExt;
use livekit::{
    track::RemoteTrack, webrtc::video_stream::native::NativeVideoStream, Room, RoomEvent,
    RoomOptions,
};
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

/// 90 kHz, and the fixture media is 15 fps.
const TICKS_PER_FRAME: i64 = 6_000;

/// NAL unit types that carry a coded picture, which is what ends an access
/// unit in a stream encoded with one slice per picture.
const NON_IDR_SLICE: u8 = 1;
const IDR_SLICE: u8 = 5;

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

/// Splits an Annex-B stream into the NAL units it carries.
fn nal_units(stream: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut index = 0;
    while index + 3 <= stream.len() {
        if stream[index..index + 3] == [0, 0, 1] {
            starts.push(index + 3);
            index += 3;
        } else {
            index += 1;
        }
    }
    starts
        .iter()
        .enumerate()
        .filter_map(|(position, start)| {
            // The next start code ends this unit, less the zero byte a
            // four-byte start code borrows from the end of the previous one.
            let end = starts
                .get(position + 1)
                .map_or(stream.len(), |next| next.saturating_sub(3));
            let end = if end > *start && stream.get(end.wrapping_sub(1)) == Some(&0) {
                end - 1
            } else {
                end
            };
            (end > *start).then(|| &stream[*start..end])
        })
        .collect()
}

/// Groups NAL units into access units, one coded picture each.
///
/// The media is encoded with one slice per picture, so a slice NAL ends the
/// access unit that the parameter sets and SEI before it belong to. Real
/// cameras are not always so tidy, which is why production has a proper
/// assembler and this fixture only has to drive it.
fn access_units(stream: &[u8], start_ticks: i64) -> Vec<Arc<H264AccessUnit>> {
    let mut units = Vec::new();
    let mut pending: Vec<&[u8]> = Vec::new();
    for nal in nal_units(stream) {
        let kind = nal[0] & 0x1f;
        pending.push(nal);
        if kind != NON_IDR_SLICE && kind != IDR_SLICE {
            continue;
        }
        let ticks = start_ticks + i64::try_from(units.len()).unwrap_or(0) * TICKS_PER_FRAME;
        let timestamp = MediaTimestamp::new(ticks, 90_000)
            .unwrap_or_else(|error| panic!("fixture timestamp: {error}"));
        let mut assembler = H264AccessUnitAssembler::new();
        let mut completed = None;
        for (index, bytes) in pending.iter().enumerate() {
            completed = assembler
                .push(
                    timestamp,
                    H264NalUnit::from_ebsp(Arc::from(*bytes))
                        .unwrap_or_else(|error| panic!("fixture NAL: {error}")),
                    index + 1 == pending.len(),
                )
                .unwrap_or_else(|error| panic!("fixture access unit: {error}"));
        }
        units.push(Arc::new(
            completed.unwrap_or_else(|| panic!("fixture access unit must complete")),
        ));
        pending.clear();
    }
    units
}

/// Loads the media the fixture script encoded.
fn fixture_media() -> Vec<u8> {
    let path = std::env::var("PHASE5_LIVEKIT_MEDIA").unwrap_or_else(|_| {
        panic!("PHASE5_LIVEKIT_MEDIA is required; scripts/phase5/room-fixture.sh sets it")
    });
    let stream =
        std::fs::read(&path).unwrap_or_else(|error| panic!("read fixture media {path}: {error}"));
    let units = access_units(&stream, 0);
    assert!(
        units.len() >= 10,
        "the fixture media must carry enough pictures to prove a stream, got {}",
        units.len()
    );
    assert!(
        units.iter().any(|unit| unit.is_keyframe()),
        "the fixture media must start a decodable run"
    );
    stream
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

/// How many frames reach the subscriber before one of them is lit.
///
/// The track arriving is not the end of it. A track is published before the
/// decoder starts, and its source emits black keepalive frames until the first
/// real picture, so a subscriber receiving video proves delivery and not
/// content. Watching the pixels is what separates them: the fixture media is
/// colour bars, and black is what a real picture has to be distinguished from.
///
/// `Err` carries how many frames were inspected before giving up, which is the
/// difference between a stream that never started and one that is all black.
async fn first_lit_frame(track: &RemoteTrack) -> Result<u32, u32> {
    let RemoteTrack::Video(video) = track else {
        panic!("the published track must be video");
    };
    let mut frames = NativeVideoStream::new(video.rtc_track());
    let mut inspected = 0_u32;
    let found = tokio::time::timeout(Duration::from_secs(30), async {
        while let Some(frame) = frames.next().await {
            inspected += 1;
            let i420 = frame.buffer.as_ref().to_i420();
            let (luma, _, _) = i420.data();
            // Any sample well above black. A keepalive frame is uniformly
            // black, so one lit sample is the whole distinction; averaging
            // would make a mostly-dark real picture look like one.
            if luma.iter().any(|sample| *sample > 64) {
                return Some(inspected);
            }
        }
        None
    })
    .await;
    match found {
        Ok(Some(count)) => Ok(count),
        _ => Err(inspected),
    }
}

#[tokio::test]
#[ignore = "orchestrated by scripts/phase5/room-fixture.sh"]
async fn a_subscriber_sees_the_pictures_a_camera_publishes() {
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
    // Taken before the adapter moves into the owner, which is the only chance:
    // `frames_published` counts decoded pictures handed to libwebrtc, and it is
    // the difference between a track existing and a track carrying a camera.
    let publication = Arc::clone(adapter.counters());
    let (owner, health) = LivePublicationOwner::new(config(), adapter, subscriptions.live);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(owner.run(shutdown_rx));

    // The stream is replayed rather than sent once: the owner connects
    // asynchronously, and units published before it is listening are dropped by
    // a bounded queue that prefers recent frames, which is behaviour under test
    // elsewhere. Each pass restarts at a keyframe, so a decoder that starts
    // late still has a decodable run to begin on.
    let media = fixture_media();
    let feeding = tokio::spawn(async move {
        // Paced at the media's own frame rate, and with the clock running
        // forward across passes. Feeding faster than real time overruns a queue
        // that is meant to drop, and rewinding the timestamps on each pass is a
        // discontinuity the owner correctly treats as a camera resynchronising.
        // Both produce decoder errors that belong to the fixture rather than to
        // the thing being qualified.
        let mut offset = 0_i64;
        for _ in 0..8_u32 {
            let units = access_units(&media, offset);
            let span = i64::try_from(units.len()).unwrap_or(0) * TICKS_PER_FRAME;
            for unit in units {
                distributor.publish(unit);
                tokio::time::sleep(Duration::from_millis(66)).await;
            }
            offset += span;
        }
    });

    let subscribed = tokio::time::timeout(Duration::from_secs(30), async {
        while let Some(event) = events.recv().await {
            if let RoomEvent::TrackSubscribed {
                track, participant, ..
            } = event
            {
                return Some((track, participant.identity().to_string()));
            }
        }
        None
    })
    .await;

    let (track, publisher) = subscribed
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

    let lit = first_lit_frame(&track).await;

    shutdown_tx.send_replace(true);
    let exit = task.await.unwrap_or_else(|error| panic!("{error}"));
    feeding.abort();

    let identity = PublicationIdentity::for_camera(NAMESPACE, &camera())
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        publisher,
        identity.participant.as_str(),
        "the track must come from this camera's publisher identity"
    );
    assert_eq!(
        track.name(),
        identity.track.as_str(),
        "and carry the track name the identity defines"
    );
    assert!(
        exit.snapshot.counters.access_units > 0,
        "the publisher must have accepted media, not merely connected"
    );
    let pictures = publication.frames_published.load(Ordering::Relaxed);
    assert!(
        pictures > 0,
        "no decoded picture reached libwebrtc: the track carries keepalive black \
         rather than a camera. Access units accepted: {}, decoder restarts: {}",
        exit.snapshot.counters.access_units,
        publication.decoder_restarts.load(Ordering::Relaxed)
    );
    assert_eq!(
        publication.frames_malformed.load(Ordering::Relaxed),
        0,
        "a picture the decoder produced could not be copied whole, which means \
         the decoder and the pinned geometry disagree"
    );
    let lit_after = lit.unwrap_or_else(|inspected| {
        panic!(
            "no lit frame reached the subscriber within 30s after {inspected} frame(s); \
             the track carries keepalive black. {pictures} pictures were published"
        )
    });
    println!(
        "{pictures} decoded pictures published; the subscriber saw a lit frame after \
         {lit_after} frame(s)"
    );
}
