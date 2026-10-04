//! Viewer-gated decode-and-re-encode publication into a `LiveKit` room.
//!
//! Encoded pass-through does not run on any platform `LiveKit` ships — see
//! `docs/adr/012-viewer-gated-transcode-publication.md` and the corrected
//! Decision 5 of ADR 011 — so the live path decodes the camera's H.264 into
//! raw pictures and hands them to libwebrtc, whose *encoders* are correct.
//!
//! Two properties are load-bearing and easy to lose in a later edit:
//!
//! * **Nothing decodes while nobody is watching.** A transcoding publisher for
//!   every camera all of the time would spend the machine's CPU on pictures no
//!   subscriber receives, and detection and recording sit above live viewing in
//!   the subsystem ordering. The decoder starts on the first remote participant
//!   and stops after the last participant's configured, bounded idle grace
//!   (ADR 012 Decision 2).
//! * **This is an adapter, not a second pipeline.** It implements the same
//!   [`PublicationAdapter`] contract as `native.rs`, so the owner, health model,
//!   failure categories, backoff, and metrics in `owner.rs` are untouched and
//!   the camera still holds exactly one upstream connection.

use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use livekit::{
    options::{TrackPublishOptions, VideoCodec},
    publication::LocalTrackPublication,
    track::{LocalTrack, LocalVideoTrack, TrackSource},
    webrtc::{
        video_frame::{I420Buffer, VideoFrame, VideoRotation},
        video_source::{native::NativeVideoSource, RtcVideoSource, VideoResolution},
    },
    Room, RoomEvent, RoomOptions,
};
use surveillance_media::sps::CodedDimensions;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{
    adapter::{
        AdapterError, AdapterErrorKind, AdapterFuture, AdapterObservation, EncodedAccessUnit,
        PublicationAdapter, SendOutcome, TimestampAuthority,
    },
    config::LiveKitConfig,
    decode::{DecodeCounters, DecodeError, DecodedFrameSink, FrameSize, H264ToI420},
    domain::PublicationIdentity,
    native::classify,
    token::{PublisherTokenIssuer, TokenIssueError},
};

/// Counters this adapter keeps for its own overload and lifecycle behaviour.
///
/// Separate from [`DecodeCounters`] because those belong to one decode session
/// and these outlive every restart of it.
#[derive(Debug, Default)]
pub struct TranscodeCounters {
    /// Decoded pictures handed to libwebrtc.
    pub frames_published: AtomicU64,
    /// Access units this adapter did not decode: no viewer, no decodable run
    /// yet, or a poisoned decode session awaiting the next keyframe.
    pub access_units_dropped: AtomicU64,
    /// Decode sessions restarted after an overload or a failed pipe.
    pub decoder_restarts: AtomicU64,
    /// Frames dropped between the decoder and libwebrtc because the picture
    /// could not be copied whole. Nonzero here means the decoder and the
    /// pinned geometry disagree, which is a defect rather than backpressure.
    pub frames_malformed: AtomicU64,
}

/// Publishes a camera into a `LiveKit` room by decoding and re-encoding it.
pub struct TranscodePublicationAdapter {
    identity: PublicationIdentity,
    tokens: PublisherTokenIssuer,
    endpoint: String,
    decoder_program: String,
    max_publish_height: u32,
    viewer_idle_grace: Duration,
    decode_counters: Arc<DecodeCounters>,
    counters: Arc<TranscodeCounters>,
    session: Option<Session>,
}

/// One connected room, its event stream, and the track while a viewer is here.
struct Session {
    room: Room,
    events: UnboundedReceiver<RoomEvent>,
    viewers: usize,
    track: Option<PublishedTrack>,
    idle_deadline: Option<Instant>,
}

/// A published track, the source feeding it, and the decode session behind it.
///
/// `decoder` is `None` between a poisoned session and the next decodable run.
/// The track stays published across that gap: a viewer whose picture freezes
/// for one keyframe interval is a better outcome than one whose track
/// disappears and returns.
struct PublishedTrack {
    publication: LocalTrackPublication,
    source: NativeVideoSource,
    size: FrameSize,
    decoder: Option<H264ToI420>,
}

