//! Bounded application owner for recording metadata delivery.

use std::{
    error::Error,
    fmt,
    num::NonZeroUsize,
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use surveillance_database::{PostgresRecordingRepository, RepositoryErrorCategory};
use surveillance_recorder::{
    runtime::RecordingLifecycleEvent,
    segment::{RecordingFormat, SegmentFailure},
};
use surveillance_storage::metadata::{
    CompletedRecording, MetadataProvenance, NonPlayableRecording, RecordingContainer,
    RecordingEvidenceLocator, RecordingFailureCategory, RecordingLocator, RecordingStatus,
    VideoCodec,
};
use tokio::sync::{mpsc, watch};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MetadataWriterSnapshot {
    pub accepted: u64,
    pub rejected_full: u64,
    pub rejected_closed: u64,
    pub persisted: u64,
    pub confirmed: u64,
    pub retryable_failures: u64,
    pub permanent_failures: u64,
    pub queue_capacity: usize,
}

#[derive(Default)]
struct MetadataWriterMetrics {
    accepted: AtomicU64,
    rejected_full: AtomicU64,
    rejected_closed: AtomicU64,
    persisted: AtomicU64,
    confirmed: AtomicU64,
    retryable_failures: AtomicU64,
    permanent_failures: AtomicU64,
}

#[derive(Clone)]
pub struct MetadataWriterHandle {
    sender: mpsc::Sender<MetadataFact>,
    capacity: NonZeroUsize,
    metrics: Arc<MetadataWriterMetrics>,
}

impl MetadataWriterHandle {
    /// Converts a recorder lifecycle fact and admits it without waiting on the
    /// database. A full queue is explicit while the final file remains durable
    /// reconciliation evidence.
    ///
    /// # Errors
    /// Returns a typed conversion, saturation, or closure failure.
    pub fn try_submit(
        &self,
        event: RecordingLifecycleEvent,
        recording_root: &Path,
    ) -> Result<(), MetadataSubmitError> {
        let fact = match event {
            RecordingLifecycleEvent::Completed {
                camera_id,
                recording_id,
                segment,
            } => {
                let relative = segment
                    .path()
                    .strip_prefix(recording_root)
                    .map_err(|_| MetadataSubmitError::PathOutsideRoot)?;
                let locator = RecordingLocator::new(relative.to_path_buf())
                    .map_err(|_| MetadataSubmitError::InvalidLocator)?;
                let (container, codec) = match segment.format() {
                    RecordingFormat::FragmentedMp4H264 => {
                        (RecordingContainer::FragmentedMp4, VideoCodec::H264)
                    }
                };
                MetadataFact::Completed(
                    CompletedRecording::new(
                        recording_id,
                        camera_id,
                        segment.started_at(),
                        segment.ended_at(),
                        locator,
                        container,
                        codec,
                        segment.dimensions().width(),
                        segment.dimensions().height(),
                        segment.size_bytes(),
                        MetadataProvenance::Recorder,
                    )
                    .map_err(|_| MetadataSubmitError::InvalidCompletedEvidence)?,
                )
            }
            RecordingLifecycleEvent::Failed {
                camera_id,
                recording_id,
                segment,
            } => {
                let relative = segment
                    .partial_path()
                    .strip_prefix(recording_root)
                    .map_err(|_| MetadataSubmitError::PathOutsideRoot)?;
                let locator = RecordingEvidenceLocator::new(relative.to_path_buf())
                    .map_err(|_| MetadataSubmitError::InvalidLocator)?;
                let failure = match segment.failure() {
                    SegmentFailure::Muxer => RecordingFailureCategory::Muxer,
                    SegmentFailure::Storage => RecordingFailureCategory::Storage,
                    SegmentFailure::Validation => RecordingFailureCategory::Validation,
                    SegmentFailure::Shutdown => RecordingFailureCategory::Shutdown,
                };
                MetadataFact::NonPlayable(
                    NonPlayableRecording::new(
                        recording_id,
                        camera_id,
                        segment.started_at(),
                        segment.ended_at(),
                        locator,
                        RecordingStatus::Failed,
                        failure,
                        MetadataProvenance::Recorder,
                    )
                    .map_err(|_| MetadataSubmitError::InvalidNonPlayableEvidence)?,
                )
            }
        };
        self.sender.try_send(fact).map_or_else(
            |error| match error {
                mpsc::error::TrySendError::Full(_) => {
                    self.metrics.rejected_full.fetch_add(1, Ordering::Relaxed);
                    Err(MetadataSubmitError::Full)
                }
                mpsc::error::TrySendError::Closed(_) => {
                    self.metrics.rejected_closed.fetch_add(1, Ordering::Relaxed);
                    Err(MetadataSubmitError::Closed)
                }
            },
            |()| {
                self.metrics.accepted.fetch_add(1, Ordering::Relaxed);
                Ok(())
            },
        )
    }

    #[must_use]
    pub fn snapshot(&self) -> MetadataWriterSnapshot {
        MetadataWriterSnapshot {
            accepted: self.metrics.accepted.load(Ordering::Relaxed),
            rejected_full: self.metrics.rejected_full.load(Ordering::Relaxed),
            rejected_closed: self.metrics.rejected_closed.load(Ordering::Relaxed),
            persisted: self.metrics.persisted.load(Ordering::Relaxed),
            confirmed: self.metrics.confirmed.load(Ordering::Relaxed),
            retryable_failures: self.metrics.retryable_failures.load(Ordering::Relaxed),
            permanent_failures: self.metrics.permanent_failures.load(Ordering::Relaxed),
            queue_capacity: self.capacity.get(),
        }
    }
}

pub struct MetadataWriter {
    repository: PostgresRecordingRepository,
    receiver: mpsc::Receiver<MetadataFact>,
    metrics: Arc<MetadataWriterMetrics>,
    retry_base: Duration,
    retry_maximum: Duration,
}

impl MetadataWriter {
    #[must_use]
    pub fn new(
        repository: PostgresRecordingRepository,
        capacity: NonZeroUsize,
        retry_base: Duration,
        retry_maximum: Duration,
    ) -> (Self, MetadataWriterHandle) {
        let (sender, receiver) = mpsc::channel(capacity.get());
        let metrics = Arc::new(MetadataWriterMetrics::default());
        (
            Self {
                repository,
                receiver,
                metrics: Arc::clone(&metrics),
                retry_base,
                retry_maximum,
            },
            MetadataWriterHandle {
                sender,
                capacity,
                metrics,
            },
        )
    }

    pub async fn run(mut self, mut shutdown: watch::Receiver<bool>) {
        loop {
            let fact = tokio::select! {
                biased;
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow_and_update() { return; }
                    continue;
                }
                fact = self.receiver.recv() => fact,
            };
            let Some(fact) = fact else {
                return;
            };
            let mut delay = self.retry_base;
            loop {
                let result = tokio::select! {
                    biased;
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow_and_update() { return; }
                        continue;
                    }
                    result = persist_fact(&self.repository, &fact) => result,
                };
                match result {
                    Ok(surveillance_database::PersistCompletedOutcome::Inserted) => {
                        self.metrics.persisted.fetch_add(1, Ordering::Relaxed);
                        break;
                    }
                    Ok(surveillance_database::PersistCompletedOutcome::Confirmed) => {
                        self.metrics.confirmed.fetch_add(1, Ordering::Relaxed);
                        break;
                    }
                    Err(error) if error.retryable() => {
                        self.metrics
                            .retryable_failures
                            .fetch_add(1, Ordering::Relaxed);
                        tracing::warn!(category = ?error.category(), "recording metadata persistence unavailable; durable file remains reconciliation evidence");
                        tokio::select! {
                            changed = shutdown.changed() => {
                                if changed.is_err() || *shutdown.borrow_and_update() { return; }
                            }
                            () = tokio::time::sleep(delay) => {}
                        }
                        delay = delay.saturating_mul(2).min(self.retry_maximum);
                    }
                    Err(error) => {
                        self.metrics
                            .permanent_failures
                            .fetch_add(1, Ordering::Relaxed);
                        tracing::error!(category = ?error.category(), "recording metadata persistence rejected immutable evidence");
                        if error.category() == RepositoryErrorCategory::Conflict {
                            break;
                        }
                        break;
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetadataSubmitError {
    PathOutsideRoot,
    InvalidLocator,
    InvalidCompletedEvidence,
    InvalidNonPlayableEvidence,
    Full,
    Closed,
}

impl fmt::Display for MetadataSubmitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "metadata fact admission failed: {self:?}")
    }
}

impl Error for MetadataSubmitError {}

enum MetadataFact {
    Completed(CompletedRecording),
    NonPlayable(NonPlayableRecording),
}

async fn persist_fact(
    repository: &PostgresRecordingRepository,
    fact: &MetadataFact,
) -> Result<surveillance_database::PersistCompletedOutcome, surveillance_database::RepositoryError>
{
    match fact {
        MetadataFact::Completed(completed) => {
            repository.insert_or_confirm_completed(completed).await
        }
        MetadataFact::NonPlayable(evidence) => {
            repository.insert_or_confirm_non_playable(evidence).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;
    use std::{path::PathBuf, time::SystemTime};
    use surveillance_common::types::{CameraId, RecordingId};
    use surveillance_recorder::segment::{
        DurableFileEvidence, OpenSegment, RecordingFormat, VideoDimensions,
    };
    use surveillance_storage::filesystem::SegmentPaths;

    fn event(root: &Path, id: &str) -> RecordingLifecycleEvent {
        let partial = root.join("camera-1").join(format!("{id}.partial.mp4"));
        let paths = SegmentPaths::from_partial(partial)
            .unwrap_or_else(|error| panic!("valid paths: {error}"));
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
        let segment = OpenSegment::new(paths.clone(), start, RecordingFormat::FragmentedMp4H264)
            .begin_finalization(start + Duration::from_secs(1))
            .unwrap_or_else(|error| panic!("valid finalization: {error}"))
            .complete(
                DurableFileEvidence::new(
                    paths.durable().to_path_buf(),
                    1,
                    VideoDimensions::new(1, 1)
                        .unwrap_or_else(|error| panic!("valid dimensions: {error}")),
                )
                .unwrap_or_else(|error| panic!("valid evidence: {error}")),
            )
            .unwrap_or_else(|error| panic!("valid completion: {error}"));
        RecordingLifecycleEvent::Completed {
            camera_id: CameraId::new("camera-1")
                .unwrap_or_else(|error| panic!("valid camera: {error}")),
            recording_id: RecordingId::new(id)
                .unwrap_or_else(|error| panic!("valid recording: {error}")),
            segment,
        }
    }

    #[tokio::test]
    async fn admission_is_bounded_and_reports_saturation() {
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://localhost/test")
            .unwrap_or_else(|error| panic!("valid lazy pool: {error}"));
        let (_writer, handle) = MetadataWriter::new(
            PostgresRecordingRepository::from_pool(pool),
            NonZeroUsize::new(1).unwrap_or(NonZeroUsize::MIN),
            Duration::from_millis(1),
            Duration::from_secs(1),
        );
        let root = PathBuf::from("recordings");
        handle
            .try_submit(event(&root, "recording-1"), &root)
            .unwrap_or_else(|error| panic!("first fact admitted: {error}"));
        assert_eq!(
            handle.try_submit(event(&root, "recording-2"), &root),
            Err(MetadataSubmitError::Full)
        );
        let snapshot = handle.snapshot();
        assert_eq!(snapshot.accepted, 1);
        assert_eq!(snapshot.rejected_full, 1);
        assert_eq!(snapshot.queue_capacity, 1);
    }

    #[tokio::test]
    async fn completed_fact_cannot_escape_recording_root() {
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://localhost/test")
            .unwrap_or_else(|error| panic!("valid lazy pool: {error}"));
        let (_writer, handle) = MetadataWriter::new(
            PostgresRecordingRepository::from_pool(pool),
            NonZeroUsize::new(1).unwrap_or(NonZeroUsize::MIN),
            Duration::from_millis(1),
            Duration::from_secs(1),
        );
        assert_eq!(
            handle.try_submit(
                event(Path::new("other"), "recording-1"),
                Path::new("recordings")
            ),
            Err(MetadataSubmitError::PathOutsideRoot)
        );
    }
}
