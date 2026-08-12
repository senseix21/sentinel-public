//! Joint ownership of one camera ingest and its mandatory recorder.

use std::{
    future::Future,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use surveillance_camera::{
    connection::ConnectionSnapshot,
    health::{
        camera_health_channel, CameraHealthPublisher, CameraHealthReader, CameraOperationalMetrics,
    },
    rtsp::RtspSourceConfig,
    supervisor::{
        run_h264_camera_observed, JitterSource, ReconnectPolicy, SupervisorCounters,
        SupervisorExit, SupervisorStopReason,
    },
};
use surveillance_common::types::CameraId;
use surveillance_livekit::owner::{PublicationHealth, PublicationSnapshot};
use surveillance_media::{
    distribution::{MediaDistributor, MediaMetricsObserver, MediaSubscriber, QueueMetrics},
    packet::H264AccessUnit,
};
use surveillance_recorder::{
    clock::MediaWallClock,
    fmp4::Fmp4SegmentSink,
    runtime::{
        run_segment_recorder, RecordingLifecycleEvent, SegmentRecorderError, SegmentRecorderExit,
    },
};
use surveillance_storage::{filesystem::RecordingLayout, segment_store::SegmentFileStore};
use tokio::sync::{mpsc, watch};

/// Final camera evidence without retaining the distributor that must be
/// dropped before the recorder can observe end-of-input.
#[derive(Clone, Debug)]
pub struct CameraOutcome {
    pub reason: SupervisorStopReason,
    pub snapshot: ConnectionSnapshot,
    pub counters: SupervisorCounters,
    pub recorder_queue: QueueMetrics,
    pub live_queue: QueueMetrics,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PipelineStopReason {
    Requested,
    CameraStopped,
    RecorderStopped,
}

#[derive(Debug)]
pub struct OneCameraExit {
    pub reason: PipelineStopReason,
    pub camera: CameraOutcome,
    pub recorder: Result<SegmentRecorderExit, SegmentRecorderError>,
}

#[derive(Clone, Debug)]
pub struct OneCameraHealthSnapshot {
    pub connection: ConnectionSnapshot,
    pub camera: CameraOperationalMetrics,
    pub recorder_queue: QueueMetrics,
    pub live_queue: QueueMetrics,
    pub recorder: RecorderOperationalMetrics,
    pub publication: PublicationSnapshot,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RecorderOperationalMetrics {
    pub completed_segments: u64,
    pub failed_segments: u64,
}

#[derive(Clone)]
pub struct RecorderHealthPublisher(Arc<RecorderMetricState>);

impl RecorderHealthPublisher {
    pub(crate) fn completed(&self) {
        self.0.completed.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn failed(&self) {
        self.0.failed.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Default)]
struct RecorderMetricState {
    completed: AtomicU64,
    failed: AtomicU64,
}

/// Constant-memory, non-media-retaining health view for an active pipeline.
#[derive(Clone)]
pub struct OneCameraHealth {
    connection: CameraHealthReader,
    media: MediaMetricsObserver<H264AccessUnit>,
    recorder: Arc<RecorderMetricState>,
    publication: PublicationHealth,
}

impl OneCameraHealth {
    #[must_use]
    pub fn snapshot(&self) -> OneCameraHealthSnapshot {
        OneCameraHealthSnapshot {
            connection: self.connection.snapshot(),
            camera: self.connection.operational_metrics(),
            recorder_queue: self.media.recorder(),
            live_queue: self.media.live(),
            recorder: RecorderOperationalMetrics {
                completed_segments: self.recorder.completed.load(Ordering::Relaxed),
                failed_segments: self.recorder.failed.load(Ordering::Relaxed),
            },
            publication: self.publication.snapshot(),
        }
    }

    /// Waits for the next camera lifecycle transition and samples queue state.
    ///
    /// # Errors
    ///
    /// Returns an error after the camera supervisor drops its publisher.
    pub async fn connection_changed(
        &mut self,
    ) -> Result<OneCameraHealthSnapshot, tokio::sync::watch::error::RecvError> {
        let _ = self.connection.changed().await?;
        Ok(self.snapshot())
    }
}

#[must_use]
pub fn one_camera_health(
    camera_id: CameraId,
    distributor: &MediaDistributor<H264AccessUnit>,
    publication: PublicationHealth,
) -> (
    CameraHealthPublisher,
    RecorderHealthPublisher,
    OneCameraHealth,
) {
    let (publisher, connection) = camera_health_channel(camera_id);
    let recorder = Arc::new(RecorderMetricState::default());
    (
        publisher,
        RecorderHealthPublisher(Arc::clone(&recorder)),
        OneCameraHealth {
            connection,
            media: distributor.metrics_observer(),
            recorder,
            publication,
        },
    )
}

/// Inputs whose ownership defines one complete camera-to-recorder pipeline.
pub struct OneCameraRuntime<'a, J> {
    pub camera_id: &'a CameraId,
    pub source: &'a RtspSourceConfig,
    pub distributor: MediaDistributor<H264AccessUnit>,
    pub recorder_subscription: MediaSubscriber<H264AccessUnit>,
    pub reconnect: ReconnectPolicy,
    pub jitter: &'a mut J,
    pub recorder: Fmp4SegmentSink,
    pub layout: &'a RecordingLayout,
    pub store: &'a SegmentFileStore,
    pub clock: MediaWallClock,
    pub lifecycle_events: &'a mpsc::Sender<RecordingLifecycleEvent>,
    pub health: CameraHealthPublisher,
}

/// Runs camera and recorder concurrently under one structured lifetime.
///
/// If either mandatory child stops, this owner stops the other child and
/// waits for it. Requested shutdown first stops ingest, drops the distributor
/// to close media input, and then permits the recorder to drain and finalize.
pub async fn run_one_camera<J: JitterSource>(
    runtime: OneCameraRuntime<'_, J>,
    shutdown: watch::Receiver<bool>,
) -> OneCameraExit {
    let camera_id = runtime.camera_id.clone();
    let (child_shutdown, child_shutdown_rx) = watch::channel(false);
    let camera = run_h264_camera_observed(
        runtime.camera_id.clone(),
        runtime.source,
        runtime.distributor,
        runtime.reconnect,
        runtime.jitter,
        child_shutdown_rx.clone(),
        runtime.health,
    );
    let recorder = run_segment_recorder(
        runtime.camera_id,
        runtime.recorder_subscription,
        runtime.recorder,
        runtime.layout,
        runtime.store,
        runtime.clock,
        runtime.lifecycle_events,
        child_shutdown_rx,
    );
    let exit = Box::pin(coordinate(camera, recorder, shutdown, child_shutdown)).await;
    tracing::info!(
        camera_id = %camera_id,
        reason = ?exit.reason,
        camera_reason = ?exit.camera.reason,
        access_units = exit.camera.counters.access_units,
        packet_loss_events = exit.camera.counters.packet_loss_events,
        recorder_rejected_full = exit.camera.recorder_queue.rejected_full,
        live_dropped_oldest = exit.camera.live_queue.dropped_oldest,
        recorder_ok = exit.recorder.is_ok(),
        "one-camera pipeline stopped"
    );
    exit
}

async fn coordinate<C, R>(
    camera: C,
    recorder: R,
    mut requested_shutdown: watch::Receiver<bool>,
    child_shutdown: watch::Sender<bool>,
) -> OneCameraExit
where
    C: Future<Output = SupervisorExit>,
    R: Future<Output = Result<SegmentRecorderExit, SegmentRecorderError>>,
{
    tokio::pin!(camera);
    tokio::pin!(recorder);
    loop {
        if *requested_shutdown.borrow() {
            request_stop(&child_shutdown);
            let camera = release_distributor(camera.await);
            let recorder = recorder.await;
            return OneCameraExit {
                reason: PipelineStopReason::Requested,
                camera,
                recorder,
            };
        }
        tokio::select! {
            biased;
            changed = requested_shutdown.changed() => {
                if changed.is_err() || *requested_shutdown.borrow_and_update() {
                    request_stop(&child_shutdown);
                    let camera = camera.await;
                    let camera = release_distributor(camera);
                    let recorder = recorder.await;
                    return OneCameraExit { reason: PipelineStopReason::Requested, camera, recorder };
                }
            }
            camera_exit = &mut camera => {
                let camera = release_distributor(camera_exit);
                let recorder = recorder.await;
                return OneCameraExit { reason: PipelineStopReason::CameraStopped, camera, recorder };
            }
            recorder_exit = &mut recorder => {
                request_stop(&child_shutdown);
                let camera = release_distributor(camera.await);
                return OneCameraExit {
                    reason: PipelineStopReason::RecorderStopped,
                    camera,
                    recorder: recorder_exit,
                };
            }
        }
    }
}

fn request_stop(sender: &watch::Sender<bool>) {
    sender.send_replace(true);
}

fn release_distributor(exit: SupervisorExit) -> CameraOutcome {
    let SupervisorExit {
        reason,
        snapshot,
        counters,
        distributor,
    } = exit;
    let recorder_queue = distributor.recorder_metrics();
    let live_queue = distributor.live_metrics();
    drop(distributor);
    CameraOutcome {
        reason,
        snapshot,
        counters,
        recorder_queue,
        live_queue,
    }
}

#[cfg(test)]
mod tests {
    use super::{coordinate, PipelineStopReason};
    use surveillance_camera::{
        connection::DisconnectedCamera,
        supervisor::{SupervisorCounters, SupervisorExit, SupervisorStopReason},
    };
    use surveillance_common::types::CameraId;
    use surveillance_media::{
        distribution::{MediaDistributor, QueueCapacity, ReceiveOutcome},
        packet::H264AccessUnit,
    };
    use surveillance_recorder::runtime::{
        SegmentRecorderCounters, SegmentRecorderExit, SegmentRecorderStopReason,
    };
    use tokio::sync::watch;

    fn camera_exit(
        distributor: MediaDistributor<H264AccessUnit>,
        reason: SupervisorStopReason,
    ) -> SupervisorExit {
        let camera_id = CameraId::new("camera-01")
            .unwrap_or_else(|error| panic!("test camera id must be valid: {error}"));
        let stopped = DisconnectedCamera::new(camera_id)
            .begin_connect()
            .unwrap_or_else(|_| panic!("fresh camera lifecycle must connect"))
            .stop();
        SupervisorExit {
            reason,
            snapshot: stopped.snapshot(),
            counters: SupervisorCounters::default(),
            distributor,
        }
    }

    fn distributor() -> (
        MediaDistributor<H264AccessUnit>,
        surveillance_media::distribution::MediaSubscriptions<H264AccessUnit>,
    ) {
        let capacity = QueueCapacity::new(1)
            .unwrap_or_else(|error| panic!("test capacity must be valid: {error}"));
        MediaDistributor::new(capacity, capacity)
    }

    fn recorder_exit(reason: SegmentRecorderStopReason) -> SegmentRecorderExit {
        SegmentRecorderExit {
            reason,
            counters: SegmentRecorderCounters::default(),
        }
    }

    #[tokio::test]
    async fn camera_exit_drops_producer_then_waits_for_recorder_drain() {
        let (distributor, mut subscriptions) = distributor();
        drop(subscriptions.live);
        let camera =
            async move { camera_exit(distributor, SupervisorStopReason::TerminalConnection) };
        let recorder = async move {
            assert!(matches!(
                subscriptions.recorder.receive().await,
                ReceiveOutcome::Closed
            ));
            Ok(recorder_exit(SegmentRecorderStopReason::InputDrained))
        };
        let (_requested_tx, requested_rx) = watch::channel(false);
        let (child_tx, _child_rx) = watch::channel(false);

        let exit = coordinate(camera, recorder, requested_rx, child_tx).await;

        assert_eq!(exit.reason, PipelineStopReason::CameraStopped);
        assert!(matches!(
            exit.recorder,
            Ok(SegmentRecorderExit {
                reason: SegmentRecorderStopReason::InputDrained,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn recorder_exit_requests_camera_stop_and_joins_it() {
        let (distributor, subscriptions) = distributor();
        drop(subscriptions);
        let (child_tx, mut child_rx) = watch::channel(false);
        let camera = async move {
            child_rx
                .changed()
                .await
                .unwrap_or_else(|error| panic!("owner must retain shutdown sender: {error}"));
            assert!(*child_rx.borrow());
            camera_exit(distributor, SupervisorStopReason::Requested)
        };
        let recorder = async { Ok(recorder_exit(SegmentRecorderStopReason::Requested)) };
        let (_requested_tx, requested_rx) = watch::channel(false);

        let exit = coordinate(camera, recorder, requested_rx, child_tx).await;

        assert_eq!(exit.reason, PipelineStopReason::RecorderStopped);
        assert_eq!(exit.camera.reason, SupervisorStopReason::Requested);
    }

    #[tokio::test]
    async fn recorder_failure_is_retained_and_stops_only_its_camera_child() {
        let (distributor, subscriptions) = distributor();
        drop(subscriptions);
        let (child_tx, mut child_rx) = watch::channel(false);
        let camera = async move {
            child_rx
                .changed()
                .await
                .unwrap_or_else(|error| panic!("shutdown signal retained: {error}"));
            camera_exit(distributor, SupervisorStopReason::Requested)
        };
        let recorder = async {
            Err(surveillance_recorder::runtime::SegmentRecorderError::MetadataChannelClosed)
        };
        let (_requested_tx, requested_rx) = watch::channel(false);

        let exit = coordinate(camera, recorder, requested_rx, child_tx).await;

        assert_eq!(exit.reason, PipelineStopReason::RecorderStopped);
        assert!(matches!(
            exit.recorder,
            Err(surveillance_recorder::runtime::SegmentRecorderError::MetadataChannelClosed)
        ));
        assert_eq!(exit.camera.reason, SupervisorStopReason::Requested);
    }

    #[tokio::test]
    async fn requested_shutdown_stops_and_joins_both_children() {
        let (distributor, subscriptions) = distributor();
        drop(subscriptions);
        let (child_tx, child_rx) = watch::channel(false);
        let mut camera_shutdown = child_rx.clone();
        let mut recorder_shutdown = child_rx;
        let camera = async move {
            camera_shutdown.changed().await.unwrap_or_else(|error| {
                panic!("owner must retain camera shutdown sender: {error}")
            });
            camera_exit(distributor, SupervisorStopReason::Requested)
        };
        let recorder = async move {
            recorder_shutdown.changed().await.unwrap_or_else(|error| {
                panic!("owner must retain recorder shutdown sender: {error}")
            });
            Ok(recorder_exit(SegmentRecorderStopReason::Requested))
        };
        let (requested_tx, requested_rx) = watch::channel(false);
        requested_tx.send_replace(true);

        let exit = coordinate(camera, recorder, requested_rx, child_tx).await;

        assert_eq!(exit.reason, PipelineStopReason::Requested);
        assert_eq!(exit.camera.reason, SupervisorStopReason::Requested);
        assert!(matches!(
            exit.recorder,
            Ok(SegmentRecorderExit {
                reason: SegmentRecorderStopReason::Requested,
                ..
            })
        ));
    }
}
