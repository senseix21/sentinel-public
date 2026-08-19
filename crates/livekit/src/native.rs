//! In-process `LiveKit` publication of already-encoded H.264.
//!
//! The child-process adapter this replaces drained about three access units a
//! second regardless of bitrate, which is well under any real camera's frame
//! rate; see `docs/adr/011-native-encoded-livekit-publication.md`. This one
//! hands the encoded access unit straight to the transport, so the camera's
//! frame rate is what the subscriber receives.
//!
//! Nothing here decodes or re-encodes. The bytes published are the bytes the
//! camera sent, which is also why the track's resolution is read from the
//! stream's own sequence parameter set rather than from configuration.

use std::time::Duration;

use livekit::{
    options::{TrackPublishOptions, VideoCodec, VideoEncoderBackend},
    rtc_engine::EngineError,
    track::{LocalTrack, LocalVideoTrack, TrackSource},
    webrtc::{
        video_frame::{EncodedFrameType, EncodedVideoCodec, EncodedVideoFrame},
        video_source::{native::NativeVideoSource, RtcVideoSource, VideoResolution},
    },
    Room, RoomError, RoomOptions,
};
use livekit_api::signal_client::SignalError;
use surveillance_media::sps::CodedDimensions;

use crate::{
    adapter::{
        AdapterError, AdapterErrorKind, AdapterFuture, EncodedAccessUnit, PublicationAdapter,
        SendOutcome, TimestampAuthority,
    },
    config::LiveKitConfig,
    domain::PublicationIdentity,
    token::{PublisherTokenIssuer, TokenIssueError},
};

/// Publishes encoded access units into a `LiveKit` room from this process.
pub struct NativePublicationAdapter {
    identity: PublicationIdentity,
    tokens: PublisherTokenIssuer,
    endpoint: String,
    session: Option<Session>,
}

/// One connected room, and the track once its resolution is known.
struct Session {
    room: Room,
    track: Option<EncodedTrack>,
}

/// A published track and the source feeding it.
struct EncodedTrack {
    source: NativeVideoSource,
    resolution: VideoResolution,
}

impl NativePublicationAdapter {
    /// Lifetime of a publisher token.
    ///
    /// This is not a deployment knob. Unlike a viewer token, which is handed to
    /// a browser and must survive the user opening the page, a publisher token
    /// is minted and consumed inside this process within milliseconds of
    /// connecting, and a reconnect mints a fresh one. A minute is generous
    /// against clock skew and long enough for a slow handshake, and there is no
    /// deployment in which a longer-lived publisher credential would be an
    /// improvement.
    pub const DEFAULT_TOKEN_TTL: Duration = Duration::from_secs(60);

