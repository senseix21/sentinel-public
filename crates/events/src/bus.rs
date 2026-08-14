//! Bounded in-process event bus with subscriber-independent queues.

use crate::event::{DeliveryClass, DomainEvent, EventKind};
use std::{
    collections::{BTreeMap, VecDeque},
    error::Error,
    fmt,
    sync::{Arc, Mutex, MutexGuard, Weak},
};
use tokio::sync::watch;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SubscriptionCapacity(usize);
impl SubscriptionCapacity {
    /// # Errors
    /// Returns an error for zero capacity.
    pub fn new(value: usize) -> Result<Self, BusError> {
        if value == 0 {
            Err(BusError::ZeroCapacity)
        } else {
            Ok(Self(value))
        }
    }
    #[must_use]
    pub const fn get(self) -> usize {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PublishReport {
    pub accepted: u64,
    pub rejected_full: u64,
    pub dropped_newest: u64,
    pub replaced_oldest: u64,
    pub coalesced: u64,
    pub closed: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiveOutcome {
    Event,
    Empty,
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SubscriptionMetrics {
    pub capacity: usize,
    pub depth: usize,
    pub accepted: u64,
    pub delivered: u64,
    pub rejected_full: u64,
    pub dropped_newest: u64,
    pub replaced_oldest: u64,
    pub coalesced: u64,
    pub dropped_on_close: u64,
    pub producer_open: bool,
    pub consumer_open: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BusError {
    ZeroCapacity,
    SubscriptionSpaceExhausted,
    Closed,
}
impl fmt::Display for BusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "event bus error: {self:?}")
    }
}
impl Error for BusError {}

#[derive(Default)]
struct Registry {
    next_id: u64,
    producer_open: bool,
    subscribers: BTreeMap<u64, Weak<Mutex<Queue>>>,
    publications: u64,
    outcomes: PublishReport,
    event_kinds: [u64; 6],
    delivery_classes: [u64; 4],
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EventBusMetrics {
    pub subscribers: usize,
    pub publications: u64,
    pub outcomes: PublishReport,
    pub event_kinds: [u64; 6],
    pub delivery_classes: [u64; 4],
}

#[derive(Clone)]
pub struct EventBus {
    registry: Arc<Mutex<Registry>>,
}
impl EventBus {
    #[must_use]
    pub fn new() -> Self {
        Self {
            registry: Arc::new(Mutex::new(Registry {
                producer_open: true,
                ..Registry::default()
            })),
        }
    }
    /// Creates one independently bounded subscription.
    ///
    /// # Errors
    /// Returns an error if subscription identity space is exhausted.
    pub fn subscribe(&self, capacity: SubscriptionCapacity) -> Result<EventSubscription, BusError> {
        let mut registry = lock(&self.registry);
        if !registry.producer_open {
            return Err(BusError::Closed);
        }
        registry.next_id = registry
            .next_id
            .checked_add(1)
            .ok_or(BusError::SubscriptionSpaceExhausted)?;
        let id = registry.next_id;
        let (signal, receiver) = watch::channel(0_u64);
        let queue = Arc::new(Mutex::new(Queue::new(capacity.get(), signal)));
        registry.subscribers.insert(id, Arc::downgrade(&queue));
        Ok(EventSubscription {
            id,
            queue,
            signal: receiver,
            registry: Arc::downgrade(&self.registry),
        })
    }
    #[allow(
        clippy::needless_pass_by_value,
        reason = "publication transfers one shared immutable event into subscriber queues"
    )]
    pub fn publish(&self, event: Arc<DomainEvent>) -> PublishReport {
        let subscribers: Vec<_> = {
            let mut registry = lock(&self.registry);
            if !registry.producer_open {
                return PublishReport {
                    closed: 1,
                    ..PublishReport::default()
                };
            }
            registry.subscribers.retain(|_, q| q.strong_count() > 0);
            registry
                .subscribers
                .values()
                .filter_map(Weak::upgrade)
                .collect()
        };
        let mut report = PublishReport::default();
        for queue in subscribers {
            let outcome = lock(&queue).push(Arc::clone(&event));
            match outcome {
                QueuePublish::Accepted => report.accepted += 1,
                QueuePublish::RejectedFull => report.rejected_full += 1,
                QueuePublish::DroppedNewest => report.dropped_newest += 1,
                QueuePublish::ReplacedOldest => report.replaced_oldest += 1,
                QueuePublish::Coalesced => report.coalesced += 1,
                QueuePublish::Closed => report.closed += 1,
            }
        }
        let mut registry = lock(&self.registry);
        registry.publications = registry.publications.saturating_add(1);
        let kind = event_kind_index(event.kind());
        registry.event_kinds[kind] = registry.event_kinds[kind].saturating_add(1);
        let class = delivery_class_index(event.delivery_class);
        registry.delivery_classes[class] = registry.delivery_classes[class].saturating_add(1);
        registry.outcomes.accepted = registry.outcomes.accepted.saturating_add(report.accepted);
        registry.outcomes.rejected_full = registry
            .outcomes
            .rejected_full
            .saturating_add(report.rejected_full);
        registry.outcomes.dropped_newest = registry
            .outcomes
            .dropped_newest
            .saturating_add(report.dropped_newest);
        registry.outcomes.replaced_oldest = registry
            .outcomes
            .replaced_oldest
            .saturating_add(report.replaced_oldest);
        registry.outcomes.coalesced = registry.outcomes.coalesced.saturating_add(report.coalesced);
        registry.outcomes.closed = registry.outcomes.closed.saturating_add(report.closed);
        report
    }
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        let mut r = lock(&self.registry);
        r.subscribers.retain(|_, q| q.strong_count() > 0);
        r.subscribers.len()
    }
    #[must_use]
    pub fn metrics(&self) -> EventBusMetrics {
        let mut registry = lock(&self.registry);
        registry
            .subscribers
            .retain(|_, queue| queue.strong_count() > 0);
        EventBusMetrics {
            subscribers: registry.subscribers.len(),
            publications: registry.publications,
            outcomes: registry.outcomes,
            event_kinds: registry.event_kinds,
            delivery_classes: registry.delivery_classes,
        }
    }
    pub fn close(&self) {
        let subscribers: Vec<_> = {
            let mut r = lock(&self.registry);
            r.producer_open = false;
            r.subscribers.values().filter_map(Weak::upgrade).collect()
        };
        for queue in subscribers {
            let mut queue = lock(&queue);
            queue.producer_open = false;
            queue.wake();
        }
    }
}
impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}
impl Drop for EventBus {
    fn drop(&mut self) {
        if Arc::strong_count(&self.registry) == 1 {
            self.close();
        }
    }
}

