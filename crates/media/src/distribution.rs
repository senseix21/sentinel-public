//! Bounded media subscription and overload semantics.
//!
//! The distributor deliberately gives each consumer an independent queue. A
//! slow live-view consumer therefore cannot consume recorder capacity or block
//! recorder delivery. This first contract is nonblocking; an async adapter can
//! add wake-ups without changing the overload rules defined here.

use std::{
    collections::VecDeque,
    error::Error,
    fmt,
    sync::{Arc, Mutex, MutexGuard},
};
use tokio::sync::watch;

/// A validated, nonzero queue capacity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueueCapacity(usize);

impl QueueCapacity {
    /// Validates a queue capacity.
    ///
    /// # Errors
    ///
    /// Returns [`QueueCapacityError::Zero`] when `value` is zero.
    pub fn new(value: usize) -> Result<Self, QueueCapacityError> {
        if value == 0 {
            return Err(QueueCapacityError::Zero);
        }

        Ok(Self(value))
    }

    /// Returns the number of media items the queue can retain.
    #[must_use]
    pub const fn get(self) -> usize {
        self.0
    }
}

/// An invalid queue-capacity configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueCapacityError {
    /// A bounded queue cannot have zero capacity.
    Zero,
}

impl fmt::Display for QueueCapacityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("media queue capacity must be greater than zero")
    }
}

impl Error for QueueCapacityError {}

/// Media whose retention under overload depends on what can be decoded without it.
///
/// A freshness-oriented queue that evicts purely by age can strand the items it
/// keeps: an inter-coded picture whose reference was discarded is not merely
/// late, it is undecodable, and a decoder handed one emits whatever its frame
/// buffer already held. The queue therefore has to know which items begin a run
/// that a consumer can decode on its own.
pub trait DecodableRunStart {
    /// Whether this item can begin a decodable run without any earlier item.
    fn starts_decodable_run(&self) -> bool;
}

/// The result of publishing to one consumer queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublishOutcome {
    /// The item was appended without discarding another item.
    Accepted,
    /// The recorder queue retained its existing items and rejected this item.
    Full,
    /// The live queue discarded its oldest item and accepted the new item.
    ReplacedOldest,
    /// The live queue dropped this item because delivering it would have
    /// handed the consumer a picture whose reference it never received.
    DroppedUndecodable,
    /// The consumer has been dropped and cannot receive more items.
    Closed,
}

/// Independent outcomes for all media consumers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PublishReport {
    /// Recorder-queue outcome.
    pub recorder: PublishOutcome,
    /// Live-view-queue outcome.
    pub live: PublishOutcome,
    /// Detection-queue outcome. A closed outcome means detection is disabled.
    pub detection: PublishOutcome,
}

/// The result of a nonblocking receive attempt.
#[derive(Debug)]
pub enum ReceiveOutcome<T> {
    /// The oldest retained item.
    Item(Arc<T>),
    /// The producer remains open but no item is currently available.
    Empty,
    /// The producer is gone and no retained items remain.
    Closed,
}

/// A point-in-time view of one queue's bounded state and counters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueueMetrics {
    /// Configured maximum retained items.
    pub capacity: usize,
    /// Items retained at snapshot time.
    pub depth: usize,
    /// Items accepted into this queue.
    pub accepted: u64,
    /// Items returned to the consumer.
    pub received: u64,
    /// New items rejected to preserve recorder backlog.
    pub rejected_full: u64,
    /// Old items evicted to preserve live freshness.
    pub dropped_oldest: u64,
    /// Items withheld because the consumer could not have decoded them.
    ///
    /// A sustained nonzero rate here means the live consumer is slower than the
    /// source: the queue is holding a decodable backlog and waiting for the next
    /// run start rather than emitting pictures that would decode to nothing.
    pub dropped_undecodable: u64,
    /// Retained items discarded when the consumer closed.
    pub dropped_on_close: u64,
    /// Whether the consumer can still receive publications.
    pub consumer_open: bool,
    /// Whether the distributor can still publish.
    pub producer_open: bool,
}

/// One recorder subscription and one live-view subscription.
pub struct MediaSubscriptions<T> {
    /// Loss-intolerant recorder subscription.
    pub recorder: MediaSubscriber<T>,
    /// Freshness-oriented live-view subscription.
    pub live: MediaSubscriber<T>,
    /// Freshness-oriented detection subscription.
    pub detection: MediaSubscriber<T>,
}