    /// Builds an adapter for one camera's publication identity.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError`] with [`AdapterErrorKind::Authentication`] when
    /// the configured credentials cannot mint a publisher token at all.
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
            session: None,
        })
    }

    /// Whether the pass-through encoder has asked for a keyframe since the
    /// last check, clearing the request.
    ///
    /// A subscriber that joins mid-stream, or one that loses a picture, raises
    /// PLI or FIR. This process cannot satisfy it — the camera owns the
    /// encoder — but the request is exactly the signal that a viewer is
    /// waiting for the next natural keyframe, so it is worth surfacing rather
    /// than discarding.
    #[must_use]
    pub fn take_keyframe_request(&self) -> bool {
        self.session
            .as_ref()
            .and_then(|session| session.track.as_ref())
            .is_some_and(|track| track.source.take_keyframe_request())
    }

    /// Creates and publishes the track once the stream has told us its size.
    async fn publish_track(&mut self, annex_b: &[u8]) -> Result<(), AdapterError> {
        let dimensions = CodedDimensions::from_annex_b(annex_b)
            // The caller only reaches here on an access unit the media plane
            // classified as starting a decodable run, which by definition
            // carries its parameter sets. A failure here means the bytes and
            // that classification disagree, which is not something to paper
            // over with a configured default.
            .map_err(|_| AdapterError::new(AdapterErrorKind::Transport))?;
        let resolution = VideoResolution {
            width: dimensions.width(),
            height: dimensions.height(),
        };

        // Encoded pass-through publication is not usable on Apple platforms
        // with webrtc-sys 0.3.42, and the failure mode is fatal to the whole
        // process rather than to this publisher.
        //
        // `TrackPublishOptions::video_encoder` does not choose the encoder. It
        // installs an `EncoderSelectorInterface` on the sender, and libwebrtc
        // consults a selector only *after* an encoder already exists: the first
        // encoder is built from the negotiated SDP format, which carries no
        // backend tag. `VideoEncoderFactory::InternalFactory::Create` then takes
        // its automatic-fallback path, which explicitly skips the pass-through
        // factory (`IsAutomaticFallbackBackend` returns false for it) and picks
        // the first real backend that supports H.264. On Apple that is
        // VideoToolbox, which is registered whenever `__APPLE__` is defined.
        //
        // The switch the selector asks for never arrives in time: the real
        // encoder receives the first pre-encoded access unit, `ToObjCVideoFrame`
        // casts our native `EncodedVideoFrameBuffer` to an `ObjCFrameBuffer`,
        // and `objc_retain` dereferences the payload as a pixel buffer. Measured
        // here: exactly 9 access units published, then SIGSEGV on the
        // "EncoderQueue" thread. That is not a Rust panic -- nothing catches it,
        // and recording dies with the process, violating the ordering that puts
        // preserving recordings first.
        //
        // Where no real H.264 encoder is registered the same code recovers:
        // initial creation fails, `OnEncoderBroken` fires, and the selector
        // switches to the pass-through backend.
        //
        // That escape hatch does not exist on any platform LiveKit ships.
        // `IsAutomaticFallbackBackend` excludes only `PreEncoded`, and the
        // factory registers OpenH264 under `WEBRTC_USE_H264`, which the
        // prebuilt defines everywhere. Only the *crash signature* is
        // Apple-specific, not the cause; Linux would select OpenH264 rather
        // than pass-through. This refusal is therefore narrower than the
        // defect, and is scoped to the platform where the failure is a
        // process-killing SIGSEGV rather than an unmeasured unknown.
        //
        // Reversal criterion: an upstream webrtc-sys that honours the requested
        // backend when the *initial* encoder is created. See ADR 011.
        //
        // `Unavailable` is not retryable, so this is reported once and the
        // camera keeps recording instead of entering a reconnect loop.
        if cfg!(target_vendor = "apple") {
            return Err(AdapterError::new(AdapterErrorKind::Unavailable));
        }

        let session = self
            .session
            .as_mut()
            .ok_or_else(|| AdapterError::new(AdapterErrorKind::Connection))?;
        let source = NativeVideoSource::new_encoded(resolution.clone());
        let track = LocalVideoTrack::create_video_track(
            self.identity.track.as_str(),
            RtcVideoSource::Native(source.clone()),
        );
        session
            .room
            .local_participant()
            .publish_track(
                LocalTrack::Video(track),
                TrackPublishOptions {
                    source: TrackSource::Camera,
                    video_codec: VideoCodec::H264,
                    // Without this the sender builds a real encoder --
                    // VideoToolbox on macOS -- and libwebrtc hands it the
                    // encoded access unit as though it were a raw frame. That
                    // is not a degraded picture: it dereferences the payload as
                    // a pixel buffer and takes the process down with SIGSEGV.
                    // `PreEncoded` selects the pass-through encoder, which is
                    // the only backend that understands what
                    // `capture_encoded_frame` produces.
                    video_encoder: VideoEncoderBackend::PreEncoded,
                    // Simulcast would require encoding additional layers, and
                    // this process has no encoder: it forwards what the camera
                    // produced. One layer is the only honest offer.
                    simulcast: false,
                    ..TrackPublishOptions::default()
                },
            )
            .await
            .map_err(|error| AdapterError::new(classify(&error)))?;
        session.track = Some(EncodedTrack { source, resolution });
        Ok(())
    }
}

