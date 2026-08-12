use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};

use surveillance_media::{
    distribution::{MediaSubscriber, ReceiveOutcome},
    packet::H264AccessUnit,
};
use tokio::sync::watch;

use crate::{
    adapter::{AdapterErrorKind, PublicationAdapter},
    config::LiveKitConfig,
    domain::{PublicationFailureCategory, PublicationState},
    rtp::{H264Adapter, PacketizationConfig},
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PublicationCounters {
    pub connection_attempts: u64,
    pub connection_successes: u64,
    pub reconnects: u64,
    pub access_units: u64,
    pub bytes: u64,
    pub keyframes: u64,
    pub parameter_set_waits: u64,
    pub stale_drops: u64,
    pub media_rejections: u64,
    pub send_failures: u64,
    pub shutdown_failures: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PublicationSnapshot {
    pub enabled: bool,
    pub state: PublicationState,
    pub last_failure: Option<PublicationFailureCategory>,
    pub counters: PublicationCounters,
}

#[derive(Default)]
struct CounterState {
    connection_attempts: AtomicU64,
    connection_successes: AtomicU64,
    reconnects: AtomicU64,
    access_units: AtomicU64,
    bytes: AtomicU64,
    keyframes: AtomicU64,
    parameter_set_waits: AtomicU64,
    stale_drops: AtomicU64,
    media_rejections: AtomicU64,
    send_failures: AtomicU64,
    shutdown_failures: AtomicU64,
}

struct HealthState {
    enabled: bool,
    state: Mutex<PublicationState>,
    failure: Mutex<Option<PublicationFailureCategory>>,
    counters: CounterState,
}

#[derive(Clone)]
pub struct PublicationHealth(Arc<HealthState>);

impl PublicationHealth {
    #[must_use]
    pub fn disabled() -> Self {
        Self(Arc::new(HealthState {
            enabled: false,
            state: Mutex::new(PublicationState::Disabled),
            failure: Mutex::new(None),
            counters: CounterState::default(),
        }))
    }
    #[must_use]
    pub fn enabled() -> Self {
        Self(Arc::new(HealthState {
            enabled: true,
            state: Mutex::new(PublicationState::Starting),
            failure: Mutex::new(None),
            counters: CounterState::default(),
        }))
    }
    #[must_use]
    pub fn snapshot(&self) -> PublicationSnapshot {
        let load = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        PublicationSnapshot {
            enabled: self.0.enabled,
            state: *lock(&self.0.state),
            last_failure: *lock(&self.0.failure),
            counters: PublicationCounters {
                connection_attempts: load(&self.0.counters.connection_attempts),
                connection_successes: load(&self.0.counters.connection_successes),
                reconnects: load(&self.0.counters.reconnects),
                access_units: load(&self.0.counters.access_units),
                bytes: load(&self.0.counters.bytes),
                keyframes: load(&self.0.counters.keyframes),
                parameter_set_waits: load(&self.0.counters.parameter_set_waits),
                stale_drops: load(&self.0.counters.stale_drops),
                media_rejections: load(&self.0.counters.media_rejections),
                send_failures: load(&self.0.counters.send_failures),
                shutdown_failures: load(&self.0.counters.shutdown_failures),
            },
        }
    }
    fn transition(&self, state: PublicationState, failure: Option<PublicationFailureCategory>) {
        let mut current = lock(&self.0.state);
        if *current != state && !current.can_transition_to(state) {
            tracing::error!(from=?*current, to=?state, "invalid publication lifecycle transition");
        }
        *current = state;
        *lock(&self.0.failure) = failure;
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
fn increment(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublicationStopReason {
    Requested,
    InputClosed,
    Terminal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PublicationExit {
    pub reason: PublicationStopReason,
    pub snapshot: PublicationSnapshot,
}

pub struct LivePublicationOwner<A> {
    config: LiveKitConfig,
    adapter: A,
    input: MediaSubscriber<H264AccessUnit>,
    health: PublicationHealth,
}

impl<A: PublicationAdapter> LivePublicationOwner<A> {
    #[must_use]
    pub fn new(
        config: LiveKitConfig,
        adapter: A,
        input: MediaSubscriber<H264AccessUnit>,
    ) -> (Self, PublicationHealth) {
        let health = PublicationHealth::enabled();
        (
            Self {
                config,
                adapter,
                input,
                health: health.clone(),
            },
            health,
        )
    }

    pub async fn run(mut self, mut shutdown: watch::Receiver<bool>) -> PublicationExit {
        let packetization =
            PacketizationConfig::new(1200, self.config.max_access_unit_bytes.get(), 4096)
                .unwrap_or_else(|_| unreachable!("validated fixed packetization bounds"));
        let mut media = H264Adapter::new(packetization);
        let mut failures = 0_u32;
        loop {
            if *shutdown.borrow() {
                return self.stop(PublicationStopReason::Requested).await;
            }
            self.health.transition(PublicationState::Connecting, None);
            increment(&self.health.0.counters.connection_attempts);
            match self.adapter.connect().await {
                Ok(()) => {
                    increment(&self.health.0.counters.connection_successes);
                    if failures > 0 {
                        increment(&self.health.0.counters.reconnects);
                    }
                    self.health.transition(PublicationState::Publishing, None);
                    match self.publish_session(&mut media, &mut shutdown).await {
                        SessionExit::Requested => {
                            return self.stop(PublicationStopReason::Requested).await
                        }
                        SessionExit::InputClosed => {
                            return self.stop(PublicationStopReason::InputClosed).await
                        }
                        SessionExit::Retryable(category) => {
                            self.health
                                .transition(PublicationState::Degraded, Some(category));
                            failures = failures.saturating_add(1);
                        }
                    }
                }
                Err(error) if !error.retryable() => {
                    self.health.transition(
                        PublicationState::Terminal,
                        Some(adapter_category(error.kind())),
                    );
                    return self.stop(PublicationStopReason::Terminal).await;
                }
                Err(error) => {
                    self.health.transition(
                        PublicationState::Backoff,
                        Some(adapter_category(error.kind())),
                    );
                    failures = failures.saturating_add(1);
                }
            }
            let _ = self.adapter.disconnect().await;
            self.health.transition(
                PublicationState::Backoff,
                self.health.snapshot().last_failure,
            );
            let delay = self.config.backoff.delay(failures.saturating_sub(1));
            tokio::select! {
                () = tokio::time::sleep(delay) => {}
                changed = shutdown.changed() => if changed.is_err() || *shutdown.borrow_and_update() { return self.stop(PublicationStopReason::Requested).await; }
            }
        }
    }

    async fn publish_session(
        &mut self,
        media: &mut H264Adapter,
        shutdown: &mut watch::Receiver<bool>,
    ) -> SessionExit {
        let mut ready = false;
        let mut pacing_origin: Option<(
            surveillance_media::packet::MediaTimestamp,
            tokio::time::Instant,
        )> = None;
        let mut last_timestamp = None;
        loop {
            tokio::select! {
                biased;
                changed = shutdown.changed() => if changed.is_err() || *shutdown.borrow_and_update() { return SessionExit::Requested; },
                outcome = self.input.receive() => match outcome {
                    ReceiveOutcome::Closed => return SessionExit::InputClosed,
                    ReceiveOutcome::Empty => {},
                    ReceiveOutcome::Item(unit) => {
                        let timestamp = unit.timestamp();
                        if last_timestamp.is_some_and(|last: surveillance_media::packet::MediaTimestamp| {
                            last.timebase_hz() != timestamp.timebase_hz() || timestamp.ticks() < last.ticks()
                        }) {
                            ready = false;
                            pacing_origin = None;
                            self.health.transition(PublicationState::Degraded, Some(PublicationFailureCategory::TimestampDiscontinuity));
                        }
                        last_timestamp = Some(timestamp);
                        if !ready {
                            if !unit.is_keyframe() || !unit.has_parameter_sets() {
                                increment(&self.health.0.counters.parameter_set_waits);
                                continue;
                            }
                            ready = true;
                            pacing_origin = Some((timestamp, tokio::time::Instant::now()));
                        }
                        if let Some((origin, wall_origin)) = pacing_origin {
                            let ticks = timestamp.ticks().saturating_sub(origin.ticks());
                            let media_elapsed = DurationFromMedia::convert(ticks, timestamp.timebase_hz());
                            let wall_elapsed = wall_origin.elapsed();
                            if wall_elapsed > media_elapsed.saturating_add(self.config.max_lag) {
                                increment(&self.health.0.counters.stale_drops);
                                continue;
                            }
                            if let Some(delay) = media_elapsed.checked_sub(wall_elapsed) {
                                tokio::select! {
                                    () = tokio::time::sleep(delay) => {}
                                    changed = shutdown.changed() => if changed.is_err() || *shutdown.borrow_and_update() { return SessionExit::Requested; }
                                }
                            }
                        }
                        let Ok(annex_b) = media.annex_b(&unit) else {
                            increment(&self.health.0.counters.media_rejections);
                            continue;
                        };
                        if self.adapter.send_access_unit(&annex_b).await.is_err() {
                            increment(&self.health.0.counters.send_failures);
                            return SessionExit::Retryable(PublicationFailureCategory::Transport);
                        }
                        increment(&self.health.0.counters.access_units);
                        self.health.0.counters.bytes.fetch_add(annex_b.len() as u64, Ordering::Relaxed);
                        if unit.is_keyframe() { increment(&self.health.0.counters.keyframes); }
                    }
                }
            }
        }
    }

    async fn stop(&mut self, reason: PublicationStopReason) -> PublicationExit {
        let current = self.health.snapshot().state;
        if current != PublicationState::Stopped {
            self.health.transition(
                PublicationState::Stopping,
                self.health.snapshot().last_failure,
            );
        }
        if self.adapter.disconnect().await.is_err() {
            increment(&self.health.0.counters.shutdown_failures);
        }
        self.health.transition(
            PublicationState::Stopped,
            self.health.snapshot().last_failure,
        );
        PublicationExit {
            reason,
            snapshot: self.health.snapshot(),
        }
    }
}

struct DurationFromMedia;
impl DurationFromMedia {
    fn convert(ticks: i64, timebase_hz: u32) -> std::time::Duration {
        let ticks = u64::try_from(ticks).unwrap_or(0);
        let seconds = ticks / u64::from(timebase_hz);
        let remainder = ticks % u64::from(timebase_hz);
        let nanos =
            u32::try_from(u128::from(remainder) * 1_000_000_000_u128 / u128::from(timebase_hz))
                .unwrap_or(999_999_999);
        std::time::Duration::new(seconds, nanos)
    }
}

enum SessionExit {
    Requested,
    InputClosed,
    Retryable(PublicationFailureCategory),
}

const fn adapter_category(kind: AdapterErrorKind) -> PublicationFailureCategory {
    match kind {
        AdapterErrorKind::Unavailable | AdapterErrorKind::VersionMismatch => {
            PublicationFailureCategory::AdapterUnavailable
        }
        AdapterErrorKind::Authentication => PublicationFailureCategory::Authentication,
        AdapterErrorKind::Connection => PublicationFailureCategory::Connection,
        AdapterErrorKind::Transport => PublicationFailureCategory::Transport,
        AdapterErrorKind::ChildExited => PublicationFailureCategory::ChildExited,
        AdapterErrorKind::Shutdown => PublicationFailureCategory::Shutdown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        adapter::MemoryAdapter,
        config::{LiveKitCredentials, PublicationBackoff},
        domain::PublicationIdentity,
    };
    use std::{path::PathBuf, sync::Arc, time::Duration};
    use surveillance_common::types::CameraId;
    use surveillance_media::{
        distribution::{MediaDistributor, QueueCapacity},
        packet::{H264AccessUnitAssembler, H264NalUnit, MediaTimestamp},
    };
    use url::Url;

    fn config() -> LiveKitConfig {
        let camera = CameraId::new("camera-01").unwrap_or_else(|error| panic!("{error}"));
        LiveKitConfig {
            endpoint: Url::parse("ws://127.0.0.1:7880").unwrap_or_else(|error| panic!("{error}")),
            credentials: LiveKitCredentials::new("key".to_owned(), "secret-value".to_owned())
                .unwrap_or_else(|error| panic!("{error}")),
            identity: PublicationIdentity::for_camera("test", &camera)
                .unwrap_or_else(|error| panic!("{error}")),
            deployment_namespace: "test".to_owned(),
            cli_path: PathBuf::from("lk"),
            cli_version: "2.18.2".to_owned(),
            queue_items: std::num::NonZeroUsize::new(4).unwrap_or(std::num::NonZeroUsize::MIN),
            max_lag: Duration::from_secs(1),
            max_access_unit_bytes: std::num::NonZeroUsize::new(1024)
                .unwrap_or(std::num::NonZeroUsize::MIN),
            ipc_write_timeout: Duration::from_secs(1),
            shutdown_timeout: Duration::from_secs(1),
            backoff: PublicationBackoff::new(
                Duration::from_millis(1),
                Duration::from_millis(2),
                Duration::from_secs(1),
            )
            .unwrap_or_else(|error| panic!("{error}")),
        }
    }

    fn keyframe() -> Arc<H264AccessUnit> {
        let mut assembler = H264AccessUnitAssembler::new();
        let timestamp = MediaTimestamp::new(0, 90_000).unwrap_or_else(|error| panic!("{error}"));
        let nals: [&[u8]; 3] = [&[0x67, 100, 0], &[0x68, 0], &[0x65, 1]];
        let mut result = None;
        for (index, bytes) in nals.iter().enumerate() {
            result = assembler
                .push(
                    timestamp,
                    H264NalUnit::from_ebsp(Arc::from(*bytes))
                        .unwrap_or_else(|error| panic!("{error}")),
                    index == 2,
                )
                .unwrap_or_else(|error| panic!("{error}"));
        }
        Arc::new(result.unwrap_or_else(|| panic!("access unit")))
    }

    #[tokio::test]
    async fn owner_waits_for_parameterized_keyframe_and_joins_shutdown() {
        let capacity = QueueCapacity::new(4).unwrap_or_else(|error| panic!("{error}"));
        let (distributor, subscriptions) = MediaDistributor::new(capacity, capacity);
        drop(subscriptions.recorder);
        let (owner, health) =
            LivePublicationOwner::new(config(), MemoryAdapter::healthy(), subscriptions.live);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = tokio::spawn(owner.run(shutdown_rx));
        distributor.publish(keyframe());
        tokio::task::yield_now().await;
        shutdown_tx.send_replace(true);
        let exit = task.await.unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(exit.reason, PublicationStopReason::Requested);
        assert_eq!(health.snapshot().state, PublicationState::Stopped);
        assert_eq!(health.snapshot().counters.access_units, 1);
    }
}
