//! Owned asynchronous segment recorder runtime.

use std::{error::Error, fmt, time::SystemTime};

use surveillance_common::types::{CameraId, RecordingId};
use surveillance_media::{
    distribution::{MediaSubscriber, ReceiveOutcome},
    packet::H264AccessUnit,
};
use surveillance_storage::{
    filesystem::{RecordingLayout, SegmentPathAtError, SegmentPaths},
    segment_store::{ActiveSegmentFile, SegmentFileStore, SegmentStoreError},
};
use tokio::sync::{mpsc, watch};

use crate::{
    clock::{ClockMapError, MediaWallClock},
    fmp4::{Fmp4Output, Fmp4SegmentSink, Fmp4SinkError, MuxedSegment},
    recorder::H264AccessUnitSink,
    segment::{
        CompletedSegment, DurableFileEvidence, DurableFileEvidenceError, FailedSegment,
        OpenSegment, RecordingFormat, SegmentFailure, SegmentTransitionError, VideoDimensions,
        VideoDimensionsError,
    },
};

/// Critical lifecycle fact delivered through a caller-owned bounded channel.
#[derive(Debug)]
pub enum RecordingLifecycleEvent {
    Completed(CompletedSegment),
    Failed(FailedSegment),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SegmentRecorderCounters {
    pub access_units: u64,
    pub completed: u64,
    pub failed: u64,
    pub clock_epochs: u64,
    pub persisted_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SegmentRecorderStopReason {
    InputDrained,
    Requested,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SegmentRecorderExit {
    pub reason: SegmentRecorderStopReason,
    pub counters: SegmentRecorderCounters,
}

struct ActiveLifecycle {
    segment: OpenSegment,
    paths: SegmentPaths,
    writer: ActiveSegmentFile,
    started_wall_time: SystemTime,
    last_wall_time: SystemTime,
}

/// Runs one recorder until producer closure or cancellation and finalizes the
/// active segment before returning.
///
/// # Errors
///
/// Returns [`SegmentRecorderError`] for clock, muxer, lifecycle, storage,
/// identity, or critical metadata-delivery failure.
#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "the linear loop keeps media, file, lifecycle, and cancellation ownership auditable"
)]
pub async fn run_segment_recorder(
    camera_id: &CameraId,
    mut subscription: MediaSubscriber<H264AccessUnit>,
    mut sink: Fmp4SegmentSink,
    layout: &RecordingLayout,
    store: &SegmentFileStore,
    mut clock: MediaWallClock,
    events: &mpsc::Sender<RecordingLifecycleEvent>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<SegmentRecorderExit, SegmentRecorderError> {
    let mut counters = SegmentRecorderCounters::default();
    let max_segment_bytes = sink.max_segment_bytes();
    let mut active = None;
    let mut sequence = 0_u64;

    let reason = loop {
        let receive = tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow_and_update() {
                    break SegmentRecorderStopReason::Requested;
                }
                continue;
            }
            receive = subscription.receive() => receive,
        };
        let access_unit = match receive {
            ReceiveOutcome::Item(access_unit) => access_unit,
            ReceiveOutcome::Closed => break SegmentRecorderStopReason::InputDrained,
            ReceiveOutcome::Empty => continue,
        };
        let mapped = clock
            .observe(access_unit.timestamp(), SystemTime::now())
            .map_err(SegmentRecorderError::Clock)?;
        if mapped.epoch_reason().is_some() {
            counters.clock_epochs = counters.clock_epochs.saturating_add(1);
        }
        let previous_wall_time = active
            .as_ref()
            .map_or(mapped.wall_time(), |state: &ActiveLifecycle| {
                state.last_wall_time
            });
        sink.write_access_unit(&access_unit)
            .map_err(SegmentRecorderError::Muxer)?;
        counters.access_units = counters.access_units.saturating_add(1);

        while let Some(output) = sink.take_output() {
            match output {
                Fmp4Output::Bytes(bytes) => {
                    if active.is_none() {
                        sequence = sequence
                            .checked_add(1)
                            .ok_or(SegmentRecorderError::IdentityExhausted)?;
                        active = Some(
                            open_lifecycle(
                                camera_id,
                                layout,
                                store,
                                max_segment_bytes,
                                mapped.wall_time(),
                                sequence,
                            )
                            .await?,
                        );
                    }
                    append_bytes(&mut active, &bytes, events, &mut counters).await?;
                }
                Fmp4Output::Completed(muxed) => {
                    let lifecycle = active
                        .take()
                        .ok_or(SegmentRecorderError::MissingLifecycle)?;
                    let ended_at = if muxed.ended_at() == access_unit.timestamp() {
                        mapped.wall_time()
                    } else {
                        previous_wall_time
                    };
                    persist_and_publish(muxed, lifecycle, ended_at, store, events, &mut counters)
                        .await?;
                }
            }
        }

        if let Some(active) = &mut active {
            active.last_wall_time = mapped.wall_time();
        }
    };

    sink.finish().map_err(SegmentRecorderError::Muxer)?;
    while let Some(output) = sink.take_output() {
        match output {
            Fmp4Output::Bytes(bytes) => {
                append_bytes(&mut active, &bytes, events, &mut counters).await?;
            }
            Fmp4Output::Completed(muxed) => {
                let lifecycle = active
                    .take()
                    .ok_or(SegmentRecorderError::MissingLifecycle)?;
                let ended_at = lifecycle.last_wall_time;
                if ended_at > lifecycle.started_wall_time {
                    persist_and_publish(muxed, lifecycle, ended_at, store, events, &mut counters)
                        .await?;
                } else {
                    let failed = lifecycle.segment.fail(SegmentFailure::Shutdown);
                    publish_event(events, RecordingLifecycleEvent::Failed(failed)).await?;
                    counters.failed = counters.failed.saturating_add(1);
                }
            }
        }
    }