pub struct EventSubscription {
    id: u64,
    queue: Arc<Mutex<Queue>>,
    signal: watch::Receiver<u64>,
    registry: Weak<Mutex<Registry>>,
}
impl EventSubscription {
    #[must_use]
    pub fn try_receive(&self) -> (ReceiveOutcome, Option<Arc<DomainEvent>>) {
        lock(&self.queue).pop()
    }
    pub async fn receive(&mut self) -> Option<Arc<DomainEvent>> {
        loop {
            match self.try_receive() {
                (ReceiveOutcome::Event, event) => return event,
                (ReceiveOutcome::Closed, _) => return None,
                (ReceiveOutcome::Empty, _) => {}
            }
            if self.signal.changed().await.is_err() {
                return self.try_receive().1;
            }
        }
    }
    #[must_use]
    pub fn metrics(&self) -> SubscriptionMetrics {
        lock(&self.queue).metrics()
    }
}
impl Drop for EventSubscription {
    fn drop(&mut self) {
        let mut q = lock(&self.queue);
        q.consumer_open = false;
        q.dropped_on_close = q.dropped_on_close.saturating_add(q.items.len() as u64);
        q.items.clear();
        drop(q);
        if let Some(registry) = self.registry.upgrade() {
            lock(&registry).subscribers.remove(&self.id);
        }
    }
}