impl TranscodePublicationAdapter {
    /// Lifetime of a publisher token.
    ///
    /// Same reasoning as the native adapter: minted and consumed inside this
    /// process within milliseconds, and a reconnect mints a fresh one.
    pub const DEFAULT_TOKEN_TTL: Duration = Duration::from_secs(60);

    /// Builds an adapter for one camera's publication identity.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError`] with [`AdapterErrorKind::Authentication`] when
    /// the configured credentials cannot mint a publisher token at all, and
    /// [`AdapterErrorKind::Unavailable`] when the requested token lifetime is
    /// outside the permitted range.
    pub fn new(config: &LiveKitConfig, token_ttl: Duration) -> Result<Self, AdapterError> {
        let tokens = PublisherTokenIssuer::new(config, token_ttl).map_err(|error| {
            AdapterError::new(match error {
                TokenIssueError::InvalidLifetime => AdapterErrorKind::Unavailable,
                _ => AdapterErrorKind::Authentication,
            })
        })?;
        Ok(Self {
            identity: config.identity.clone(),
            tokens,
            endpoint: config.endpoint.as_str().to_owned(),
            decoder_program: config.decoder_program.clone(),
            max_publish_height: config.max_publish_height,
            viewer_idle_grace: config.viewer_idle_grace,
            decode_counters: Arc::new(DecodeCounters::default()),
            counters: Arc::new(TranscodeCounters::default()),
            session: None,
        })
    }

    /// Remote participants currently in the room, as last observed.
    ///
    /// This is the gate on decoding, and it is an approximation: a participant
    /// who joined without subscribing counts as a viewer (ADR 012 Decision 2).
    /// It errs towards decoding, which costs CPU, rather than towards a black
    /// tile, which costs the operator the thing they opened the page for.
    #[must_use]
    pub fn viewers(&self) -> usize {
        self.session.as_ref().map_or(0, |session| session.viewers)
    }

    /// Whether a decode session is running right now.
    #[must_use]
    pub fn decoding(&self) -> bool {
        self.session
            .as_ref()
            .and_then(|session| session.track.as_ref())
            .is_some_and(|track| track.decoder.is_some())
    }

    /// Pictures the decoder has produced since this adapter was built.
    #[must_use]
    pub fn frames_decoded(&self) -> u64 {
        self.decode_counters.frames_decoded.load(Ordering::Relaxed)
    }

    /// Counters for the publication path, shared with the decode sink.
    #[must_use]
    pub fn counters(&self) -> &Arc<TranscodeCounters> {
        &self.counters
    }
}

impl Session {
    /// Room events drained per access unit.
    ///
    /// The receiver `Room::connect` hands back is unbounded and belongs to
    /// `livekit`, not to this crate, so the only thing this code controls is
    /// how promptly it is emptied. A publish-only participant with
    /// `auto_subscribe` off sees participant and track lifecycle events and
    /// nothing per-frame, so this ceiling exists to stop a pathological burst
    /// from starving media handling, not because it is expected to bind.
    const MAX_EVENTS_PER_ACCESS_UNIT: usize = 256;

    /// Drains pending room events and re-reads the viewer count if one moved it.
    ///
    /// The count is re-read from the room rather than incremented and
    /// decremented, so a missed or duplicated event cannot leave a permanent
    /// drift that either burns CPU forever or blacks out every viewer.
    fn refresh_viewers(&mut self) {
        let mut changed = false;
        for _ in 0..Self::MAX_EVENTS_PER_ACCESS_UNIT {
            match self.events.try_recv() {
                Ok(
                    RoomEvent::ParticipantConnected(_)
                    | RoomEvent::ParticipantDisconnected(_)
                    | RoomEvent::Disconnected { .. }
                    | RoomEvent::Reconnected,
                ) => changed = true,
                Ok(_) => {}
                Err(_) => break,
            }
        }
        if changed {
            self.viewers = self.room.remote_participants().len();
        }
    }

