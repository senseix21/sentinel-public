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
//! Orchestrated by `scripts/phase5/room-fixture.sh`, which encodes the media
//! and runs these serially. The server's lifetime belongs to each test rather
//! than to the script, because qualifying a restart means stopping it at a
//! chosen moment and starting it again, which a wrapper cannot do. Serial
//! because two servers would collide on the RTC ports, not only the signalling
//! one. Ignored by default for the same reason every other external-dependency
//! test here is.

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
    owner::{LivePublicationOwner, PublicationStopReason},
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

/// A `LiveKit` server this fixture starts, stops, and can start again.
struct LiveKitServer {
    child: std::process::Child,
}

impl LiveKitServer {
    fn port() -> u16 {
        Url::parse(&endpoint())
            .ok()
            .and_then(|url| url.port())
            .unwrap_or(7880)
    }

    fn listening() -> bool {
        std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], Self::port())),
            Duration::from_millis(100),
        )
        .is_ok()
    }

    fn start() -> Self {
        let binary =
            std::env::var("LIVEKIT_SERVER_BIN").unwrap_or_else(|_| "livekit-server".to_owned());
        let child = std::process::Command::new(&binary)
            .args([
                "--dev",
                "--bind",
                "0.0.0.0",
                "--config-body",
                &format!("port: {}", Self::port()),
                "--keys",
                &format!("{}: {}", api_key(), api_secret()),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap_or_else(|error| panic!("start {binary}: {error}"));
        let server = Self { child };
        for _ in 0..200 {
            if Self::listening() {
                return server;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!(
            "the LiveKit server did not listen on {} in 10s",
            Self::port()
        );
    }

    /// Stops the server and waits until the port is refused.
    ///
    /// Waiting matters: a test that continued while the socket was still
    /// accepting would be qualifying a server that is merely busy, and would
    /// pass whether or not anything recovers.
    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        for _ in 0..200 {
            if !Self::listening() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("the LiveKit server kept listening after being killed");
    }
}

impl Drop for LiveKitServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn api_key() -> String {
    std::env::var("PHASE5_LIVEKIT_KEY").unwrap_or_else(|_| "devkey".to_owned())
}

fn api_secret() -> String {
    std::env::var("PHASE5_LIVEKIT_SECRET")
        .unwrap_or_else(|_| "local-development-secret-not-for-deployment".to_owned())
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
        credentials: LiveKitCredentials::new(api_key(), api_secret())
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

/// Publishes the fixture media on a loop until the returned task is aborted.
///
/// Paced just ahead of the media's own frame rate, with the clock running
/// forward across passes.
///
/// Three pacing mistakes were made here before this comment existed. Feeding
/// much faster than real time overruns a queue that is meant to drop, and
/// rewinding the timestamps on each pass is a discontinuity the owner correctly
/// treats as a camera resynchronising; both produced decoder errors that
/// belonged to the fixture. The third was subtler: 60 ms of wall time per frame
/// against 66.67 ms of media time is deliberate, because a fixture whose media
/// clock outruns the wall clock asks the owner to sleep until that media
/// arrives — which it does, consuming nothing, exactly as designed. That looks
/// like a stall and is not one. A real camera's media clock does not outrun the
/// wall.
fn feed_continuously(distributor: MediaDistributor<H264AccessUnit>) -> tokio::task::JoinHandle<()> {
    let media = fixture_media();
    tokio::spawn(async move {
        let mut offset = 0_i64;
        loop {
            let units = access_units(&media, offset);
            let span = i64::try_from(units.len()).unwrap_or(0) * TICKS_PER_FRAME;
            for unit in units {
                distributor.publish(unit);
                tokio::time::sleep(Duration::from_millis(60)).await;
            }
            offset += span;
        }
    })
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
    let _server = LiveKitServer::start();

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
    let feeding = feed_continuously(distributor);

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

/// Waits for a lit picture to reach a subscriber of this camera's track.
///
/// Returns the publisher identity it came from, so a caller can tell a track
/// published after a reconnect from one left over before it.
async fn await_lit_picture(
    stage: &str,
    events: &mut tokio::sync::mpsc::UnboundedReceiver<RoomEvent>,
) -> Option<String> {
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
    .await
    .unwrap_or_else(|_| panic!("{stage}: no track reached the subscriber within 30s"));
    // `None` means the room closed under this viewer, which is the viewer's
    // problem and not evidence about the camera. The caller decides.
    let (track, publisher) = subscribed?;
    let lit = first_lit_frame(&track).await.ok()?;
    println!("  {stage}: lit frame after {lit} frame(s)");
    Some(publisher)
}

/// Joins as a viewer and waits for a lit picture, retrying the join.
///
/// A viewer that arrives while the publisher is still re-establishing can have
/// its own room closed under it, or be handed a track that is not yet carrying
/// anything. A browser would reconnect; so does this. The retry is on the
/// viewer's side only — nothing about the camera's recovery is retried, and the
/// caller has already established that pictures resumed before this is called.
async fn lit_picture_with_rejoin(stage: &str, attempts: u32) -> String {
    let mut last = String::new();
    for attempt in 1..=attempts {
        let (viewer, mut events) = subscriber().await;
        if let Ok(Some(publisher)) = tokio::time::timeout(
            Duration::from_secs(10),
            await_lit_picture(stage, &mut events),
        )
        .await
        {
            return publisher;
        }
        last = format!("attempt {attempt} of {attempts} found nothing");
        drop(viewer);
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("{stage}: no lit picture reached a viewer across {attempts} joins ({last})")
}

/// A camera whose `LiveKit` server is killed under it publishes again when the
/// server comes back.
///
/// Four Phase 5 items are qualified here: that a real server stop and restart
/// is recovered from deterministically, that the track is re-published rather
/// than left duplicated or missing, that a startup outage is isolated to live
/// viewing, and that shutdown after all of it leaves nothing behind.
///
/// The outage is a kill, not a graceful stop, and the test waits for the port
/// to be refused before continuing. A server that is merely busy would let this
/// pass without anything recovering.
///
/// The `track_republications` counter is the correct gate: it fires only after
/// `publish_track` completes and the SDK has wired the source to the new
/// encoder. The previous gate (`frames_published > published_before`) fired
/// immediately from grace-period frames and proved nothing — that was the root
/// cause of the 1-in-3 failures recorded in the prior handoff.
#[tokio::test]
#[ignore = "orchestrated by scripts/phase5/room-fixture.sh"]
async fn a_camera_publishes_again_after_its_server_is_killed_and_restarted() {
    let mut server = LiveKitServer::start();
    let (viewer, mut events) = subscriber().await;

    let capacity = QueueCapacity::new(8).unwrap_or_else(|error| panic!("{error}"));
    let (distributor, subscriptions) = MediaDistributor::new(capacity, capacity);
    drop(subscriptions.recorder);
    let adapter = TranscodePublicationAdapter::new(&config(), Duration::from_secs(300))
        .unwrap_or_else(|error| panic!("adapter: {error:?}"));
    let publication = Arc::clone(adapter.counters());
    let (owner, _health) = LivePublicationOwner::new(config(), adapter, subscriptions.live);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(owner.run(shutdown_rx));
    let feeding = feed_continuously(distributor);

    let before = await_lit_picture("before the outage", &mut events)
        .await
        .unwrap_or_else(|| panic!("no lit picture before the outage"));
    let published_before = publication.frames_published.load(Ordering::Relaxed);
    assert!(
        published_before > 0,
        "nothing was published before the outage"
    );

    // The outage. The subscriber's own room dies with the server, which is what
    // a viewer would experience, so it is dropped rather than reused.
    drop(viewer);
    drop(events);
    server.stop();

    // The publication must survive it. A camera that gives up here needs an
    // operator to restart it after a server upgrade that took ten seconds.
    //
    // Which layer recovers is deliberately not asserted. The `LiveKit` SDK
    // reconnects its own room and re-publishes the track when the server
    // returns, so the owner's backoff never runs, and a test demanding
    // `Backoff` here would be demanding a particular implementation rather than
    // the behaviour an operator cares about. What is asserted is that nothing
    // gave up, and that pictures resume.
    for _ in 0..40 {
        assert!(
            !task.is_finished(),
            "the publication stopped during the outage instead of waiting it out"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    // The server comes back, and so must the camera, without being asked.
    let _restarted = LiveKitServer::start();

    // Wait until the SDK has fully re-established the track before joining a
    // viewer. `reconnections` fires on both the resume and the full-restart
    // path, always after SyncState, restart_publisher, and ICE settle — the
    // track is subscribable once this fires. The previous check
    // (`frames_published > published_before`) fired immediately from
    // grace-period frames and was the root cause of the 1-in-3 failures.
    let mut resumed = false;
    for _ in 0..120 {
        if publication.reconnections.load(Ordering::Relaxed) > 0 {
            resumed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(
        resumed,
        "the SDK did not reconnect in the 30 s after the server returned; \
         {published_before} pictures were produced before the outage"
    );

    let after = lit_picture_with_rejoin("after the restart", 5).await;

    shutdown_tx.send_replace(true);
    let exit = task.await.unwrap_or_else(|error| panic!("{error}"));
    feeding.abort();

    let identity = PublicationIdentity::for_camera(NAMESPACE, &camera())
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(before, identity.participant.as_str());
    assert_eq!(
        after,
        identity.participant.as_str(),
        "the camera must republish under its own identity, not a second one"
    );
    assert_eq!(
        exit.reason,
        PublicationStopReason::Requested,
        "the publication must end because this test asked, not because it gave \
         up on a server that came back"
    );
    let published_after = publication.frames_published.load(Ordering::Relaxed);
    assert!(
        published_after > published_before,
        "no new picture was published after the restart: {published_before} \
         before, {published_after} after"
    );
    println!(
        "{published_before} pictures before the outage, {published_after} after \
         the restart"
    );
}