#[derive(Clone, Copy)]
enum QueuePublish {
    Accepted,
    RejectedFull,
    DroppedNewest,
    ReplacedOldest,
    Coalesced,
    Closed,
}
struct Queue {
    items: VecDeque<Arc<DomainEvent>>,
    capacity: usize,
    signal: watch::Sender<u64>,
    accepted: u64,
    delivered: u64,
    rejected_full: u64,
    dropped_newest: u64,
    replaced_oldest: u64,
    coalesced: u64,
    dropped_on_close: u64,
    producer_open: bool,
    consumer_open: bool,
}
impl Queue {
    fn new(capacity: usize, signal: watch::Sender<u64>) -> Self {
        Self {
            items: VecDeque::with_capacity(capacity),
            capacity,
            signal,
            accepted: 0,
            delivered: 0,
            rejected_full: 0,
            dropped_newest: 0,
            replaced_oldest: 0,
            coalesced: 0,
            dropped_on_close: 0,
            producer_open: true,
            consumer_open: true,
        }
    }
    fn push(&mut self, event: Arc<DomainEvent>) -> QueuePublish {
        if !self.consumer_open {
            return QueuePublish::Closed;
        }
        if event.delivery_class == DeliveryClass::LatestState {
            if let Some(key) = event.state_key() {
                if let Some(index) = self
                    .items
                    .iter()
                    .position(|old| old.state_key() == Some(key))
                {
                    self.items[index] = event;
                    self.coalesced = self.coalesced.saturating_add(1);
                    self.wake();
                    return QueuePublish::Coalesced;
                }
            }
        }
        if self.items.len() == self.capacity {
            match event.delivery_class {
                DeliveryClass::CriticalLifecycle => {
                    self.rejected_full += 1;
                    return QueuePublish::RejectedFull;
                }
                DeliveryClass::Telemetry => {
                    self.dropped_newest += 1;
                    return QueuePublish::DroppedNewest;
                }
                DeliveryClass::Detection | DeliveryClass::LatestState => {
                    let _ = self.items.pop_front();
                    self.replaced_oldest += 1;
                    self.items.push_back(event);
                    self.accepted += 1;
                    self.wake();
                    return QueuePublish::ReplacedOldest;
                }
            }
        }
        self.items.push_back(event);
        self.accepted += 1;
        self.wake();
        QueuePublish::Accepted
    }
    fn pop(&mut self) -> (ReceiveOutcome, Option<Arc<DomainEvent>>) {
        if let Some(event) = self.items.pop_front() {
            self.delivered += 1;
            (ReceiveOutcome::Event, Some(event))
        } else if self.producer_open {
            (ReceiveOutcome::Empty, None)
        } else {
            (ReceiveOutcome::Closed, None)
        }
    }
    fn metrics(&self) -> SubscriptionMetrics {
        SubscriptionMetrics {
            capacity: self.capacity,
            depth: self.items.len(),
            accepted: self.accepted,
            delivered: self.delivered,
            rejected_full: self.rejected_full,
            dropped_newest: self.dropped_newest,
            replaced_oldest: self.replaced_oldest,
            coalesced: self.coalesced,
            dropped_on_close: self.dropped_on_close,
            producer_open: self.producer_open,
            consumer_open: self.consumer_open,
        }
    }
    fn wake(&self) {
        self.signal
            .send_modify(|generation| *generation = generation.saturating_add(1));
    }
}
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