    /// Creates and publishes the track at a geometry the stream itself declared.
    async fn publish(
        &mut self,
        identity: &PublicationIdentity,
        size: FrameSize,
    ) -> Result<(), AdapterError> {
        let resolution = VideoResolution {
            width: size.width(),
            height: size.height(),
        };
        // The raw constructor, not `new_encoded`: this source carries decoded
        // pictures and wants libwebrtc's real encoder. It also emits black
        // keepalive frames at 10fps until the first real capture, which is why
        // a subscriber joining before the first decoded picture sees black
        // rather than the undefined green plane an unfed decoder produced.
        // That keepalive task lives inside `libwebrtc` and stops itself on the
        // first captured frame, which the decode session delivers immediately.
        let source = NativeVideoSource::new(resolution, false);
        let track = LocalVideoTrack::create_video_track(
            identity.track.as_str(),
            RtcVideoSource::Native(source.clone()),
        );
        let publication = self
            .room
            .local_participant()
            .publish_track(
                LocalTrack::Video(track),
                TrackPublishOptions {
                    source: TrackSource::Camera,
                    video_codec: VideoCodec::H264,
                    // No `video_encoder` override. Choosing the pass-through
                    // backend is what breaks; a real encoder is the point here.
                    //
                    // Simulcast is off deliberately. Each additional layer is
                    // another encode of the same picture, multiplying the cost
                    // this adapter already gates behind viewer presence, and a
                    // surveillance operator watching a wall of tiles gains
                    // nothing from layers nobody selects.
                    simulcast: false,
                    ..TrackPublishOptions::default()
                },
            )
            .await
            .map_err(|error| AdapterError::new(classify(&error)))?;
        self.track = Some(PublishedTrack {
            publication,
            source,
            size,
            decoder: None,
        });
        Ok(())
    }

    /// Starts a decode session feeding the published track's source.
    fn start_decoder(
        &mut self,
        program: &str,
        decode_counters: &Arc<DecodeCounters>,
        counters: &Arc<TranscodeCounters>,
    ) -> Result<(), AdapterError> {
        let Some(track) = self.track.as_mut() else {
            return Err(AdapterError::new(AdapterErrorKind::Connection));
        };
        let sink = SourceSink {
            source: track.source.clone(),
            counters: Arc::clone(counters),
        };
        let decoder = H264ToI420::spawn(program, track.size, sink, Arc::clone(decode_counters))
            .map_err(|error| AdapterError::new(start_failure_kind(error)))?;
        track.decoder = Some(decoder);
        Ok(())
    }

    /// Stops decoding and withdraws the track.
    ///
    /// Ordered decoder-first so no frame can arrive for a track that is being
    /// withdrawn. An unpublish failure is not surfaced: the caller reached here
    /// because nobody is watching, and turning that into a publication fault
    /// would restart a pipeline to fix a room that already has no audience.
    async fn stop_publishing(&mut self) {
        let Some(track) = self.track.take() else {
            return;
        };
        if let Some(decoder) = track.decoder {
            decoder.shutdown().await;
        }
        let _ = self
            .room
            .local_participant()
            .unpublish_track(&track.publication.sid())
            .await;
    }
}

impl PublishedTrack {
    /// Ends the current decode session, leaving the track published.
    async fn stop_decoder(&mut self) {
        if let Some(decoder) = self.decoder.take() {
            decoder.shutdown().await;
        }
    }
}

impl PublicationAdapter for TranscodePublicationAdapter {
    fn observe(&self) -> AdapterObservation {
        AdapterObservation {
            // `try_from` rather than `as`: a saturating conversion cannot
            // silently report a small viewer count for a large one, and a
            // participant list longer than `u64::MAX` is not reachable.
            viewers: u64::try_from(self.viewers()).unwrap_or(u64::MAX),
            media_produced: self.counters.frames_published.load(Ordering::Relaxed),
            restarts: self.counters.decoder_restarts.load(Ordering::Relaxed),
        }
    }