    Ok(SegmentRecorderExit { reason, counters })
}

async fn append_bytes(
    active: &mut Option<ActiveLifecycle>,
    bytes: &[u8],
    events: &mpsc::Sender<RecordingLifecycleEvent>,
    counters: &mut SegmentRecorderCounters,
) -> Result<(), SegmentRecorderError> {
    let result = active
        .as_mut()
        .ok_or(SegmentRecorderError::MissingLifecycle)?
        .writer
        .append(bytes)
        .await;
    if let Err(error) = result {
        let lifecycle = active
            .take()
            .ok_or(SegmentRecorderError::MissingLifecycle)?;
        tracing::error!(path = %lifecycle.paths.partial().display(), error = %error, "recording fragment append failed");
        let failed = lifecycle.segment.fail(SegmentFailure::Storage);
        publish_event(events, RecordingLifecycleEvent::Failed(failed)).await?;
        counters.failed = counters.failed.saturating_add(1);
        return Err(SegmentRecorderError::Storage(error));
    }
    Ok(())
}

async fn open_lifecycle(
    camera_id: &CameraId,
    layout: &RecordingLayout,
    store: &SegmentFileStore,
    max_segment_bytes: usize,
    started_at: SystemTime,
    sequence: u64,
) -> Result<ActiveLifecycle, SegmentRecorderError> {
    let recording_id = recording_id(started_at, sequence)?;
    let paths = layout
        .segment_paths_at(camera_id, &recording_id, started_at)
        .map_err(SegmentRecorderError::Path)?;
    let writer = store
        .begin(&paths, max_segment_bytes)
        .await
        .map_err(SegmentRecorderError::Storage)?;
    Ok(ActiveLifecycle {
        segment: OpenSegment::new(
            paths.clone(),
            started_at,
            RecordingFormat::FragmentedMp4H264,
        ),
        paths,
        writer,
        started_wall_time: started_at,
        last_wall_time: started_at,
    })
}

fn recording_id(
    started_at: SystemTime,
    sequence: u64,
) -> Result<RecordingId, SegmentRecorderError> {
    let elapsed = started_at
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|_| SegmentRecorderError::WallTimeBeforeUnixEpoch)?;
    RecordingId::new(format!(
        "r-{:x}-{:x}-{sequence:x}",
        elapsed.as_secs(),
        elapsed.subsec_nanos()
    ))
    .map_err(|_| SegmentRecorderError::IdentityInvalid)
}

async fn persist_and_publish(
    _muxed: MuxedSegment,
    lifecycle: ActiveLifecycle,
    ended_at: SystemTime,
    store: &SegmentFileStore,
    events: &mpsc::Sender<RecordingLifecycleEvent>,
    counters: &mut SegmentRecorderCounters,
) -> Result<(), SegmentRecorderError> {
    let finalizing = lifecycle
        .segment
        .begin_finalization(ended_at)
        .map_err(SegmentRecorderError::Transition)?;
    let stored = match store.finalize(lifecycle.writer).await {
        Ok(stored) => stored,
        Err(error) => {
            let failed = finalizing.fail(SegmentFailure::Storage);
            tracing::error!(path = %lifecycle.paths.partial().display(), error = %error, "recording segment persistence failed");
            publish_event(events, RecordingLifecycleEvent::Failed(failed)).await?;
            counters.failed = counters.failed.saturating_add(1);
            return Err(SegmentRecorderError::Storage(error));
        }
    };
    let dimensions = VideoDimensions::new(stored.width(), stored.height())
        .map_err(SegmentRecorderError::Dimensions)?;
    let evidence =
        DurableFileEvidence::new(stored.path().to_path_buf(), stored.size_bytes(), dimensions)
            .map_err(SegmentRecorderError::Evidence)?;
    let completed = finalizing
        .complete(evidence)
        .map_err(SegmentRecorderError::Transition)?;
    counters.persisted_bytes = counters.persisted_bytes.saturating_add(stored.size_bytes());
    tracing::info!(
        path = %stored.path().display(),
        size_bytes = stored.size_bytes(),
        width = stored.width(),
        height = stored.height(),
        "recording segment completed"
    );
    publish_event(events, RecordingLifecycleEvent::Completed(completed)).await?;
    counters.completed = counters.completed.saturating_add(1);
    Ok(())
}

async fn publish_event(
    events: &mpsc::Sender<RecordingLifecycleEvent>,
    event: RecordingLifecycleEvent,
) -> Result<(), SegmentRecorderError> {
    events
        .send(event)
        .await
        .map_err(|_| SegmentRecorderError::MetadataChannelClosed)
}

#[derive(Debug)]
pub enum SegmentRecorderError {
    Clock(ClockMapError),
    Muxer(Fmp4SinkError),
    Path(SegmentPathAtError),
    Transition(SegmentTransitionError),
    Storage(SegmentStoreError),
    Dimensions(VideoDimensionsError),
    Evidence(DurableFileEvidenceError),
    MetadataChannelClosed,
    MissingLifecycle,
    IdentityExhausted,
    IdentityInvalid,
    WallTimeBeforeUnixEpoch,
}

impl fmt::Display for SegmentRecorderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "segment recorder runtime failed: {self:?}")
    }
}

impl Error for SegmentRecorderError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Clock(error) => Some(error),
            Self::Muxer(error) => Some(error),
            Self::Path(error) => Some(error),
            Self::Transition(error) => Some(error),
            Self::Storage(error) => Some(error),
            Self::Dimensions(error) => Some(error),
            Self::Evidence(error) => Some(error),
            _ => None,
        }
    }
}
