//! Owned, restartable construction boundary for one camera-to-recorder pipeline.

use std::{
    collections::hash_map::DefaultHasher,
    error::Error,
    fmt,
    hash::{Hash, Hasher},
    sync::Arc,
    time::SystemTime,
};

use surveillance_camera::connection::{ConnectionState, StopReason};
use surveillance_camera::supervisor::{JitterSource, SupervisorStopReason};
use surveillance_config::settings::OneCameraConfig;
use surveillance_detector::{
    backend::PersonInferenceBackend,
    decoder::{FfmpegDecoderConfig, FfmpegH264Decoder},
    detector::{DetectionHealth, DetectionOwner},
    gate::GatePolicy,
    model_output::YOLOX_LETTERBOX_ALIGNMENT,
    sampler::TimestampSampler,
};
use surveillance_events::{
    bus::EventBus,
    event::{
        CameraState, DeliveryClass, DomainEvent, EventId, EventPayload, FailureCategory,
        OccurredAtUnixMicros,
    },
};
use surveillance_livekit::{
    owner::{LivePublicationOwner, PublicationHealth},
    transcode::TranscodePublicationAdapter,
};
use surveillance_media::distribution::MediaDistributor;
use surveillance_media::frame::FrameLimits;
use surveillance_recorder::{
    clock::MediaWallClock, fmp4::Fmp4SegmentSink, runtime::RecordingLifecycleEvent,
};
use surveillance_storage::segment_store::SegmentFileStore;
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
};

use crate::metadata_writer::MetadataWriterHandle;
use crate::one_camera::{
    one_camera_health, run_one_camera, OneCameraExit, OneCameraHealth, OneCameraRuntime,
    RecorderHealthPublisher,
};

/// Bounded diagnostic category retained after a pipeline exits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionableFailure {
    TerminalCameraConfiguration,
    RecorderBackpressure,
    RecorderFailure,
    PipelineTaskCancelled,
    PipelineTaskPanicked,
}

/// Construction or join failure without source URLs, credentials, or paths.
#[derive(Debug)]
pub enum CameraPipelineError {
    Reconciliation,
    DetectionConfiguration,
    /// A configured inference backend refused to come up. Carries the reason,
    /// because "detection configuration is invalid" tells an operator nothing
    /// about a missing model file or a hash that does not match.
    InferenceBackend(String),
    InferenceBackendTask,
    PublicationConfiguration,
    JoinCancelled,
    JoinPanicked,
}

impl fmt::Display for CameraPipelineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Self::InferenceBackend(reason) = self {
            return write!(
                formatter,
                "camera detection backend refused to start: {reason}"
            );
        }
        formatter.write_str(match self {
            Self::Reconciliation => "camera recording reconciliation discovery failed",
            Self::DetectionConfiguration => "camera detection configuration is invalid",
            Self::InferenceBackend(_) => unreachable!("handled above"),
            Self::InferenceBackendTask => "camera detection backend initialization task failed",
            Self::PublicationConfiguration => "camera live publication configuration is invalid",
            Self::JoinCancelled => "camera pipeline task was cancelled",
            Self::JoinPanicked => "camera pipeline task panicked",
        })
    }
}

impl Error for CameraPipelineError {}

/// Singular owner of a spawned camera pipeline and its cooperative stop signal.
pub struct CameraPipeline {
    shutdown: watch::Sender<bool>,
    health: OneCameraHealth,
    join: Option<JoinHandle<OneCameraExit>>,
}