    fn timestamp_authority(&self) -> TimestampAuthority {
        // Not `Media`, and this is not a formality. A decoder emits pictures in
        // presentation order from access units in decode order, so the mapping
        // from an access unit's capture timestamp to the frame that leaves this
        // adapter is not one-to-one and cannot be recovered here. Glass-to-glass
        // latency is therefore not qualifiable through this path (ADR 012
        // Decision 3), and declaring `Media` would let the owner claim a
        // measurement it does not have.
        TimestampAuthority::AdapterClock
    }

    fn connect(&mut self) -> AdapterFuture<'_, ()> {
        Box::pin(async move {
            let access = self
                .tokens
                .issue(&self.identity)
                .map_err(|_| AdapterError::new(AdapterErrorKind::Authentication))?;
            // `RoomOptions` is `#[non_exhaustive]`, so it is built by mutating
            // the default rather than by struct expression.
            let mut options = RoomOptions::default();
            // This participant publishes and never consumes; a subscription
            // would be inbound media nobody reads.
            options.auto_subscribe = false;
            let (room, events) = Room::connect(&self.endpoint, access.token(), options)
                .await
                .map_err(|error| AdapterError::new(classify(&error)))?;
            // Seeded from the room rather than from zero: a camera restarted
            // while an operator has the page open must start decoding on its
            // first keyframe, not wait for the next join event that will never
            // come.
            let viewers = room.remote_participants().len();
            self.session = Some(Session {
                room,
                events,
                viewers,
                track: None,
                idle_deadline: None,
            });
            Ok(())
        })
    }

    fn send_access_unit<'a>(
        &'a mut self,
        access_unit: EncodedAccessUnit<'a>,
    ) -> AdapterFuture<'a, SendOutcome> {
        Box::pin(async move {
            // Destructured so the session borrow does not conflict with the
            // configuration and counters read alongside it.
            let Self {
                identity,
                decoder_program,
                max_publish_height,
                viewer_idle_grace,
                decode_counters,
                counters,
                session,
                ..
            } = self;
            let Some(session) = session.as_mut() else {
                return Err(AdapterError::new(AdapterErrorKind::Connection));
            };
            session.refresh_viewers();

            if should_stop_publishing(
                session.viewers,
                Instant::now(),
                &mut session.idle_deadline,
                *viewer_idle_grace,
            ) {
                session.stop_publishing().await;
                counters
                    .access_units_dropped
                    .fetch_add(1, Ordering::Relaxed);
                return Ok(SendOutcome::Dropped);
            }

            if session.viewers == 0 {
                // Keep feeding the already-running bounded session during the
                // grace. That preserves a current decoder and track for a
                // quick route change without creating another queue or camera
                // connection.
                let Some(decoder) = session
                    .track
                    .as_mut()
                    .and_then(|track| track.decoder.as_mut())
                else {
                    counters
                        .access_units_dropped
                        .fetch_add(1, Ordering::Relaxed);
                    return Ok(SendOutcome::Dropped);
                };
                let outcome = decoder.feed(access_unit.annex_b).await;
                if outcome.is_ok() {
                    return Ok(SendOutcome::Published);
                }
                if let Some(track) = session.track.as_mut() {
                    track.stop_decoder().await;
                }
                counters.decoder_restarts.fetch_add(1, Ordering::Relaxed);
                counters
                    .access_units_dropped
                    .fetch_add(1, Ordering::Relaxed);
                return Ok(SendOutcome::Dropped);
            }

            let running = match session.track.as_ref() {
                Some(track) => track.decoder.is_some(),
                None => false,
            };
            if !running {
                if !access_unit.starts_decodable_run {
                    // A decoder started mid-GOP produces nothing until the next
                    // keyframe anyway, and a track sized from a guess would be
                    // wrong for the whole session. Waiting costs one keyframe
                    // interval.
                    counters
                        .access_units_dropped
                        .fetch_add(1, Ordering::Relaxed);
                    return Ok(SendOutcome::Dropped);
                }
                if session.track.is_none() {
                    let size = publish_size(access_unit.annex_b, *max_publish_height)?;
                    session.publish(identity, size).await?;
                }
                session.start_decoder(decoder_program, decode_counters, counters)?;
            }

            let Some(decoder) = session
                .track
                .as_mut()
                .and_then(|track| track.decoder.as_mut())
            else {
                return Err(AdapterError::new(AdapterErrorKind::Connection));
            };
            match decoder.feed(access_unit.annex_b).await {
                // The access unit was accepted by the decoder. Whether the
                // picture it becomes reaches a subscriber is libwebrtc's
                // decision, taken on its own encoder queue; this is the last
                // point at which this code knows anything.
                Ok(()) => Ok(SendOutcome::Published),
                Err(DecodeError::Unavailable) => {
                    Err(AdapterError::new(AdapterErrorKind::Unavailable))
                }
                Err(error) => {
                    // A timed-out or broken write can leave a truncated NAL in
                    // the pipe, so the session is discarded rather than trusted
                    // to resynchronise. The track stays published and the next
                    // decodable run starts a fresh decoder: a viewer sees a
                    // freeze, not a disappearing track, and the camera never
                    // reconnects over a transcoding hiccup.
                    //
                    // Logged with the kind because
                    // `surveillance_livekit_adapter_restarts_total` says a
                    // restart happened and nothing else does. `Overloaded` is a
                    // decoder that could not keep up; `Failed` is a decoder that
                    // died. The two need different responses and the counter
                    // cannot tell them apart.
                    tracing::warn!(
                        room = %identity.room,
                        cause = %error,
                        "restarting the live decoder at the next keyframe"
                    );
                    if let Some(track) = session.track.as_mut() {
                        track.stop_decoder().await;
                    }
                    counters.decoder_restarts.fetch_add(1, Ordering::Relaxed);
                    counters
                        .access_units_dropped
                        .fetch_add(1, Ordering::Relaxed);
                    Ok(SendOutcome::Dropped)
                }
            }
        })
    }

    fn disconnect(&mut self) -> AdapterFuture<'_, ()> {
        Box::pin(async move {
            if let Some(mut session) = self.session.take() {
                session.stop_publishing().await;
                // Dropping the room closes it; awaiting the explicit close lets
                // the server see the participant leave rather than time it out.
                let _ = session.room.close().await;
            }
            Ok(())
        })
    }
}