impl PublicationAdapter for NativePublicationAdapter {
    fn timestamp_authority(&self) -> TimestampAuthority {
        TimestampAuthority::Media
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
            let (room, _events) = Room::connect(&self.endpoint, access.token(), options)
                .await
                .map_err(|error| AdapterError::new(classify(&error)))?;
            // The track waits for the first parameterised keyframe: an encoded
            // source is constructed with a resolution, and only the sequence
            // parameter set knows what it is.
            self.session = Some(Session { room, track: None });
            Ok(())
        })
    }

    fn send_access_unit<'a>(
        &'a mut self,
        access_unit: EncodedAccessUnit<'a>,
    ) -> AdapterFuture<'a, SendOutcome> {
        Box::pin(async move {
            if self.session.is_none() {
                return Err(AdapterError::new(AdapterErrorKind::Connection));
            }
            let published = self
                .session
                .as_ref()
                .is_some_and(|session| session.track.is_some());
            if !published {
                if !access_unit.starts_decodable_run {
                    // Nothing downstream could decode this yet. Waiting costs
                    // one keyframe interval; publishing a track sized from a
                    // guess would be wrong for the whole session.
                    return Ok(SendOutcome::Dropped);
                }
                self.publish_track(access_unit.annex_b).await?;
            }

            let Some(track) = self
                .session
                .as_ref()
                .and_then(|session| session.track.as_ref())
            else {
                return Err(AdapterError::new(AdapterErrorKind::Connection));
            };

            let frame = EncodedVideoFrame {
                codec: EncodedVideoCodec::H264,
                payload: access_unit.annex_b,
                timestamp_us: access_unit.capture_timestamp_us,
                frame_type: if access_unit.starts_decodable_run {
                    EncodedFrameType::Key
                } else {
                    EncodedFrameType::Delta
                },
                resolution: track.resolution.clone(),
                frame_metadata: None,
            };
            // A copy into the transport's queue, not a network write: this
            // does not block the executor, and a refusal means the transport
            // was momentarily not ready rather than that the session failed.
            if track.source.capture_encoded_frame(&frame) {
                Ok(SendOutcome::Published)
            } else {
                Ok(SendOutcome::Dropped)
            }
        })
    }

    fn disconnect(&mut self) -> AdapterFuture<'_, ()> {
        Box::pin(async move {
            if let Some(session) = self.session.take() {
                // Dropping the room closes it; awaiting the explicit close
                // lets the server see the participant leave rather than time
                // it out, which is what makes a restart look like a restart.
                let _ = session.room.close().await;
            }
            Ok(())
        })
    }
}

/// Maps a room failure onto the adapter's retry contract.
///
/// Only a rejected credential is terminal. Everything else is a connectivity
/// or transport fault that a fresh session may well survive, and treating one
/// of those as permanent would leave a camera unpublished until an operator
/// noticed.
pub(crate) fn classify(error: &RoomError) -> AdapterErrorKind {
    match error {
        RoomError::Engine(engine) => classify_engine(engine),
        RoomError::TrackAlreadyPublished => AdapterErrorKind::Transport,
        _ => AdapterErrorKind::Connection,
    }
}

fn classify_engine(error: &EngineError) -> AdapterErrorKind {
    match error {
        // The signal client documents `Client(401|403)` as the authoritative
        // terminal authentication result, distinct from a handshake status.
        EngineError::Signal(SignalError::Client(status, _))
            if matches!(status.as_u16(), 401 | 403) =>
        {
            AdapterErrorKind::Authentication
        }
        EngineError::Signal(SignalError::TokenFormat) => AdapterErrorKind::Authentication,
        _ => AdapterErrorKind::Connection,
    }
}

#[cfg(test)]
mod tests {

