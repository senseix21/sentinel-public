//! Encoded-media recorder input boundary.

use std::sync::Arc;

use surveillance_media::{
    distribution::{MediaSubscriber, ReceiveOutcome},
    packet::H264AccessUnit,
};

/// The narrow substitution boundary implemented by the future fragmented-MP4
/// muxer and by deterministic tests.
pub trait H264AccessUnitSink {
    /// Sink-specific error retained at the recorder supervision boundary.
    type Error;

    /// Accepts one complete H.264 presentation unit in source order.
    ///
    /// Implementations must return only after they have either accepted the
    /// unit into their own explicitly bounded state or rejected it.
    ///
    /// # Errors
    ///
    /// Returns the sink-specific error when the unit was not accepted. The
    /// recorder consumer then returns ownership of that unit to its supervisor.
    fn write_access_unit(&mut self, access_unit: &H264AccessUnit) -> Result<(), Self::Error>;
}

/// Observable recorder-consumer counters.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RecorderConsumerMetrics {
    /// Access units successfully accepted by the sink.
    pub written: u64,
    /// Access units rejected by the sink and returned to the supervisor.
    pub sink_failures: u64,
}

/// Result of one nonblocking recorder-consumer step.
#[derive(Debug)]
pub enum RecorderConsumeOutcome<E> {
    /// One access unit was accepted by the sink.
    Written,
    /// No access unit is currently available; the producer remains open.
    Empty,
    /// The producer is closed and its retained queue has been drained.
    InputClosed,
    /// The sink rejected an access unit. Ownership is returned so the
    /// supervisor can apply an explicit retry or terminal-failure policy.
    SinkFailed {
        access_unit: Arc<H264AccessUnit>,
        error: E,
    },
}

/// Connects the recorder's media subscription to one owned H.264 sink.
pub struct RecorderConsumer<S> {
    subscription: MediaSubscriber<H264AccessUnit>,
    sink: S,
    metrics: RecorderConsumerMetrics,
}

