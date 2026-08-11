//! Owned asynchronous recorder task lifecycle.

use std::{error::Error, fmt, sync::Arc};

use surveillance_media::packet::H264AccessUnit;
use tokio::{
    sync::watch,
    task::{JoinError, JoinHandle},
};

use crate::recorder::{H264AccessUnitSink, RecorderConsumeOutcome, RecorderConsumer};

/// Terminal result of an owned recorder task.
pub enum RecorderTaskExit<S, E> {
    /// The media producer closed and all retained access units were consumed.
    InputDrained { consumer: RecorderConsumer<S> },
    /// The owner explicitly cancelled the task. Retained queue state remains in
    /// the returned consumer for an explicit follow-up policy.
    Cancelled { consumer: RecorderConsumer<S> },
    /// The sink rejected one unit. That unit and the consumer are returned to
    /// the supervisor; no implicit retry or loss decision is made.
    SinkFailed {
        consumer: RecorderConsumer<S>,
        access_unit: Arc<H264AccessUnit>,
        error: E,
    },
}

/// Owns recorder cancellation and the sole join handle.
///
/// Dropping this owner aborts the task instead of detaching it. Orderly process
/// shutdown should close media production and call [`Self::join`] so queued
/// media drains before the task exits.
pub struct RecorderTask<S>
where
    S: H264AccessUnitSink,
{
    cancel: watch::Sender<bool>,
    join: Option<JoinHandle<RecorderTaskExit<S, S::Error>>>,
}

impl<S> RecorderTask<S>
where
    S: H264AccessUnitSink + Send + 'static,
    S::Error: Send + 'static,
{
    /// Spawns one task with singular ownership of the recorder consumer.
    #[must_use]
    pub fn spawn(consumer: RecorderConsumer<S>) -> Self {
        let (cancel, receiver) = watch::channel(false);
        let join = tokio::spawn(run(consumer, receiver));
        Self {
            cancel,
            join: Some(join),
        }
    }

    /// Requests cooperative cancellation without waiting for task termination.
    ///
    /// Cancellation does not promise queue drain. For graceful drain, close the
    /// upstream distributor and await [`Self::join`].
    pub fn request_cancellation(&self) {
        self.cancel.send_replace(true);
    }

    /// Requests cancellation and waits until resource ownership is returned.
    ///
    /// # Errors
    ///
    /// Returns [`RecorderTaskJoinError`] if the Tokio task panicked or was
    /// externally aborted.
    pub async fn cancel_and_join(
        mut self,
    ) -> Result<RecorderTaskExit<S, S::Error>, RecorderTaskJoinError> {
        self.request_cancellation();
        self.join_inner().await
    }

    /// Waits for input drain, cancellation, or sink failure.
    ///
    /// # Errors
    ///
    /// Returns [`RecorderTaskJoinError`] if the Tokio task panicked or was
    /// externally aborted.
    pub async fn join(mut self) -> Result<RecorderTaskExit<S, S::Error>, RecorderTaskJoinError> {
        self.join_inner().await
    }

    async fn join_inner(&mut self) -> Result<RecorderTaskExit<S, S::Error>, RecorderTaskJoinError> {
        let Some(join) = self.join.take() else {
            return Err(RecorderTaskJoinError::AlreadyJoined);
        };
        join.await.map_err(RecorderTaskJoinError::Task)
    }
}

impl<S> Drop for RecorderTask<S>
where
    S: H264AccessUnitSink,
{
    fn drop(&mut self) {
        if let Some(join) = &self.join {
            join.abort();
        }
    }
}

/// Failure to recover ownership from the spawned recorder task.
#[derive(Debug)]
pub enum RecorderTaskJoinError {
    AlreadyJoined,
    Task(JoinError),
}

impl fmt::Display for RecorderTaskJoinError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyJoined => formatter.write_str("recorder task was already joined"),
            Self::Task(error) => write!(formatter, "recorder task failed to join: {error}"),
        }
    }
}

impl Error for RecorderTaskJoinError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::AlreadyJoined => None,
            Self::Task(error) => Some(error),
        }
    }
}