impl CameraPipeline {
    /// Transactionally constructs and starts one owned pipeline.
    ///
    /// # Errors
    ///
    /// Reserved for construction failures introduced by concrete media/storage
    /// boundaries; process-level reconciliation runs once before supervision.
    #[allow(
        clippy::too_many_lines,
        reason = "one construction boundary keeps child ownership explicit"
    )]
    pub async fn start(
        config: Arc<OneCameraConfig>,
        metadata_writer: Option<MetadataWriterHandle>,
        event_bus: EventBus,
    ) -> Result<Self, CameraPipelineError> {
        let lifecycle_bus = event_bus.clone();
        let (distributor, subscriptions) = MediaDistributor::new_with_detection(
            config.recorder_queue,
            config.live_queue,
            config.detection_queue,
        );
        let (detection, detection_health) = if config.detection_enabled {
            // Frames are produced at the model's input geometry, not the
            // camera's, so the bound is on what the detector actually holds.
            // At 1080p into a 416x416 model this is roughly a fifth of what the
            // camera's own geometry would have reserved.
            let limits = FrameLimits::new(
                config.inference.input_width,
                config.inference.input_height,
                config.decoded_frame_max_bytes,
            )
            .map_err(|_| CameraPipelineError::DetectionConfiguration)?;
            let sampler =
                TimestampSampler::new(config.detection_sample_rate, config.detection_max_gap)
                    .map_err(|_| CameraPipelineError::DetectionConfiguration)?;
            // Selection, not construction: a configured model that cannot be
            // loaded, or that this build has no backend for, refuses the
            // pipeline rather than silently running the fixture. The error
            // carries the reason so an operator can act on it.
            let inference_config = config.inference.clone();
            let inference = tokio::task::spawn_blocking(move || {
                PersonInferenceBackend::select(&inference_config)
            })
            .await
            .map_err(|_| CameraPipelineError::InferenceBackendTask)?
            .map_err(|error| CameraPipelineError::InferenceBackend(error.to_string()))?;
            let decoder = FfmpegH264Decoder::start(FfmpegDecoderConfig {
                executable: config.detection_decoder.clone(),
                width: config.inference.input_width,
                height: config.inference.input_height,
                hwaccel: config.detection_hwaccel.clone(),
                letterbox_alignment: YOLOX_LETTERBOX_ALIGNMENT,
                frame_limits: limits,
                response_timeout: config.detection_decode_timeout,
                max_access_unit_bytes: config.max_access_unit_bytes,
            })
            .map_err(|_| CameraPipelineError::DetectionConfiguration)?;
            let gate_policy = GatePolicy::new(
                config.detection_motion_threshold_percent,
                config.detection_gate_cooldown,
                config.detection_gate_heartbeat,
            )
            .map_err(|_| CameraPipelineError::DetectionConfiguration)?;
            let (owner, health) = DetectionOwner::start_observed(
                config.camera_id.clone(),
                subscriptions.detection,
                sampler,
                decoder,
                inference,
                gate_policy,
                event_bus,
            );
            (Some(owner), health)
        } else {
            drop(subscriptions.detection);
            (None, DetectionHealth::disabled())
        };
        let (publication_task, publication_health) = if let Some(livekit) = config.livekit.clone() {
            // A publisher that cannot mint its own token will never publish.
            // Reporting that at startup is better than a camera whose live
            // view silently never appears.
            //
            // The transcoding adapter, not the encoded pass-through one:
            // pass-through crashes the process on Apple platforms and selects
            // the wrong encoder everywhere else (ADR 011 Decision 5, corrected;
            // ADR 012). `NativePublicationAdapter` is kept, not deleted, so the
            // reversal criterion in ADR 012 is a constructor change here.
            //
            // Nothing decodes until a viewer joins the room, so a deployment
            // with no operator watching pays nothing for this.
            let adapter = TranscodePublicationAdapter::new(
                &livekit,
                TranscodePublicationAdapter::DEFAULT_TOKEN_TTL,
            )
            .map_err(|_| CameraPipelineError::PublicationConfiguration)?;
            let (owner, health) = LivePublicationOwner::new(livekit, adapter, subscriptions.live);
            (Some(owner), health)
        } else {
            drop(subscriptions.live);
            (None, PublicationHealth::disabled())
        };
        let (health_publisher, recorder_health, health) = one_camera_health(
            config.camera_id.clone(),
            &distributor,
            publication_health,
            detection_health,
        );
        let (shutdown, shutdown_rx) = watch::channel(false);
        let mut camera_event_health = health.clone();
        let join = tokio::spawn(async move {
            let mut camera_event_sequence = 0_u64;
            let mut publication =
                publication_task.map(|owner| Box::pin(owner.run(shutdown_rx.clone())));
            let store = SegmentFileStore::new(config.layout.clone(), config.ffprobe.clone());
            let recorder = Fmp4SegmentSink::new(config.recorder);
            let clock = MediaWallClock::new(config.clock_drift);
            let (event_tx, mut event_rx) = mpsc::channel(config.lifecycle_event_items.get());
            let mut jitter = ProcessJitter::new();
            let pipeline = run_one_camera(
                OneCameraRuntime {
                    camera_id: &config.camera_id,
                    source: &config.source,
                    distributor,
                    recorder_subscription: subscriptions.recorder,
                    reconnect: config.reconnect,
                    jitter: &mut jitter,
                    recorder,
                    layout: &config.layout,
                    store: &store,
                    clock,
                    lifecycle_events: &event_tx,
                    health: health_publisher,
                },
                shutdown_rx,
            );
            tokio::pin!(pipeline);
            let exit = loop {
                tokio::select! {
                    exit = &mut pipeline => break exit,
                    changed = camera_event_health.connection_changed() => {
                        if let Ok(snapshot) = changed {
                            camera_event_sequence = camera_event_sequence.saturating_add(1);
                            publish_camera_state(&snapshot.connection, camera_event_sequence, &lifecycle_bus);
                        }
                    }
                    event = event_rx.recv() => handle_lifecycle_event(event, &recorder_health, metadata_writer.as_ref(), config.layout.root(), &lifecycle_bus),
                    publication_exit = async {
                        match publication.as_mut() {
                            Some(publication) => Some(publication.await),
                            None => std::future::pending().await,
                        }
                    } => {
                        if let Some(publication_exit) = publication_exit {
                            tracing::warn!(reason=?publication_exit.reason, state=?publication_exit.snapshot.state, "optional LiveKit publication stopped; recording continues");
                            publication = None;
                        }
                    }
                }
            };
            while let Ok(event) = event_rx.try_recv() {
                handle_lifecycle_event(
                    Some(event),
                    &recorder_health,
                    metadata_writer.as_ref(),
                    config.layout.root(),
                    &lifecycle_bus,
                );
            }
            if let Some(publication) = publication {
                let publication_exit = publication.await;
                tracing::info!(reason=?publication_exit.reason, "LiveKit publication owner joined");
            }
            if let Some(detection) = detection {
                match detection.stop_and_join().await {
                    Ok(exit) => tracing::info!(reason=?exit.reason, "detection owner joined"),
                    Err(_) => {
                        tracing::warn!("detection owner join failed; recording exit preserved");
                    }
                }
            }
            exit
        });
        Ok(Self {
            shutdown,
            health,
            join: Some(join),
        })
    }

    #[must_use]
    pub fn health(&self) -> OneCameraHealth {
        self.health.clone()
    }

    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.join.as_ref().is_none_or(JoinHandle::is_finished)
    }

    pub fn request_stop(&self) {
        self.shutdown.send_replace(true);
    }

    /// Requests cooperative shutdown and consumes the one join result.
    ///
    /// # Errors
    ///
    /// Classifies task cancellation and panic without exposing panic payloads.
    pub async fn stop_and_join(&mut self) -> Result<Option<OneCameraExit>, CameraPipelineError> {
        self.request_stop();
        let Some(join) = self.join.take() else {
            return Ok(None);
        };
        match join.await {
            Ok(exit) => Ok(Some(exit)),
            Err(error) if error.is_cancelled() => Err(CameraPipelineError::JoinCancelled),
            Err(_) => Err(CameraPipelineError::JoinPanicked),
        }
    }

    /// Joins a child already known to have completed without changing its result.
    ///
    /// # Errors
    ///
    /// Classifies task cancellation and panic without exposing panic payloads.
    pub async fn join_finished(&mut self) -> Result<Option<OneCameraExit>, CameraPipelineError> {
        let Some(join) = self.join.take() else {
            return Ok(None);
        };
        match join.await {
            Ok(exit) => Ok(Some(exit)),
            Err(error) if error.is_cancelled() => Err(CameraPipelineError::JoinCancelled),
            Err(_) => Err(CameraPipelineError::JoinPanicked),
        }
    }
}