impl<S> RecorderConsumer<S>
where
    S: H264AccessUnitSink,
{
    /// Creates a consumer with singular ownership of its queue and sink.
    #[must_use]
    pub const fn new(subscription: MediaSubscriber<H264AccessUnit>, sink: S) -> Self {
        Self {
            subscription,
            sink,
            metrics: RecorderConsumerMetrics {
                written: 0,
                sink_failures: 0,
            },
        }
    }

    /// Attempts to consume at most one access unit without waiting.
    pub fn try_consume(&mut self) -> RecorderConsumeOutcome<S::Error> {
        match self.subscription.try_receive() {
            ReceiveOutcome::Item(access_unit) => self.write(access_unit),
            ReceiveOutcome::Empty => RecorderConsumeOutcome::Empty,
            ReceiveOutcome::Closed => RecorderConsumeOutcome::InputClosed,
        }
    }

    /// Waits for and consumes at most one access unit, or observes input
    /// closure. Cancellation while waiting does not remove media from the
    /// bounded queue.
    pub async fn consume(&mut self) -> RecorderConsumeOutcome<S::Error> {
        match self.subscription.receive().await {
            ReceiveOutcome::Item(access_unit) => self.write(access_unit),
            ReceiveOutcome::Empty => RecorderConsumeOutcome::Empty,
            ReceiveOutcome::Closed => RecorderConsumeOutcome::InputClosed,
        }
    }

    fn write(&mut self, access_unit: Arc<H264AccessUnit>) -> RecorderConsumeOutcome<S::Error> {
        match self.sink.write_access_unit(&access_unit) {
            Ok(()) => {
                self.metrics.written = self.metrics.written.saturating_add(1);
                RecorderConsumeOutcome::Written
            }
            Err(error) => {
                self.metrics.sink_failures = self.metrics.sink_failures.saturating_add(1);
                RecorderConsumeOutcome::SinkFailed { access_unit, error }
            }
        }
    }

    /// Returns recorder-consumer counters.
    #[must_use]
    pub const fn metrics(&self) -> RecorderConsumerMetrics {
        self.metrics
    }

    /// Returns shared access to the owned sink for observation.
    #[must_use]
    pub const fn sink(&self) -> &S {
        &self.sink
    }

    /// Decomposes the consumer for orderly shutdown or adapter replacement.
    #[must_use]
    pub fn into_parts(self) -> (MediaSubscriber<H264AccessUnit>, S) {
        (self.subscription, self.sink)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        H264AccessUnitSink, RecorderConsumeOutcome, RecorderConsumer, RecorderConsumerMetrics,
    };
    use std::sync::Arc;
    use surveillance_media::{
        distribution::{MediaDistributor, PublishOutcome, QueueCapacity},
        packet::{H264AccessUnit, H264AccessUnitAssembler, H264NalUnit, MediaTimestamp},
    };

    #[derive(Debug, Default)]
    struct TestSink {
        timestamps: Vec<i64>,
        fail_at: Option<i64>,
    }

    impl H264AccessUnitSink for TestSink {
        type Error = &'static str;

        fn write_access_unit(&mut self, access_unit: &H264AccessUnit) -> Result<(), Self::Error> {
            let timestamp = access_unit.timestamp().ticks();
            if self.fail_at == Some(timestamp) {
                return Err("injected sink failure");
            }
            self.timestamps.push(timestamp);
            Ok(())
        }
    }

    fn capacity(value: usize) -> QueueCapacity {
        QueueCapacity::new(value).unwrap_or_else(|error| panic!("valid test capacity: {error}"))
    }

    fn access_unit(timestamp: i64) -> Arc<H264AccessUnit> {
        let timestamp = MediaTimestamp::new(timestamp, 90_000)
            .unwrap_or_else(|error| panic!("valid timestamp: {error}"));
        let nal = H264NalUnit::from_ebsp(Arc::from([0x65, 0x01]))
            .unwrap_or_else(|error| panic!("valid NAL: {error}"));
        let mut assembler = H264AccessUnitAssembler::new();
        Arc::new(
            assembler
                .push(timestamp, nal, true)
                .unwrap_or_else(|error| panic!("valid access unit: {error}"))
                .unwrap_or_else(|| panic!("marker completes access unit")),
        )
    }

    #[test]
    fn consumes_distributed_access_units_in_order() {
        let (distributor, subscriptions) = MediaDistributor::new(capacity(2), capacity(1));
        let mut recorder = RecorderConsumer::new(subscriptions.recorder, TestSink::default());

        assert_eq!(
            distributor.publish(access_unit(1)).recorder,
            PublishOutcome::Accepted
        );
        assert_eq!(
            distributor.publish(access_unit(2)).recorder,
            PublishOutcome::Accepted
        );
        assert!(matches!(
            recorder.try_consume(),
            RecorderConsumeOutcome::Written
        ));
        assert!(matches!(
            recorder.try_consume(),
            RecorderConsumeOutcome::Written
        ));
        assert!(matches!(
            recorder.try_consume(),
            RecorderConsumeOutcome::Empty
        ));
        assert_eq!(recorder.sink().timestamps, [1, 2]);
        assert_eq!(
            recorder.metrics(),
            RecorderConsumerMetrics {
                written: 2,
                sink_failures: 0
            }
        );
    }

    #[test]
    fn returns_rejected_access_unit_to_supervisor() {
        let (distributor, subscriptions) = MediaDistributor::new(capacity(2), capacity(1));
        let sink = TestSink {
            timestamps: Vec::new(),
            fail_at: Some(7),
        };
        let mut recorder = RecorderConsumer::new(subscriptions.recorder, sink);
        let _ = distributor.publish(access_unit(7));
        let _ = distributor.publish(access_unit(8));

        match recorder.try_consume() {
            RecorderConsumeOutcome::SinkFailed { access_unit, error } => {
                assert_eq!(access_unit.timestamp().ticks(), 7);
                assert_eq!(error, "injected sink failure");
            }
            _ => panic!("expected the injected sink failure"),
        }
        assert!(matches!(
            recorder.try_consume(),
            RecorderConsumeOutcome::Written
        ));
        assert_eq!(recorder.sink().timestamps, [8]);
        assert_eq!(recorder.metrics().sink_failures, 1);
    }

    #[test]
    fn drains_before_reporting_input_closed() {
        let (distributor, subscriptions) = MediaDistributor::new(capacity(1), capacity(1));
        let mut recorder = RecorderConsumer::new(subscriptions.recorder, TestSink::default());
        let _ = distributor.publish(access_unit(9));
        drop(distributor);

        assert!(matches!(
            recorder.try_consume(),
            RecorderConsumeOutcome::Written
        ));
        assert!(matches!(
            recorder.try_consume(),
            RecorderConsumeOutcome::InputClosed
        ));
    }
}