/// Publishes immutable media to independently bounded consumer queues.
pub struct MediaDistributor<T> {
    recorder: QueueSender<T>,
    live: QueueSender<T>,
    detection: QueueSender<T>,
}

/// Read-only queue telemetry which neither retains media outside the queues nor
/// changes producer/consumer lifetime.
pub struct MediaMetricsObserver<T> {
    recorder: Arc<Mutex<QueueState<T>>>,
    live: Arc<Mutex<QueueState<T>>>,
    detection: Arc<Mutex<QueueState<T>>>,
}

impl<T> Clone for MediaMetricsObserver<T> {
    fn clone(&self) -> Self {
        Self {
            recorder: Arc::clone(&self.recorder),
            live: Arc::clone(&self.live),
            detection: Arc::clone(&self.detection),
        }
    }
}

impl<T> MediaMetricsObserver<T> {
    #[must_use]
    pub fn recorder(&self) -> QueueMetrics {
        QueueState::metrics(&lock_recover(&self.recorder))
    }

    #[must_use]
    pub fn live(&self) -> QueueMetrics {
        QueueState::metrics(&lock_recover(&self.live))
    }

    #[must_use]
    pub fn detection(&self) -> QueueMetrics {
        QueueState::metrics(&lock_recover(&self.detection))
    }
}

impl<T> MediaDistributor<T> {
    /// Creates a distributor and its single-owner consumer subscriptions.
    #[must_use]
    pub fn new(
        recorder_capacity: QueueCapacity,
        live_capacity: QueueCapacity,
    ) -> (Self, MediaSubscriptions<T>) {
        let detection_capacity = QueueCapacity(1);
        Self::new_with_detection(recorder_capacity, live_capacity, detection_capacity)
    }

    /// Creates a distributor with an independent freshness-oriented detection queue.
    #[must_use]
    pub fn new_with_detection(
        recorder_capacity: QueueCapacity,
        live_capacity: QueueCapacity,
        detection_capacity: QueueCapacity,
    ) -> (Self, MediaSubscriptions<T>) {
        let (recorder, recorder_subscription, recorder_signal) =
            QueueSender::new(recorder_capacity);
        let (live, live_subscription, live_signal) = QueueSender::new(live_capacity);
        let (detection, detection_subscription, detection_signal) =
            QueueSender::new(detection_capacity);

        (
            Self {
                recorder,
                live,
                detection,
            },
            MediaSubscriptions {
                recorder: MediaSubscriber::new(recorder_subscription, recorder_signal),
                live: MediaSubscriber::new(live_subscription, live_signal),
                detection: MediaSubscriber::new(detection_subscription, detection_signal),
            },
        )
    }

    /// Attempts delivery to every consumer without waiting for any of them.
    ///
    /// Recorder overflow preserves the existing backlog and returns
    /// [`PublishOutcome::Full`]. Live overflow prefers freshness but will not
    /// break the consumer's decodable run: it evicts to the newest run start
    /// where it can, returning [`PublishOutcome::ReplacedOldest`], and
    /// otherwise withholds the item as [`PublishOutcome::DroppedUndecodable`]
    /// until the next run start arrives. Detection keeps plain age-ordered
    /// eviction; it owns its own sampling policy.
    pub fn publish(&self, item: Arc<T>) -> PublishReport
    where
        T: DecodableRunStart,
    {
        let recorder = self.recorder.send_preserving_backlog(Arc::clone(&item));
        let live = self.live.send_latest_decodable(Arc::clone(&item));
        let detection = self.detection.send_latest(item);

        PublishReport {
            recorder,
            live,
            detection,
        }
    }

    /// Creates a non-owning, read-only view of queue telemetry.
    #[must_use]
    pub fn metrics_observer(&self) -> MediaMetricsObserver<T> {
        MediaMetricsObserver {
            recorder: Arc::clone(&self.recorder.queue),
            live: Arc::clone(&self.live.queue),
            detection: Arc::clone(&self.detection.queue),
        }
    }

    /// Returns recorder-queue metrics.
    #[must_use]
    pub fn recorder_metrics(&self) -> QueueMetrics {
        self.recorder.metrics()
    }

    /// Returns live-view-queue metrics.
    #[must_use]
    pub fn live_metrics(&self) -> QueueMetrics {
        self.live.metrics()
    }

    /// Returns detection-queue metrics.
    #[must_use]
    pub fn detection_metrics(&self) -> QueueMetrics {
        self.detection.metrics()
    }
}