/// How a failed decoder start is reported to the publication loop.
///
/// A missing or unrunnable decoder binary will still be missing next time, so
/// it stops the publication once instead of entering a reconnect loop over a
/// deployment fault. Everything else is a fault of the moment and is retried.
///
/// Exhaustive rather than a wildcard: the difference between these two arms is
/// a camera that comes back on its own and one that does not, so a variant
/// added later has to be placed here deliberately instead of inheriting
/// whichever answer happened to be the default. Named rather than inline so the
/// recovery test can produce an error through the real decoder and send it
/// through this exact mapping, instead of asserting against a copy of it.
pub(crate) const fn start_failure_kind(error: DecodeError) -> AdapterErrorKind {
    match error {
        DecodeError::Unavailable => AdapterErrorKind::Unavailable,
        DecodeError::StartFailed
        | DecodeError::Geometry
        | DecodeError::Failed
        | DecodeError::Overloaded => AdapterErrorKind::Transport,
    }
}

fn should_stop_publishing(
    viewers: usize,
    now: Instant,
    idle_deadline: &mut Option<Instant>,
    grace: Duration,
) -> bool {
    if viewers > 0 {
        *idle_deadline = None;
        return false;
    }
    let deadline = idle_deadline.get_or_insert_with(|| now + grace);
    now >= *deadline
}

/// The geometry to publish at: the stream's own size, under the configured cap.
///
/// Read from the sequence parameter set rather than from configuration, because
/// the camera is the only authority on what it is sending, and then capped —
/// which turns a mid-session resolution change into a scaling event instead of
/// a desynchronised byte stream (ADR 012 Decision 4).
fn publish_size(annex_b: &[u8], max_height: u32) -> Result<FrameSize, AdapterError> {
    // The caller only reaches here on an access unit the media plane classified
    // as starting a decodable run, which by definition carries its parameter
    // sets. A failure means the bytes and that classification disagree, which
    // is not something to paper over with a configured default.
    let dimensions = CodedDimensions::from_annex_b(annex_b)
        .map_err(|_| AdapterError::new(AdapterErrorKind::Transport))?;
    FrameSize::new(dimensions.width(), dimensions.height())
        .and_then(|size| size.capped_to_height(max_height))
        .map_err(|_| AdapterError::new(AdapterErrorKind::Transport))
}