fn publish_camera_state(
    snapshot: &surveillance_camera::connection::ConnectionSnapshot,
    sequence: u64,
    bus: &EventBus,
) {
    let state = match snapshot.state() {
        ConnectionState::Disconnected => CameraState::Disconnected,
        ConnectionState::Connecting { .. } => CameraState::Connecting,
        ConnectionState::Connected { .. } => CameraState::Connected,
        ConnectionState::Reconnecting { .. } => CameraState::Reconnecting,
        ConnectionState::Stopped {
            reason: StopReason::Terminal(_) | StopReason::LifecycleExhausted(_),
        } => CameraState::Terminal,
        ConnectionState::Stopped {
            reason: StopReason::Requested,
        } => CameraState::Stopped,
    };
    let Some(occurred_at) = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_micros()).ok())
        .and_then(|value| OccurredAtUnixMicros::new(value).ok())
    else {
        return;
    };
    let mut camera_hash = DefaultHasher::new();
    snapshot.camera_id().hash(&mut camera_hash);
    let Ok(event_id) = EventId::new(format!("cam-{:016x}-{sequence}", camera_hash.finish())) else {
        return;
    };
    let Ok(event) = DomainEvent::new(
        event_id,
        occurred_at,
        Some(snapshot.camera_id().clone()),
        DeliveryClass::LatestState,
        EventPayload::CameraStateChanged { state },
    ) else {
        return;
    };
    let _ = bus.publish(Arc::new(event));
}