impl<T> Drop for MediaDistributor<T> {
    fn drop(&mut self) {
        self.recorder.close_producer();
        self.live.close_producer();
        self.detection.close_producer();
    }
}

/// The single consumer for one distributor queue.
pub struct MediaSubscriber<T> {
    queue: Arc<Mutex<QueueState<T>>>,
    signal: watch::Receiver<u64>,
}

impl<T> MediaSubscriber<T> {
    fn new(queue: Arc<Mutex<QueueState<T>>>, signal: watch::Receiver<u64>) -> Self {
        Self { queue, signal }
    }

    /// Returns the oldest item immediately, if one is available.
    #[must_use]
    pub fn try_receive(&self) -> ReceiveOutcome<T> {
        let mut state = lock_recover(&self.queue);
        if let Some(item) = state.items.pop_front() {
            state.received = state.received.saturating_add(1);
            return ReceiveOutcome::Item(item);
        }

        if state.producer_open {
            ReceiveOutcome::Empty
        } else {
            ReceiveOutcome::Closed
        }
    }

    /// Waits until an item is available or the producer is closed.
    ///
    /// The watch channel carries only a generation counter; media remains in
    /// the explicitly bounded queue. Checking the queue before `changed()` and
    /// watch's retained generation prevent a publication between those steps
    /// from becoming a lost wake-up.
    pub async fn receive(&mut self) -> ReceiveOutcome<T> {
        loop {
            match self.try_receive() {
                ReceiveOutcome::Empty => {}
                outcome => return outcome,
            }

            if self.signal.changed().await.is_err() {
                return self.try_receive();
            }
        }
    }

    /// Returns this queue's metrics.
    #[must_use]
    pub fn metrics(&self) -> QueueMetrics {
        QueueState::metrics(&lock_recover(&self.queue))
    }
}

impl<T> Drop for MediaSubscriber<T> {
    fn drop(&mut self) {
        let mut state = lock_recover(&self.queue);
        state.consumer_open = false;
        state.dropped_on_close = state
            .dropped_on_close
            .saturating_add(state.items.len() as u64);
        state.items.clear();
    }
}

struct QueueSender<T> {
    queue: Arc<Mutex<QueueState<T>>>,
    signal: watch::Sender<u64>,
}

impl<T> QueueSender<T> {
    fn new(capacity: QueueCapacity) -> (Self, Arc<Mutex<QueueState<T>>>, watch::Receiver<u64>) {
        let queue = Arc::new(Mutex::new(QueueState::new(capacity)));
        let (signal, receiver) = watch::channel(0);
        (
            Self {
                queue: Arc::clone(&queue),
                signal,
            },
            queue,
            receiver,
        )
    }

    fn send_preserving_backlog(&self, item: Arc<T>) -> PublishOutcome {
        let mut state = lock_recover(&self.queue);
        if !state.consumer_open {
            return PublishOutcome::Closed;
        }
        if state.items.len() == state.capacity {
            state.rejected_full = state.rejected_full.saturating_add(1);
            return PublishOutcome::Full;
        }

        state.items.push_back(item);
        state.accepted = state.accepted.saturating_add(1);
        drop(state);
        self.signal_change();
        PublishOutcome::Accepted
    }

    fn send_latest(&self, item: Arc<T>) -> PublishOutcome {
        let mut state = lock_recover(&self.queue);
        if !state.consumer_open {
            return PublishOutcome::Closed;
        }

        let replaced = if state.items.len() == state.capacity {
            let _ = state.items.pop_front();
            state.dropped_oldest = state.dropped_oldest.saturating_add(1);
            true
        } else {
            false
        };
        state.items.push_back(item);
        state.accepted = state.accepted.saturating_add(1);
        drop(state);
        self.signal_change();

        if replaced {
            PublishOutcome::ReplacedOldest
        } else {
            PublishOutcome::Accepted
        }
    }