async fn run<S>(
    mut consumer: RecorderConsumer<S>,
    mut cancel: watch::Receiver<bool>,
) -> RecorderTaskExit<S, S::Error>
where
    S: H264AccessUnitSink,
{
    loop {
        if *cancel.borrow() {
            return RecorderTaskExit::Cancelled { consumer };
        }

        tokio::select! {
            biased;
            changed = cancel.changed() => {
                if changed.is_err() || *cancel.borrow_and_update() {
                    return RecorderTaskExit::Cancelled { consumer };
                }
            }
            outcome = consumer.consume() => {
                match outcome {
                    RecorderConsumeOutcome::Written | RecorderConsumeOutcome::Empty => {}
                    RecorderConsumeOutcome::InputClosed => {
                        return RecorderTaskExit::InputDrained { consumer };
                    }
                    RecorderConsumeOutcome::SinkFailed { access_unit, error } => {
                        return RecorderTaskExit::SinkFailed {
                            consumer,
                            access_unit,
                            error,
                        };
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{RecorderTask, RecorderTaskExit};
    use crate::recorder::{H264AccessUnitSink, RecorderConsumer};
    use std::sync::Arc;
    use surveillance_media::{
        distribution::{MediaDistributor, QueueCapacity},
        packet::{H264AccessUnit, H264AccessUnitAssembler, H264NalUnit, MediaTimestamp},
    };

    #[derive(Default)]
    struct TestSink {
        timestamps: Vec<i64>,
        fail: bool,
    }

    impl H264AccessUnitSink for TestSink {
        type Error = &'static str;

        fn write_access_unit(&mut self, unit: &H264AccessUnit) -> Result<(), Self::Error> {
            if self.fail {
                return Err("injected task sink failure");
            }
            self.timestamps.push(unit.timestamp().ticks());
            Ok(())
        }
    }

    fn capacity(value: usize) -> QueueCapacity {
        QueueCapacity::new(value).unwrap_or_else(|error| panic!("valid capacity: {error}"))
    }

    fn access_unit(ticks: i64) -> Arc<H264AccessUnit> {
        let timestamp = MediaTimestamp::new(ticks, 90_000)
            .unwrap_or_else(|error| panic!("valid timestamp: {error}"));
        let nal = H264NalUnit::from_ebsp(Arc::from([0x65, 0x01]))
            .unwrap_or_else(|error| panic!("valid NAL: {error}"));
        let mut assembler = H264AccessUnitAssembler::new();
        Arc::new(
            assembler
                .push(timestamp, nal, true)
                .unwrap_or_else(|error| panic!("valid access unit: {error}"))
                .unwrap_or_else(|| panic!("marker completes unit")),
        )
    }

    #[tokio::test]
    async fn producer_close_wakes_task_drains_and_returns_ownership() {
        let (distributor, subscriptions) = MediaDistributor::new(capacity(2), capacity(1));
        let consumer = RecorderConsumer::new(subscriptions.recorder, TestSink::default());
        let task = RecorderTask::spawn(consumer);
        let _ = distributor.publish(access_unit(1));
        let _ = distributor.publish(access_unit(2));
        drop(distributor);

        let exit = task
            .join()
            .await
            .unwrap_or_else(|error| panic!("task joins: {error}"));
        match exit {
            RecorderTaskExit::InputDrained { consumer } => {
                assert_eq!(consumer.sink().timestamps, [1, 2]);
                assert_eq!(consumer.metrics().written, 2);
            }
            _ => panic!("expected drained input"),
        }
    }

    #[tokio::test]
    async fn cancellation_returns_consumer_without_detaching_task() {
        let (_distributor, subscriptions) =
            MediaDistributor::<H264AccessUnit>::new(capacity(1), capacity(1));
        let consumer = RecorderConsumer::new(subscriptions.recorder, TestSink::default());
        let task = RecorderTask::spawn(consumer);

        let exit = task
            .cancel_and_join()
            .await
            .unwrap_or_else(|error| panic!("task joins: {error}"));
        assert!(matches!(exit, RecorderTaskExit::Cancelled { .. }));
    }

    #[tokio::test]
    async fn sink_failure_returns_unit_error_and_consumer() {
        let (distributor, subscriptions) = MediaDistributor::new(capacity(1), capacity(1));
        let sink = TestSink {
            timestamps: Vec::new(),
            fail: true,
        };
        let consumer = RecorderConsumer::new(subscriptions.recorder, sink);
        let task = RecorderTask::spawn(consumer);
        let _ = distributor.publish(access_unit(7));

        let exit = task
            .join()
            .await
            .unwrap_or_else(|error| panic!("task joins: {error}"));
        match exit {
            RecorderTaskExit::SinkFailed {
                consumer,
                access_unit,
                error,
            } => {
                assert_eq!(access_unit.timestamp().ticks(), 7);
                assert_eq!(error, "injected task sink failure");
                assert_eq!(consumer.metrics().sink_failures, 1);
            }
            _ => panic!("expected sink failure"),
        }
    }
}