/// Copies decoded pictures into libwebrtc's frame source.
struct SourceSink {
    source: NativeVideoSource,
    counters: Arc<TranscodeCounters>,
}

/// Splits a tightly packed planar I420 frame into its three planes.
///
/// Returns `None` when the buffer is not exactly one picture of this geometry,
/// which is the only signal available here that the decoder and the pinned size
/// have diverged.
fn split_planes(yuv420p: &[u8], size: FrameSize) -> Option<(&[u8], &[u8], &[u8])> {
    let luma = size.width() as usize * size.height() as usize;
    let chroma = (size.width() as usize / 2) * (size.height() as usize / 2);
    Some((
        yuv420p.get(..luma)?,
        yuv420p.get(luma..luma + chroma)?,
        yuv420p.get(luma + chroma..luma + chroma + chroma)?,
    ))
}

impl DecodedFrameSink for SourceSink {
    fn accept(&mut self, yuv420p: &[u8], size: FrameSize) {
        let width = size.width() as usize;
        let height = size.height() as usize;
        let chroma_width = width / 2;
        let chroma_height = height / 2;
        let Some((source_y, source_u, source_v)) = split_planes(yuv420p, size) else {
            self.counters
                .frames_malformed
                .fetch_add(1, Ordering::Relaxed);
            return;
        };

        let mut buffer = I420Buffer::new(size.width(), size.height());
        let (stride_y, stride_u, stride_v) = buffer.strides();
        let (destination_y, destination_u, destination_v) = buffer.data_mut();
        // `I420Buffer::new` leaves its planes uninitialised. A partially copied
        // frame would publish recycled heap memory to every subscriber, so a
        // picture that cannot be copied whole is dropped rather than sent.
        let complete = copy_plane(source_y, width, height, destination_y, stride_y as usize)
            && copy_plane(
                source_u,
                chroma_width,
                chroma_height,
                destination_u,
                stride_u as usize,
            )
            && copy_plane(
                source_v,
                chroma_width,
                chroma_height,
                destination_v,
                stride_v as usize,
            );
        if !complete {
            self.counters
                .frames_malformed
                .fetch_add(1, Ordering::Relaxed);
            return;
        }

        self.counters
            .frames_published
            .fetch_add(1, Ordering::Relaxed);
        // The timestamp stays zero, which makes the source stamp arrival. That
        // is precisely what `TimestampAuthority::AdapterClock` declares, and
        // inventing a media-derived value here would be a fabricated timestamp:
        // the decoder reorders, and this code cannot undo that.
        self.source
            .capture_frame(&VideoFrame::new(VideoRotation::VideoRotation0, buffer));
    }
}

/// Copies one tightly packed plane into a strided destination.
///
/// Returns `false` without writing further if either side is short, so a
/// geometry disagreement drops a frame instead of panicking on a live path.
fn copy_plane(
    source: &[u8],
    width: usize,
    rows: usize,
    destination: &mut [u8],
    stride: usize,
) -> bool {
    for row in 0..rows {
        let (Some(from), Some(to)) = (
            source.get(row * width..row * width + width),
            destination.get_mut(row * stride..row * stride + width),
        ) else {
            return false;
        };
        to.copy_from_slice(from);
    }
    true
}

#[cfg(test)]
mod tests {
    use surveillance_common::types::CameraId;

    use super::*;
    use crate::config::LiveKitSettings;