impl Drop for CameraPipeline {
    fn drop(&mut self) {
        self.shutdown.send_replace(true);
        if let Some(join) = self.join.take() {
            join.abort();
        }
    }
}

#[must_use]
pub fn classify_exit(exit: &OneCameraExit) -> Option<ActionableFailure> {
    if exit.recorder.is_err() {
        return Some(ActionableFailure::RecorderFailure);
    }
    match exit.camera.reason {
        SupervisorStopReason::TerminalConnection => {
            Some(ActionableFailure::TerminalCameraConfiguration)
        }
        SupervisorStopReason::RecorderBackpressure => Some(ActionableFailure::RecorderBackpressure),
        SupervisorStopReason::Requested | SupervisorStopReason::LifecycleExhausted => None,
    }
}

fn handle_lifecycle_event(
    event: Option<RecordingLifecycleEvent>,
    health: &RecorderHealthPublisher,
    metadata_writer: Option<&MetadataWriterHandle>,
    recording_root: &std::path::Path,
    event_bus: &EventBus,
) {
    match event.as_ref() {
        Some(RecordingLifecycleEvent::Completed { segment, .. }) => {
            health.completed();
            tracing::info!(path = %segment.path().display(), "recording lifecycle event delivered");
        }
        Some(RecordingLifecycleEvent::Failed { segment, .. }) => {
            health.failed();
            tracing::error!(failure = ?segment.failure(), "recording lifecycle failure delivered");
        }
        None => tracing::debug!("recording lifecycle channel closed"),
    }
    if let Some(event) = event.as_ref() {
        publish_recording_domain_event(event, event_bus);
    }
    if let (Some(event), Some(writer)) = (event, metadata_writer) {
        if let Err(error) = writer.try_submit(event, recording_root) {
            tracing::warn!(category = ?error, "recording metadata fact remains filesystem reconciliation evidence");
        }
    }
}

fn publish_recording_domain_event(event: &RecordingLifecycleEvent, bus: &EventBus) {
    let (camera_id, recording_id, occurred, payload) = match event {
        RecordingLifecycleEvent::Completed {
            camera_id,
            recording_id,
            segment,
        } => (
            camera_id,
            recording_id,
            segment.ended_at(),
            EventPayload::RecordingSegmentCompleted {
                recording_id: recording_id.clone(),
                size_bytes: segment.size_bytes(),
            },
        ),
        RecordingLifecycleEvent::Failed {
            camera_id,
            recording_id,
            segment,
        } => {
            let Ok(category) =
                FailureCategory::new(format!("{:?}", segment.failure()).to_ascii_lowercase())
            else {
                return;
            };
            (
                camera_id,
                recording_id,
                segment.ended_at().unwrap_or_else(SystemTime::now),
                EventPayload::RecordingSegmentFailed {
                    recording_id: recording_id.clone(),
                    failure_category: category,
                },
            )
        }
    };
    let Some(micros) = occurred
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_micros()).ok())
        .and_then(|value| OccurredAtUnixMicros::new(value).ok())
    else {
        return;
    };
    let suffix = if matches!(payload, EventPayload::RecordingSegmentCompleted { .. }) {
        "completed"
    } else {
        "failed"
    };
    let mut recording_hash = DefaultHasher::new();
    recording_id.hash(&mut recording_hash);
    let Ok(event_id) = EventId::new(format!("rec-{:016x}-{suffix}", recording_hash.finish()))
    else {
        return;
    };
    let Ok(domain_event) = DomainEvent::new(
        event_id,
        micros,
        Some(camera_id.clone()),
        DeliveryClass::CriticalLifecycle,
        payload,
    ) else {
        return;
    };
    let report = bus.publish(Arc::new(domain_event));
    if report.rejected_full > 0 {
        tracing::warn!(rejected=report.rejected_full, "critical recording event admission failed; indexed/filesystem reconciliation remains authoritative");
    }
}

struct ProcessJitter(u64);

impl ProcessJitter {
    fn new() -> Self {
        let seed = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(1, |duration| {
                duration.as_secs() ^ u64::from(duration.subsec_nanos())
            })
            .max(1);
        Self(seed)
    }
}

impl JitterSource for ProcessJitter {
    fn sample_basis_points(&mut self) -> u16 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        u16::try_from(self.0 % 10_001).unwrap_or(5_000)
    }
}