    use http::StatusCode;
    use surveillance_common::types::CameraId;

    use super::*;
    use crate::config::LiveKitSettings;

    fn config() -> LiveKitConfig {
        let camera = CameraId::new("camera-01").unwrap_or_else(|error| panic!("{error}"));
        LiveKitConfig::validate(
            &camera,
            LiveKitSettings {
                decoder_program: "ffmpeg".to_owned(),
                max_publish_height: 0,
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

    fn adapter() -> NativePublicationAdapter {
        NativePublicationAdapter::new(&config(), Duration::from_secs(300))
            .unwrap_or_else(|error| panic!("{error}"))
    }

    #[test]
    fn the_publisher_reads_its_timestamps_from_the_media() {
        assert_eq!(adapter().timestamp_authority(), TimestampAuthority::Media);
    }

    #[tokio::test]
    async fn sending_before_a_session_exists_is_a_connection_fault_not_a_drop() {
        let mut adapter = adapter();
        let error = adapter
            .send_access_unit(EncodedAccessUnit {
                annex_b: &[0, 0, 0, 1, 0x65, 1],
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
        // Shutdown runs on paths that may never have connected, and a publisher
        // that reports a failure there would make an ordinary stop look like a
        // fault.
        assert!(adapter().disconnect().await.is_ok());
    }

    #[test]
    fn an_adapter_with_no_track_reports_no_keyframe_request() {
        assert!(!adapter().take_keyframe_request());
    }

    #[test]
    fn a_lifetime_outside_the_permitted_range_is_a_configuration_fault() {
        // A token that outlives its usefulness is a standing credential; one
        // that expires before a reconnect finishes is an outage. Neither is an
        // authentication failure, so neither may be reported as one.
        let error = NativePublicationAdapter::new(&config(), Duration::from_secs(3600))
            .err()
            .unwrap_or_else(|| panic!("an hour-long publisher token must be refused"));
        assert_eq!(error.kind(), AdapterErrorKind::Unavailable);
    }

    #[test]
    fn only_a_rejected_credential_is_terminal() {
        let terminal = [
            RoomError::Engine(EngineError::Signal(SignalError::Client(
                StatusCode::UNAUTHORIZED,
                "invalid token".to_owned(),
            ))),
            RoomError::Engine(EngineError::Signal(SignalError::Client(
                StatusCode::FORBIDDEN,
                "grant refused".to_owned(),
            ))),
            RoomError::Engine(EngineError::Signal(SignalError::TokenFormat)),
        ];
        for error in &terminal {
            assert_eq!(classify(error), AdapterErrorKind::Authentication);
            assert!(!AdapterError::new(classify(error)).retryable());
        }

        let retryable = [
            // A handshake status is a WebSocket upgrade result, not the
            // authoritative `rtc/validate` answer, so a 401 seen here is not
            // the same fact as `Client(401)` and must not end the session.
            RoomError::Engine(EngineError::Signal(SignalError::Handshake {
                status: StatusCode::UNAUTHORIZED,
            })),
            RoomError::Engine(EngineError::Signal(SignalError::Client(
                StatusCode::TOO_MANY_REQUESTS,
                "slow down".to_owned(),
            ))),
            RoomError::Engine(EngineError::Signal(SignalError::Closed)),
            RoomError::Engine(EngineError::Connection("ice failed".into())),
            RoomError::AlreadyClosed,
        ];
        for error in &retryable {
            assert!(
                AdapterError::new(classify(error)).retryable(),
                "{error} must not end the publication"
            );
        }

        assert_eq!(
            classify(&RoomError::TrackAlreadyPublished),
            AdapterErrorKind::Transport
        );
    }

    #[test]
    fn the_credentials_never_reach_a_log_line() {
        let adapter = adapter();
        let rendered = format!("{:?}", adapter.tokens);
        assert!(!rendered.contains("development-secret"));
        assert!(!rendered.contains("devkey"));
    }
}