    /// Freshness-oriented delivery that keeps what it delivers decodable.
    ///
    /// Age-ordered eviction alone is unsound for a consumer that decodes: it
    /// discards the reference and keeps the pictures that depend on it. This
    /// preserves one invariant instead — every item handed to the consumer is
    /// either a run start, or immediately follows an item the consumer also
    /// received. Freshness is still preferred wherever it can be had without
    /// breaking that invariant.
    fn send_latest_decodable(&self, item: Arc<T>) -> PublishOutcome
    where
        T: DecodableRunStart,
    {
        let mut state = lock_recover(&self.queue);
        if !state.consumer_open {
            return PublishOutcome::Closed;
        }

        let starts_run = item.starts_decodable_run();
        let full = state.items.len() >= state.capacity;

        if !starts_run && (state.run_broken || full) {
            // Either the run is already broken, or evicting to make room would
            // break it. Hold the decodable backlog and resynchronise at the
            // next run start rather than emit pictures that decode to nothing.
            state.run_broken = true;
            state.dropped_undecodable = state.dropped_undecodable.saturating_add(1);
            return PublishOutcome::DroppedUndecodable;
        }

        if full {
            // `item` starts its own run, so the queue may be shortened. Evict
            // the smallest prefix that still leaves a run start at the head.
            let _ = state.items.pop_front();
            state.dropped_oldest = state.dropped_oldest.saturating_add(1);
            while state
                .items
                .front()
                .is_some_and(|head| !head.starts_decodable_run())
            {
                let _ = state.items.pop_front();
                state.dropped_oldest = state.dropped_oldest.saturating_add(1);
            }
        }

        if starts_run {
            state.run_broken = false;
        }
        state.items.push_back(item);
        state.accepted = state.accepted.saturating_add(1);
        drop(state);
        self.signal_change();

        if full {
            PublishOutcome::ReplacedOldest
        } else {
            PublishOutcome::Accepted
        }
    }

    fn metrics(&self) -> QueueMetrics {
        QueueState::metrics(&lock_recover(&self.queue))
    }

    fn close_producer(&self) {
        lock_recover(&self.queue).producer_open = false;
        self.signal_change();
    }

    fn signal_change(&self) {
        self.signal
            .send_modify(|generation| *generation = generation.saturating_add(1));
    }
}

struct QueueState<T> {
    items: VecDeque<Arc<T>>,
    capacity: usize,
    accepted: u64,
    received: u64,
    rejected_full: u64,
    dropped_oldest: u64,
    dropped_undecodable: u64,
    dropped_on_close: u64,
    consumer_open: bool,
    producer_open: bool,
    /// Set when an eviction would have left a gap in the delivered run. While
    /// it is set the queue accepts only items that start their own run, so a
    /// consumer never receives a picture whose predecessor it never saw.
    /// Starts set: nothing has been delivered yet, so no run is established.
    run_broken: bool,
}

impl<T> QueueState<T> {
    fn new(capacity: QueueCapacity) -> Self {
        Self {
            items: VecDeque::with_capacity(capacity.get()),
            capacity: capacity.get(),
            accepted: 0,
            received: 0,
            rejected_full: 0,
            dropped_oldest: 0,
            dropped_undecodable: 0,
            dropped_on_close: 0,
            consumer_open: true,
            producer_open: true,
            run_broken: true,
        }
    }

    fn metrics(&self) -> QueueMetrics {
        QueueMetrics {
            capacity: self.capacity,
            depth: self.items.len(),
            accepted: self.accepted,
            received: self.received,
            rejected_full: self.rejected_full,
            dropped_oldest: self.dropped_oldest,
            dropped_undecodable: self.dropped_undecodable,
            dropped_on_close: self.dropped_on_close,
            consumer_open: self.consumer_open,
            producer_open: self.producer_open,
        }
    }
}

fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DecodableRunStart, MediaDistributor, PublishOutcome, QueueCapacity, QueueCapacityError,
        ReceiveOutcome,
    };
    use std::sync::Arc;

    /// The overload tests below are about capacity and ownership, not about
    /// codec structure, so their items are all independently decodable. That
    /// keeps them exercising exactly the age-ordered behaviour they were
    /// written for; the decodability rule is tested separately against items
    /// that are not.
    impl DecodableRunStart for u64 {
        fn starts_decodable_run(&self) -> bool {
            true
        }
    }

    /// An item that carries whether a consumer could decode it on its own.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct Coded {
        id: u64,
        run_start: bool,
    }

    impl Coded {
        const fn start(id: u64) -> Self {
            Self {
                id,
                run_start: true,
            }
        }
        const fn delta(id: u64) -> Self {
            Self {
                id,
                run_start: false,
            }
        }
    }

    impl DecodableRunStart for Coded {
        fn starts_decodable_run(&self) -> bool {
            self.run_start
        }
    }

    fn drain(subscriber: &super::MediaSubscriber<Coded>) -> Vec<Coded> {
        let mut received = Vec::new();
        loop {
            match subscriber.try_receive() {
                ReceiveOutcome::Item(item) => received.push(*item),
                ReceiveOutcome::Empty | ReceiveOutcome::Closed => return received,
            }
        }
    }

    #[test]
    fn the_live_queue_never_delivers_a_picture_whose_reference_it_dropped() {
        let (distributor, subscriptions) = MediaDistributor::<Coded>::new(capacity(8), capacity(4));

        // Deltas before any run start cannot be decoded by a joining consumer.
        assert_eq!(
            distributor.publish(Arc::new(Coded::delta(0))).live,
            PublishOutcome::DroppedUndecodable
        );

        assert_eq!(
            distributor.publish(Arc::new(Coded::start(1))).live,
            PublishOutcome::Accepted
        );
        for id in 2..=4 {
            assert_eq!(
                distributor.publish(Arc::new(Coded::delta(id))).live,
                PublishOutcome::Accepted
            );
        }

        // The queue is now full of one contiguous run. Evicting to make room
        // would strand everything that follows the evicted item, so the new
        // delta is withheld instead.
        for id in 5..=7 {
            assert_eq!(
                distributor.publish(Arc::new(Coded::delta(id))).live,
                PublishOutcome::DroppedUndecodable
            );
        }

        // A run start resynchronises: it needs nothing earlier, so the stale
        // run is evicted for it.
        assert_eq!(
            distributor.publish(Arc::new(Coded::start(8))).live,
            PublishOutcome::ReplacedOldest
        );
        assert_eq!(
            distributor.publish(Arc::new(Coded::delta(9))).live,
            PublishOutcome::Accepted
        );

        let delivered = drain(&subscriptions.live);
        assert_eq!(
            delivered,
            vec![Coded::start(8), Coded::delta(9)],
            "the consumer receives a contiguous run beginning at a run start"
        );

        let metrics = subscriptions.live.metrics();
        assert_eq!(metrics.dropped_undecodable, 4);
        assert_eq!(metrics.dropped_oldest, 4);
    }

    #[test]
    fn eviction_stops_at_the_newest_run_start_rather_than_dropping_one_item() {
        let (distributor, subscriptions) = MediaDistributor::<Coded>::new(capacity(8), capacity(4));

        for item in [
            Coded::start(1),
            Coded::delta(2),
            Coded::start(3),
            Coded::delta(4),
        ] {
            assert_eq!(
                distributor.publish(Arc::new(item)).live,
                PublishOutcome::Accepted
            );
        }

        // Full. The incoming run start makes room by discarding the prefix up
        // to the newest retained run start, not just the single oldest item,
        // because item 2 cannot be decoded once item 1 is gone.
        assert_eq!(
            distributor.publish(Arc::new(Coded::start(5))).live,
            PublishOutcome::ReplacedOldest
        );

        assert_eq!(
            drain(&subscriptions.live),
            vec![Coded::start(3), Coded::delta(4), Coded::start(5)]
        );
        assert_eq!(subscriptions.live.metrics().dropped_oldest, 2);
    }

    fn capacity(value: usize) -> QueueCapacity {
        QueueCapacity::new(value).unwrap_or_else(|error| panic!("valid test capacity: {error}"))
    }

    fn receive_value(subscriber: &super::MediaSubscriber<u64>) -> u64 {
        match subscriber.try_receive() {
            ReceiveOutcome::Item(value) => *value,
            ReceiveOutcome::Empty => panic!("expected an item, queue was empty"),
            ReceiveOutcome::Closed => panic!("expected an item, queue was closed"),
        }
    }

    #[test]
    fn rejects_zero_capacity() {
        assert_eq!(QueueCapacity::new(0), Err(QueueCapacityError::Zero));
    }

    #[test]
    fn stalled_live_consumer_drops_only_its_oldest_items() {
        let (distributor, subscriptions) = MediaDistributor::new(capacity(1), capacity(2));

        for value in 1..=5 {
            let report = distributor.publish(Arc::new(value));
            assert_eq!(report.recorder, PublishOutcome::Accepted);
            assert_eq!(receive_value(&subscriptions.recorder), value);
        }

        assert_eq!(receive_value(&subscriptions.live), 4);
        assert_eq!(receive_value(&subscriptions.live), 5);
        assert!(matches!(
            subscriptions.live.try_receive(),
            ReceiveOutcome::Empty
        ));

        let recorder = distributor.recorder_metrics();
        assert_eq!(recorder.accepted, 5);
        assert_eq!(recorder.received, 5);
        assert_eq!(recorder.rejected_full, 0);

        let live = distributor.live_metrics();
        assert_eq!(live.accepted, 5);
        assert_eq!(live.received, 2);
        assert_eq!(live.dropped_oldest, 3);
    }

    #[test]
    fn stalled_detection_consumer_keeps_newest_without_affecting_recorder() {
        let (distributor, subscriptions) =
            MediaDistributor::new_with_detection(capacity(1), capacity(1), capacity(2));
        drop(subscriptions.live);
        for value in 1..=5 {
            let report = distributor.publish(Arc::new(value));
            assert_eq!(report.recorder, PublishOutcome::Accepted);
            assert_eq!(receive_value(&subscriptions.recorder), value);
        }
        assert_eq!(receive_value(&subscriptions.detection), 4);
        assert_eq!(receive_value(&subscriptions.detection), 5);
        assert_eq!(distributor.detection_metrics().dropped_oldest, 3);
        assert_eq!(distributor.recorder_metrics().rejected_full, 0);
    }

    #[test]
    fn recorder_overflow_preserves_existing_backlog() {
        let (distributor, subscriptions) = MediaDistributor::new(capacity(2), capacity(1));

        assert_eq!(
            distributor.publish(Arc::new(1)).recorder,
            PublishOutcome::Accepted
        );
        assert_eq!(
            distributor.publish(Arc::new(2)).recorder,
            PublishOutcome::Accepted
        );
        let third = distributor.publish(Arc::new(3));
        assert_eq!(third.recorder, PublishOutcome::Full);
        assert_eq!(third.live, PublishOutcome::ReplacedOldest);

        assert_eq!(receive_value(&subscriptions.recorder), 1);
        assert_eq!(receive_value(&subscriptions.recorder), 2);
        assert_eq!(receive_value(&subscriptions.live), 3);
        assert_eq!(distributor.recorder_metrics().rejected_full, 1);
    }

    #[test]
    fn closing_one_consumer_does_not_close_the_other() {
        let (distributor, subscriptions) = MediaDistributor::new(capacity(1), capacity(1));
        drop(subscriptions.live);

        let report = distributor.publish(Arc::new(7));
        assert_eq!(report.recorder, PublishOutcome::Accepted);
        assert_eq!(report.live, PublishOutcome::Closed);
        assert_eq!(receive_value(&subscriptions.recorder), 7);
    }

    #[test]
    fn subscriber_drains_retained_items_after_producer_closes() {
        let (distributor, subscriptions) = MediaDistributor::new(capacity(1), capacity(1));
        let _ = distributor.publish(Arc::new(9));
        drop(distributor);

        assert_eq!(receive_value(&subscriptions.recorder), 9);
        assert!(matches!(
            subscriptions.recorder.try_receive(),
            ReceiveOutcome::Closed
        ));
        assert!(!subscriptions.recorder.metrics().producer_open);
    }

    #[test]
    fn dropping_consumer_releases_its_retained_items() {
        let (distributor, subscriptions) = MediaDistributor::new(capacity(1), capacity(2));
        let _ = distributor.publish(Arc::new(11));
        let _ = distributor.publish(Arc::new(12));
        drop(subscriptions.live);

        let metrics = distributor.live_metrics();
        assert_eq!(metrics.depth, 0);
        assert_eq!(metrics.dropped_on_close, 2);
        assert!(!metrics.consumer_open);
    }

    #[tokio::test]
    async fn async_receive_observes_publication_without_lost_wakeup() {
        let (distributor, mut subscriptions) = MediaDistributor::new(capacity(1), capacity(1));
        let _ = distributor.publish(Arc::new(21));

        match subscriptions.recorder.receive().await {
            ReceiveOutcome::Item(value) => assert_eq!(*value, 21),
            ReceiveOutcome::Empty => panic!("publication wake-up was lost"),
            ReceiveOutcome::Closed => panic!("producer unexpectedly closed"),
        }
    }

    #[tokio::test]
    async fn async_receive_wakes_when_producer_closes() {
        let (distributor, mut subscriptions) =
            MediaDistributor::<u64>::new(capacity(1), capacity(1));
        drop(distributor);

        assert!(matches!(
            subscriptions.recorder.receive().await,
            ReceiveOutcome::Closed
        ));
    }
}