    /// High Profile 1920x1080 from libx264, prefixed with a four-byte start
    /// code. Real media: the same bytes `crates/media/src/sps.rs` parses.
    const KEYFRAME_1920X1080: &[u8] = &[
        0x00, 0x00, 0x00, 0x01, 0x67, 0x64, 0x00, 0x28, 0xac, 0xd9, 0x40, 0x78, 0x02, 0x27, 0xe5,
        0xc0, 0x44, 0x00, 0x00, 0x03, 0x00, 0x04, 0x00, 0x00, 0x03, 0x00, 0xf0, 0x3c, 0x60, 0xc6,
        0x58,
    ];

    fn config(max_publish_height: u32) -> LiveKitConfig {
        let camera = CameraId::new("camera-01").unwrap_or_else(|error| panic!("{error}"));
        LiveKitConfig::validate(
            &camera,
            LiveKitSettings {
                decoder_program: "ffmpeg".to_owned(),
                max_publish_height,
                viewer_idle_grace: Duration::from_secs(5),
                endpoint: "ws://127.0.0.1:7880".to_owned(),
                api_key: "devkey".to_owned(),
                api_secret: "development-secret".to_owned(),
                deployment_namespace: "site-a".to_owned(),
                queue_items: 4,
                max_lag: Duration::from_millis(500),
                max_access_unit_bytes: 2 * 1024 * 1024,
                shutdown_timeout: Duration::from_secs(2),
                reconnect_initial: Duration::from_millis(250),
                reconnect_maximum: Duration::from_secs(30),
                reconnect_stable_reset: Duration::from_secs(30),
            },
        )
        .unwrap_or_else(|error| panic!("{error}"))
    }

    fn adapter() -> TranscodePublicationAdapter {
        TranscodePublicationAdapter::new(&config(0), Duration::from_secs(300))
            .unwrap_or_else(|error| panic!("{error}"))
    }

    #[test]
    fn the_publisher_cannot_qualify_latency_and_says_so() {
        // A transcoding path reorders, so claiming `Media` here would let the
        // owner report a glass-to-glass figure it has no basis for.
        assert_eq!(
            adapter().timestamp_authority(),
            TimestampAuthority::AdapterClock
        );
    }

    #[test]
    fn an_unconnected_adapter_has_no_viewers_and_decodes_nothing() {
        let adapter = adapter();
        assert_eq!(adapter.viewers(), 0);
        assert!(!adapter.decoding());
        assert_eq!(adapter.frames_decoded(), 0);
    }

    #[test]
    fn idle_grace_is_bounded_and_a_returning_viewer_cancels_teardown() {
        let start = Instant::now();
        let grace = Duration::from_secs(5);
        let mut deadline = None;

        assert!(!should_stop_publishing(0, start, &mut deadline, grace));
        assert!(!should_stop_publishing(
            0,
            start + Duration::from_secs(4),
            &mut deadline,
            grace
        ));
        assert!(!should_stop_publishing(
            1,
            start + Duration::from_secs(4),
            &mut deadline,
            grace
        ));
        assert_eq!(deadline, None);
        assert!(!should_stop_publishing(
            0,
            start + Duration::from_secs(10),
            &mut deadline,
            grace
        ));
        assert!(should_stop_publishing(
            0,
            start + Duration::from_secs(15),
            &mut deadline,
            grace
        ));
    }

    #[test]
    fn zero_idle_grace_preserves_immediate_teardown() {
        assert!(should_stop_publishing(
            0,
            Instant::now(),
            &mut None,
            Duration::ZERO
        ));
    }

    #[tokio::test]
    async fn sending_before_a_session_exists_is_a_connection_fault_not_a_drop() {
        let mut adapter = adapter();
        let error = adapter
            .send_access_unit(EncodedAccessUnit {
                annex_b: KEYFRAME_1920X1080,
                rtp_timestamp: 0,
                epoch: 1,
                starts_decodable_run: true,
                capture_timestamp_us: 0,
            })
            .await
            .err()
            .unwrap_or_else(|| panic!("an unconnected adapter must not report a send"));
        assert_eq!(error.kind(), AdapterErrorKind::Connection);
        assert!(error.retryable());
    }

    #[tokio::test]
    async fn disconnecting_without_a_session_is_not_a_failure() {
        assert!(adapter().disconnect().await.is_ok());
    }

