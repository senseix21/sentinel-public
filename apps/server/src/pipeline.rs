//! Owned, restartable construction boundary for one camera-to-recorder pipeline.

use std::{error::Error, fmt, sync::Arc, time::SystemTime};

use surveillance_camera::supervisor::{JitterSource, SupervisorStopReason};
use surveillance_config::settings::OneCameraConfig;
use surveillance_media::distribution::MediaDistributor;
use surveillance_recorder::{
    clock::MediaWallClock, fmp4::Fmp4SegmentSink, runtime::RecordingLifecycleEvent,
};
use surveillance_storage::segment_store::SegmentFileStore;
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
};

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
    JoinCancelled,
    JoinPanicked,
}

impl fmt::Display for CameraPipelineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Reconciliation => "camera recording reconciliation discovery failed",
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
    pub fn start(config: Arc<OneCameraConfig>) -> Result<Self, CameraPipelineError> {
        let (distributor, subscriptions) =
            MediaDistributor::new(config.recorder_queue, config.live_queue);
        drop(subscriptions.live);
        let (health_publisher, recorder_health, health) =
            one_camera_health(config.camera_id.clone(), &distributor);
        let (shutdown, shutdown_rx) = watch::channel(false);
        let join = tokio::spawn(async move {
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
                    event = event_rx.recv() => log_lifecycle_event(event, &recorder_health),
                }
            };
            while let Ok(event) = event_rx.try_recv() {
                log_lifecycle_event(Some(event), &recorder_health);
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

fn log_lifecycle_event(event: Option<RecordingLifecycleEvent>, health: &RecorderHealthPublisher) {
    match event {
        Some(RecordingLifecycleEvent::Completed(segment)) => {
            health.completed();
            tracing::info!(path = %segment.path().display(), "recording lifecycle event delivered");
        }
        Some(RecordingLifecycleEvent::Failed(segment)) => {
            health.failed();
            tracing::error!(failure = ?segment.failure(), "recording lifecycle failure delivered");
        }
        None => tracing::debug!("recording lifecycle channel closed"),
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