const fn event_kind_index(kind: EventKind) -> usize {
    match kind {
        EventKind::CameraStateChanged => 0,
        EventKind::RecordingSegmentCompleted => 1,
        EventKind::RecordingSegmentFailed => 2,
        EventKind::StoragePressureChanged => 3,
        EventKind::PersonDetectionObserved => 4,
        EventKind::DetectorStateChanged => 5,
    }
}
const fn delivery_class_index(class: DeliveryClass) -> usize {
    match class {
        DeliveryClass::CriticalLifecycle => 0,
        DeliveryClass::LatestState => 1,
        DeliveryClass::Telemetry => 2,
        DeliveryClass::Detection => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::{EventBus, ReceiveOutcome, SubscriptionCapacity};
    use crate::event::{
        CameraState, DeliveryClass, DomainEvent, EventId, EventPayload, OccurredAtUnixMicros,
    };
    use std::sync::Arc;
    use surveillance_common::types::CameraId;

    fn event(id: u32, class: DeliveryClass, state: CameraState) -> Arc<DomainEvent> {
        Arc::new(
            DomainEvent::new(
                EventId::new(format!("event-{id}"))
                    .unwrap_or_else(|error| panic!("event ID: {error}")),
                OccurredAtUnixMicros::new(i64::from(id) + 1)
                    .unwrap_or_else(|error| panic!("time: {error}")),
                Some(CameraId::new("camera-01").unwrap_or_else(|error| panic!("camera: {error}"))),
                class,
                EventPayload::CameraStateChanged { state },
            )
            .unwrap_or_else(|error| panic!("event: {error}")),
        )
    }

    fn capacity(value: usize) -> SubscriptionCapacity {
        SubscriptionCapacity::new(value).unwrap_or_else(|error| panic!("capacity: {error}"))
    }

    #[test]
    fn preserves_order_for_accepted_events() {
        let bus = EventBus::new();
        let subscription = bus
            .subscribe(capacity(3))
            .unwrap_or_else(|error| panic!("subscribe: {error}"));
        for id in 1..=3 {
            let report = bus.publish(event(
                id,
                DeliveryClass::CriticalLifecycle,
                CameraState::Connected,
            ));
            assert_eq!(report.accepted, 1);
        }
        for id in 1..=3 {
            let (_, observed) = subscription.try_receive();
            assert_eq!(
                observed.map(|event| event.event_id.as_str().to_owned()),
                Some(format!("event-{id}"))
            );
        }
    }

    #[test]
    fn critical_overflow_is_visible_and_detection_keeps_newest() {
        let bus = EventBus::new();
        let critical = bus
            .subscribe(capacity(1))
            .unwrap_or_else(|error| panic!("subscribe: {error}"));
        let _ = bus.publish(event(
            1,
            DeliveryClass::CriticalLifecycle,
            CameraState::Connected,
        ));
        assert_eq!(
            bus.publish(event(
                2,
                DeliveryClass::CriticalLifecycle,
                CameraState::Stopped
            ))
            .rejected_full,
            1
        );
        drop(critical);

        let detection = bus
            .subscribe(capacity(1))
            .unwrap_or_else(|error| panic!("subscribe: {error}"));
        let _ = bus.publish(event(3, DeliveryClass::Detection, CameraState::Connected));
        assert_eq!(
            bus.publish(event(4, DeliveryClass::Detection, CameraState::Stopped))
                .replaced_oldest,
            1
        );
        let (_, observed) = detection.try_receive();
        assert_eq!(
            observed.map(|event| event.event_id.as_str().to_owned()),
            Some("event-4".to_owned())
        );
    }

    #[test]
    fn latest_state_coalesces_and_telemetry_drops_newest() {
        let bus = EventBus::new();
        let subscription = bus
            .subscribe(capacity(1))
            .unwrap_or_else(|error| panic!("subscribe: {error}"));
        let _ = bus.publish(event(
            1,
            DeliveryClass::LatestState,
            CameraState::Connecting,
        ));
        assert_eq!(
            bus.publish(event(2, DeliveryClass::LatestState, CameraState::Connected))
                .coalesced,
            1
        );
        let (_, observed) = subscription.try_receive();
        assert_eq!(
            observed.map(|event| event.event_id.as_str().to_owned()),
            Some("event-2".to_owned())
        );
        let _ = bus.publish(event(3, DeliveryClass::Telemetry, CameraState::Connected));
        assert_eq!(
            bus.publish(event(4, DeliveryClass::Telemetry, CameraState::Stopped))
                .dropped_newest,
            1
        );
    }

    #[tokio::test]
    async fn close_wakes_waiter_and_unsubscribe_is_independent() {
        let bus = EventBus::new();
        let mut waiting = bus
            .subscribe(capacity(1))
            .unwrap_or_else(|error| panic!("subscribe: {error}"));
        let dead = bus
            .subscribe(capacity(1))
            .unwrap_or_else(|error| panic!("subscribe: {error}"));
        drop(dead);
        assert_eq!(bus.subscriber_count(), 1);
        bus.close();
        assert_eq!(
            bus.publish(event(9, DeliveryClass::Telemetry, CameraState::Stopped))
                .closed,
            1
        );
        assert!(matches!(
            bus.subscribe(capacity(1)),
            Err(super::BusError::Closed)
        ));
        assert!(waiting.receive().await.is_none());
        assert!(matches!(
            waiting.try_receive(),
            (ReceiveOutcome::Closed, None)
        ));
    }

    #[test]
    fn concurrent_publish_and_unsubscribe_remain_bounded() {
        let bus = EventBus::new();
        let subscription = bus
            .subscribe(capacity(2))
            .unwrap_or_else(|error| panic!("subscribe: {error}"));
        std::thread::scope(|scope| {
            let publisher = bus.clone();
            scope.spawn(move || {
                for id in 1..=100 {
                    let _ = publisher.publish(event(
                        id,
                        DeliveryClass::Detection,
                        CameraState::Connected,
                    ));
                }
            });
            scope.spawn(move || drop(subscription));
        });
        assert!(bus.subscriber_count() <= 1);
        let metrics = bus.metrics();
        assert_eq!(metrics.publications, 100);
        assert!(
            metrics.outcomes.accepted + metrics.outcomes.replaced_oldest + metrics.outcomes.closed
                <= 100
        );
    }
}