    #[test]
    fn a_lifetime_outside_the_permitted_range_is_a_configuration_fault() {
        let error = TranscodePublicationAdapter::new(&config(0), Duration::from_secs(3600))
            .err()
            .unwrap_or_else(|| panic!("an hour-long publisher token must be refused"));
        assert_eq!(error.kind(), AdapterErrorKind::Unavailable);
    }

    #[test]
    fn the_credentials_never_reach_a_log_line() {
        let adapter = adapter();
        let rendered = format!("{:?}", adapter.tokens);
        assert!(!rendered.contains("development-secret"));
        assert!(!rendered.contains("devkey"));
    }

    #[test]
    fn the_published_size_comes_from_the_stream_and_honours_the_ceiling() {
        let uncapped = publish_size(KEYFRAME_1920X1080, 0)
            .unwrap_or_else(|error| panic!("{error:?} for real 1080p media"));
        assert_eq!((uncapped.width(), uncapped.height()), (1920, 1080));

        let capped = publish_size(KEYFRAME_1920X1080, 720)
            .unwrap_or_else(|error| panic!("{error:?} for real 1080p media"));
        assert_eq!((capped.width(), capped.height()), (1280, 720));

        // A ceiling above the stream must not scale it up: inventing detail
        // costs the same CPU as carrying real detail and delivers less.
        let generous = publish_size(KEYFRAME_1920X1080, 2160)
            .unwrap_or_else(|error| panic!("{error:?} for real 1080p media"));
        assert_eq!((generous.width(), generous.height()), (1920, 1080));
    }

    #[test]
    fn an_access_unit_without_parameter_sets_is_a_transport_fault() {
        // Not a silent fallback to a configured default: publishing a track at
        // a guessed size would be wrong for the whole session.
        let error = publish_size(&[0, 0, 0, 1, 0x41, 0x9a], 0)
            .err()
            .unwrap_or_else(|| panic!("a stream with no SPS has no publishable size"));
        assert_eq!(error.kind(), AdapterErrorKind::Transport);
    }

    #[test]
    fn a_plane_copy_honours_stride_and_refuses_a_short_destination() {
        let source: Vec<u8> = (0..12_u8).collect();
        let mut destination = vec![0_u8; 4 * 3];
        assert!(copy_plane(&source, 3, 4, &mut destination, 3));
        assert_eq!(destination[..12], source[..12]);

        // Padded rows: the copy must land on stride boundaries, not run on.
        let mut padded = vec![0xFF_u8; 4 * 5];
        assert!(copy_plane(&source, 3, 4, &mut padded, 5));
        assert_eq!(padded[0..3], source[0..3]);
        assert_eq!(padded[5..8], source[3..6]);
        assert_eq!(padded[15..18], source[9..12]);
        assert_eq!(padded[3], 0xFF, "padding must be left alone");

        let mut short = vec![0_u8; 5];
        assert!(
            !copy_plane(&source, 3, 4, &mut short, 3),
            "a destination that cannot hold the plane must be refused, not truncated"
        );
    }

    #[test]
    fn a_frame_that_is_not_exactly_one_picture_is_refused() {
        // The uninitialised-memory guard: `I420Buffer::new` does not clear its
        // planes, so a frame that cannot be filled whole must never reach a
        // subscriber. Refusing at the split is what makes that reachable.
        let size = FrameSize::new(64, 64).unwrap_or_else(|error| panic!("{error}"));
        let exact = vec![0_u8; size.frame_bytes()];
        let (luma, blue, red) = split_planes(&exact, size)
            .unwrap_or_else(|| panic!("an exactly sized frame must split"));
        assert_eq!(luma.len(), 64 * 64);
        assert_eq!((blue.len(), red.len()), (32 * 32, 32 * 32));

        assert!(
            split_planes(&exact[..exact.len() - 1], size).is_none(),
            "one byte short is not a picture"
        );
        assert!(
            split_planes(&[], size).is_none(),
            "an empty read is not a picture"
        );
    }
}
