//! Owns live publication lifecycle, pacing, health, retry, cancellation, and counters.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};

use surveillance_media::{
    distribution::{DecodableRunStart, MediaSubscriber, ReceiveOutcome},
    packet::H264AccessUnit,
};
use tokio::sync::watch;

use crate::{
    adapter::{
        AdapterErrorKind, AdapterObservation, EncodedAccessUnit, PublicationAdapter, SendOutcome,
        TimestampAuthority,
    },
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
    /// Times the media timeline stopped being usable for pacing and this
    /// session resynchronised at the next parameterised keyframe.
    ///
    /// Counts both directions. A timebase change or a timestamp that ran
    /// backwards is the obvious case. The one that mattered in practice is a
    /// timestamp that jumped *forward*: pacing is computed against an origin,
    /// so a forward jump asks this loop to sleep until that origin's future
    /// arrives, and while it sleeps it consumes nothing at all.
    pub timestamp_discontinuities: u64,
    pub media_rejections: u64,
    /// Access units the adapter accepted but did not put on the wire.
    pub adapter_drops: u64,
    /// Subscribers the adapter last reported, for one that gates on them.
    ///
    /// A gauge, not a total: it is stored from the adapter's own view rather
    /// than accumulated here, because the owner has no way to derive it.
    pub adapter_viewers: u64,
    /// Media items the adapter last reported producing from what it was
    /// offered — decoded and re-encoded pictures, for a transcoding adapter.
    pub adapter_media_produced: u64,
    /// Internal restarts the adapter last reported. Climbing here with a
    /// healthy state is a live view that is degraded and says it is not.
    pub adapter_restarts: u64,
    pub send_failures: u64,
    pub shutdown_failures: u64,
    pub timestamp_authority: TimestampAuthority,
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
    timestamp_discontinuities: AtomicU64,
    media_rejections: AtomicU64,
    adapter_drops: AtomicU64,
    adapter_viewers: AtomicU64,
    adapter_media_produced: AtomicU64,
    adapter_restarts: AtomicU64,
    send_failures: AtomicU64,
    shutdown_failures: AtomicU64,
}

struct HealthState {
    enabled: bool,
    state: Mutex<PublicationState>,
    failure: Mutex<Option<PublicationFailureCategory>>,
    counters: CounterState,
    timestamp_authority: TimestampAuthority,
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
            timestamp_authority: TimestampAuthority::Media,
        }))
    }
    #[must_use]
    pub fn enabled(timestamp_authority: TimestampAuthority) -> Self {
        Self(Arc::new(HealthState {
            enabled: true,
            state: Mutex::new(PublicationState::Starting),
            failure: Mutex::new(None),
            counters: CounterState::default(),
            timestamp_authority,
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
                timestamp_discontinuities: load(&self.0.counters.timestamp_discontinuities),
                media_rejections: load(&self.0.counters.media_rejections),
                adapter_drops: load(&self.0.counters.adapter_drops),
                adapter_viewers: load(&self.0.counters.adapter_viewers),
                adapter_media_produced: load(&self.0.counters.adapter_media_produced),
                adapter_restarts: load(&self.0.counters.adapter_restarts),
                send_failures: load(&self.0.counters.send_failures),
                shutdown_failures: load(&self.0.counters.shutdown_failures),
                timestamp_authority: self.0.timestamp_authority,
            },
        }
    }
    /// Stores what the adapter reports about itself.
    ///
    /// Stored rather than accumulated: these are the adapter's own running
    /// values, and adding them here would double-count every sample.
    fn record_adapter(&self, observation: AdapterObservation) {
        let counters = &self.0.counters;
        counters
            .adapter_viewers
            .store(observation.viewers, Ordering::Relaxed);
        counters
            .adapter_media_produced
            .store(observation.media_produced, Ordering::Relaxed);
        counters
            .adapter_restarts
            .store(observation.restarts, Ordering::Relaxed);
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
        let health = PublicationHealth::enabled(adapter.timestamp_authority());
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
        let mut publication_epoch = 0_u64;
        loop {
            if *shutdown.borrow() {
                return self.stop(PublicationStopReason::Requested).await;
            }
            self.health.transition(PublicationState::Connecting, None);
            increment(&self.health.0.counters.connection_attempts);
            match self.adapter.connect().await {
                Ok(()) => {
                    publication_epoch = publication_epoch.saturating_add(1);
                    increment(&self.health.0.counters.connection_successes);
                    if failures > 0 {
                        increment(&self.health.0.counters.reconnects);
                    }
                    self.health.transition(PublicationState::Publishing, None);
                    match self
                        .publish_session(&mut media, publication_epoch, &mut shutdown)
                        .await
                    {
                        SessionExit::Requested => {
                            return self.stop(PublicationStopReason::Requested).await
                        }
                        SessionExit::InputClosed => {
                            return self.stop(PublicationStopReason::InputClosed).await
                        }
                        SessionExit::Retryable(category) => {
                            // A publication that tears down and rebuilds itself
                            // used to do so entirely silently: nothing at INFO,
                            // and the child's own stderr discarded. A publisher
                            // could flap indefinitely while the log stayed
                            // clean. This is the one line that makes it visible.
                            tracing::warn!(
                                failure = ?category,
                                publication_epoch,
                                consecutive_failures = failures.saturating_add(1),
                                "live publication session ended and will reconnect; \
                                 recording is unaffected"
                            );
                            self.health
                                .transition(PublicationState::Degraded, Some(category));
                            failures = failures.saturating_add(1);
                        }
                        SessionExit::Terminal(category) => {
                            tracing::warn!(
                                failure = ?category,
                                publication_epoch,
                                "live publication cannot run on this build and will \
                                 not be retried; recording is unaffected"
                            );
                            self.health
                                .transition(PublicationState::Terminal, Some(category));
                            return self.stop(PublicationStopReason::Terminal).await;
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

    /// Records that the media timeline is no longer usable for pacing.
    ///
    /// The caller clears `ready` and `pacing_origin` and resynchronises at the
    /// next parameterised keyframe. Three conditions reach here: a timebase
    /// change, a timestamp that ran backwards, and a timestamp that jumped
    /// *forward* past `max_lag`.
    ///
    /// The forward case is the one that was missing, and it is the expensive one.
    /// Pacing is computed against an origin, so a forward jump asks
    /// `publish_session` to sleep until that origin's future arrives. Because that
    /// loop is the only consumer of the live subscription, sleeping stops it
    /// draining at all: the distributor evicts the entire live lane, every counter
    /// in the session freezes, and the state still reads `Publishing`. An RTSP
    /// reconnect supplies exactly such a jump, because a fresh session starts at a
    /// new RTP base and `crates/camera/src/rtsp.rs` passes it through.
    ///
    /// The stale-drop guard cannot catch it: that one fires when media runs
    /// *behind* the wall clock, which is the opposite direction.
    fn note_timeline_discontinuity(&self) {
        increment(&self.health.0.counters.timestamp_discontinuities);
        self.health.transition(
            PublicationState::Degraded,
            Some(PublicationFailureCategory::TimestampDiscontinuity),
        );
    }

    /// Decides what to do with one access unit's place on the media timeline,
    /// waiting out ordinary jitter but never an implausible gap.
    ///
    /// Both bounds are `max_lag`, used symmetrically. Media behind the wall clock
    /// by more than that is stale and dropped, which is the freshness-over-
    /// completeness policy `AGENTS.md` sets for live viewing. Media *ahead* of its
    /// own pacing origin by more than that is not something to sleep off — see
    /// `note_timeline_discontinuity` for why sleeping it was the defect.
    async fn pace(
        &self,
        media_elapsed: std::time::Duration,
        wall_elapsed: std::time::Duration,
        shutdown: &mut watch::Receiver<bool>,
    ) -> Pacing {
        if wall_elapsed > media_elapsed.saturating_add(self.config.max_lag) {
            increment(&self.health.0.counters.stale_drops);
            return Pacing::Drop;
        }
        if let Some(delay) = media_elapsed.checked_sub(wall_elapsed) {
            if delay > self.config.max_lag {
                self.note_timeline_discontinuity();
                return Pacing::Resynchronise;
            }
            tokio::select! {
                () = tokio::time::sleep(delay) => {}
                changed = shutdown.changed() => if changed.is_err() || *shutdown.borrow_and_update() { return Pacing::Stop; }
            }
        }
        Pacing::Deliver
    }

    /// Converts one access unit into the bytes and RTP timestamp the adapter
    /// takes, counting a rejection rather than failing the session.
    ///
    /// A unit this adapter cannot represent is one gap for a subscriber, not a
    /// reason to tear down a working publication.
    fn adapt(
        &self,
        media: &mut H264Adapter,
        unit: &H264AccessUnit,
        epoch: crate::rtp::PublicationEpoch,
    ) -> Option<(Vec<u8>, u32)> {
        let Ok(annex_b) = media.annex_b(unit) else {
            increment(&self.health.0.counters.media_rejections);
            return None;
        };
        let Ok(rtp_timestamp) = media.timestamp(epoch, unit) else {
            increment(&self.health.0.counters.media_rejections);
            return None;
        };
        Some((annex_b, rtp_timestamp))
    }

    async fn publish_session(
        &mut self,
        media: &mut H264Adapter,
        publication_epoch: u64,
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
                            self.note_timeline_discontinuity();
                            ready = false;
                            pacing_origin = None;
                        }
                        last_timestamp = Some(timestamp);
                        if !ready {
                            if !unit.is_keyframe() || !unit.has_parameter_sets() {
                                increment(&self.health.0.counters.parameter_set_waits);
                                continue;
                            }
                            ready = true;
                            pacing_origin = Some((timestamp, tokio::time::Instant::now()));
                            self.health.transition(PublicationState::Publishing, None);
                        }
                        let Some((origin, wall_origin)) = pacing_origin else {
                            // `ready` and `pacing_origin` are set and cleared
                            // together above, so this is unreachable. Resynchronise
                            // at the next parameterised keyframe rather than publish
                            // a capture time with no origin behind it.
                            ready = false;
                            continue;
                        };
                        let ticks = timestamp.ticks().saturating_sub(origin.ticks());
                        let media_elapsed = DurationFromMedia::convert(ticks, timestamp.timebase_hz());
                        let wall_elapsed = wall_origin.elapsed();
                        match self.pace(media_elapsed, wall_elapsed, shutdown).await {
                            Pacing::Deliver => {}
                            Pacing::Drop => continue,
                            Pacing::Resynchronise => {
                                ready = false;
                                pacing_origin = None;
                                continue;
                            }
                            Pacing::Stop => return SessionExit::Requested,
                        }
                        // Media time since this publication began. Saturation is
                        // unreachable in practice: i64 microseconds spans longer
                        // than any session, and the alternative would be to send a
                        // timestamp that ran backwards.
                        let capture_timestamp_us = i64::try_from(media_elapsed.as_micros()).unwrap_or(i64::MAX);
                        let epoch = crate::rtp::PublicationEpoch::new(publication_epoch);
                        let Some((annex_b, rtp_timestamp)) = self.adapt(media, &unit, epoch) else {
                            continue;
                        };
                        let outcome = self
                            .adapter
                            .send_access_unit(EncodedAccessUnit {
                                annex_b: &annex_b,
                                rtp_timestamp,
                                epoch: epoch.get(),
                                starts_decodable_run: unit.starts_decodable_run(),
                                capture_timestamp_us,
                            })
                            .await;
                        // Sampled on every access unit, before the outcome is
                        // classified: a viewer count that only refreshed on a
                        // published frame would read zero for exactly as long
                        // as the gate it controls was holding media back.
                        self.health.record_adapter(self.adapter.observe());
                        match outcome {
                            Ok(SendOutcome::Published) => {}
                            Ok(SendOutcome::Dropped) => {
                                // The adapter took the access unit and did not
                                // put it on the wire — no track yet, or a
                                // transport that was momentarily not ready.
                                // That is a gap in what a subscriber sees, not
                                // a failed session, and treating it as one
                                // would turn a momentary refusal into a
                                // reconnect loop.
                                increment(&self.health.0.counters.adapter_drops);
                                continue;
                            }
                            Err(error) => {
                                increment(&self.health.0.counters.send_failures);
                                // Preserve why it failed. Discarding the kind
                                // here turned every send failure into a
                                // transport failure, so an adapter that cannot
                                // publish at all on this platform reconnected
                                // on every backoff interval and reported itself
                                // merely degraded.
                                let category = adapter_category(error.kind());
                                if error.retryable() {
                                    return SessionExit::Retryable(category);
                                }
                                return SessionExit::Terminal(category);
                            }
                        }
                        increment(&self.health.0.counters.access_units);
                        self.health
                            .0
                            .counters
                            .bytes
                            .fetch_add(annex_b.len() as u64, Ordering::Relaxed);
                        if unit.is_keyframe() {
                            increment(&self.health.0.counters.keyframes);
                        }
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

/// What the media timeline says to do with one access unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pacing {
    /// Deliver it now; any pacing wait is already served.
    Deliver,
    /// Too stale to be worth a viewer's latency.
    Drop,
    /// The timeline is unusable; resynchronise at the next parameterised keyframe.
    Resynchronise,
    /// Shutdown arrived while pacing.
    Stop,
}

enum SessionExit {
    Requested,
    InputClosed,
    Retryable(PublicationFailureCategory),
    /// The adapter refused in a way that reconnecting cannot change -- a
    /// capability this build does not have, rather than a transport that
    /// happened to fail. Retrying it would log a failure every backoff interval
    /// forever and leave the publisher reporting `Degraded` when the honest
    /// state is `Terminal`.
    Terminal(PublicationFailureCategory),
}

const fn adapter_category(kind: AdapterErrorKind) -> PublicationFailureCategory {
    match kind {
        AdapterErrorKind::Unavailable => PublicationFailureCategory::AdapterUnavailable,
        AdapterErrorKind::Authentication => PublicationFailureCategory::Authentication,
        AdapterErrorKind::Connection => PublicationFailureCategory::Connection,
        AdapterErrorKind::Transport => PublicationFailureCategory::Transport,
        AdapterErrorKind::Shutdown => PublicationFailureCategory::Shutdown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        adapter::{AdapterError, AdapterFuture, MemoryAdapter},
        config::{LiveKitCredentials, PublicationBackoff},
        domain::PublicationIdentity,
    };
    use std::{sync::Arc, time::Duration};
    use surveillance_common::types::CameraId;
    use surveillance_media::{
        distribution::{MediaDistributor, QueueCapacity},
        packet::{H264AccessUnitAssembler, H264NalUnit, MediaTimestamp},
    };
    use url::Url;

    #[derive(Default)]
    struct RestartEvidence {
        connects: usize,
        disconnects: usize,
        sends: Vec<(u64, u32)>,
    }

    struct RestartOnceAdapter(Arc<Mutex<RestartEvidence>>);

    impl PublicationAdapter for RestartOnceAdapter {
        fn timestamp_authority(&self) -> TimestampAuthority {
            TimestampAuthority::Media
        }

        fn connect(&mut self) -> crate::adapter::AdapterFuture<'_, ()> {
            Box::pin(async move {
                lock(&self.0).connects += 1;
                Ok(())
            })
        }

        fn send_access_unit<'a>(
            &'a mut self,
            access_unit: EncodedAccessUnit<'a>,
        ) -> crate::adapter::AdapterFuture<'a, SendOutcome> {
            Box::pin(async move {
                let mut evidence = lock(&self.0);
                evidence
                    .sends
                    .push((access_unit.epoch, access_unit.rtp_timestamp));
                if evidence.sends.len() == 1 {
                    Err(crate::adapter::AdapterError::new(
                        AdapterErrorKind::Transport,
                    ))
                } else {
                    Ok(SendOutcome::Published)
                }
            })
        }

        fn disconnect(&mut self) -> crate::adapter::AdapterFuture<'_, ()> {
            Box::pin(async move {
                lock(&self.0).disconnects += 1;
                Ok(())
            })
        }
    }

    /// Records how the owner described each access unit it offered.
    struct DescribingAdapter(Arc<Mutex<Vec<(bool, i64)>>>);

    impl PublicationAdapter for DescribingAdapter {
        fn timestamp_authority(&self) -> TimestampAuthority {
            TimestampAuthority::Media
        }

        fn connect(&mut self) -> crate::adapter::AdapterFuture<'_, ()> {
            Box::pin(async move { Ok(()) })
        }

        fn send_access_unit<'a>(
            &'a mut self,
            access_unit: EncodedAccessUnit<'a>,
        ) -> crate::adapter::AdapterFuture<'a, SendOutcome> {
            Box::pin(async move {
                lock(&self.0).push((
                    access_unit.starts_decodable_run,
                    access_unit.capture_timestamp_us,
                ));
                Ok(SendOutcome::Published)
            })
        }

        fn disconnect(&mut self) -> crate::adapter::AdapterFuture<'_, ()> {
            Box::pin(async move { Ok(()) })
        }
    }

    fn config() -> LiveKitConfig {
        let camera = CameraId::new("camera-01").unwrap_or_else(|error| panic!("{error}"));
        LiveKitConfig {
            decoder_program: "ffmpeg".to_owned(),
            max_publish_height: 0,
            viewer_idle_grace: Duration::from_secs(5),
            endpoint: Url::parse("ws://127.0.0.1:7880").unwrap_or_else(|error| panic!("{error}")),
            credentials: LiveKitCredentials::new("key".to_owned(), "secret-value".to_owned())
                .unwrap_or_else(|error| panic!("{error}")),
            identity: PublicationIdentity::for_camera("test", &camera)
                .unwrap_or_else(|error| panic!("{error}")),
            deployment_namespace: "test".to_owned(),
            queue_items: std::num::NonZeroUsize::new(4).unwrap_or(std::num::NonZeroUsize::MIN),
            max_lag: Duration::from_secs(1),
            max_access_unit_bytes: std::num::NonZeroUsize::new(1024)
                .unwrap_or(std::num::NonZeroUsize::MIN),
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
        assemble(0, &[&[0x67, 100, 0], &[0x68, 0], &[0x65, 1]])
    }

    /// A non-IDR picture, which decodes only against an earlier reference.
    fn delta(ticks: i64) -> Arc<H264AccessUnit> {
        assemble(ticks, &[&[0x41, 1]])
    }

    fn keyframe_at(ticks: i64) -> Arc<H264AccessUnit> {
        assemble(ticks, &[&[0x67, 100, 0], &[0x68, 0], &[0x65, 1]])
    }

    fn assemble(ticks: i64, nals: &[&[u8]]) -> Arc<H264AccessUnit> {
        let mut assembler = H264AccessUnitAssembler::new();
        let timestamp =
            MediaTimestamp::new(ticks, 90_000).unwrap_or_else(|error| panic!("{error}"));
        let mut result = None;
        for (index, bytes) in nals.iter().enumerate() {
            result = assembler
                .push(
                    timestamp,
                    H264NalUnit::from_ebsp(Arc::from(*bytes))
                        .unwrap_or_else(|error| panic!("{error}")),
                    index + 1 == nals.len(),
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

    #[tokio::test]
    async fn each_access_unit_carries_its_run_start_and_media_derived_capture_time() {
        // An adapter that publishes encoded media has to label frames key or
        // delta and stamp them. Both answers are already known here, and both
        // come from the media rather than from the publisher's own clock.
        let capacity = QueueCapacity::new(4).unwrap_or_else(|error| panic!("{error}"));
        let (distributor, subscriptions) = MediaDistributor::new(capacity, capacity);
        drop(subscriptions.recorder);
        let described = Arc::new(Mutex::new(Vec::new()));
        let (owner, _health) = LivePublicationOwner::new(
            config(),
            DescribingAdapter(Arc::clone(&described)),
            subscriptions.live,
        );
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = tokio::spawn(owner.run(shutdown_rx));

        distributor.publish(keyframe());
        // 3000 ticks of a 90 kHz timebase is one frame at 30 fps.
        distributor.publish(delta(3_000));
        for _ in 0..200 {
            if lock(&described).len() >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        shutdown_tx.send_replace(true);
        let _ = task.await.unwrap_or_else(|error| panic!("{error}"));

        assert_eq!(
            *lock(&described),
            vec![(true, 0), (false, 33_333)],
            "publication begins at media time zero and advances by media time, \
             and only the parameterised keyframe starts a decodable run"
        );
    }

    /// A forward timestamp jump must resynchronise, not be slept out.
    ///
    /// This is the defect measured on the local stack on 2026-08-17: an RTSP
    /// reconnect gives the camera a new RTP base, the jump is forward rather than
    /// backward so the discontinuity guard above did not fire, and pacing then
    /// asked this loop to sleep until the old origin's future arrived. Because
    /// this loop is the only consumer of the live subscription, it stopped
    /// draining entirely — the distributor evicted the whole live lane, every
    /// counter in the session froze, and health still reported `Publishing`.
    ///
    /// The assertion that matters is that publication *continues*. A test that
    /// only checked the counter would pass against a loop that counted the jump
    /// and then stalled anyway.
    #[tokio::test]
    async fn a_forward_timestamp_jump_resynchronises_instead_of_stalling_the_lane() {
        // One hour of 90 kHz ticks: what a fresh RTSP session's new base looks
        // like from here. `max_lag` in `config()` is one second, so this is
        // unambiguously past it.
        const ONE_HOUR_OF_TICKS: i64 = 90_000 * 3_600;

        let capacity = QueueCapacity::new(4).unwrap_or_else(|error| panic!("{error}"));
        let (distributor, subscriptions) = MediaDistributor::new(capacity, capacity);
        drop(subscriptions.recorder);
        let described = Arc::new(Mutex::new(Vec::new()));
        let (owner, health) = LivePublicationOwner::new(
            config(),
            DescribingAdapter(Arc::clone(&described)),
            subscriptions.live,
        );
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = tokio::spawn(owner.run(shutdown_rx));

        // One ordinary session establishes the pacing origin at media zero.
        distributor.publish(keyframe());
        for _ in 0..200 {
            if !lock(&described).is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }

        // Before the fix the owner slept on this unit and never consumed another.
        distributor.publish(delta(ONE_HOUR_OF_TICKS));
        // The recovery keyframe re-establishes the origin at the new base.
        distributor.publish(keyframe_at(ONE_HOUR_OF_TICKS + 3_000));

        for _ in 0..200 {
            if lock(&described).len() >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let snapshot = health.snapshot();
        shutdown_tx.send_replace(true);
        let _ = task.await.unwrap_or_else(|error| panic!("{error}"));

        assert_eq!(
            *lock(&described),
            vec![(true, 0), (true, 0)],
            "the lane keeps publishing across a forward jump, and the recovery \
             keyframe starts a fresh timestamp epoch at media time zero rather \
             than inheriting the stale origin"
        );
        assert_eq!(
            snapshot.counters.timestamp_discontinuities, 1,
            "the jump is counted, so an operator can tell a resynchronising \
             source from a healthy one"
        );
        assert_eq!(
            snapshot.state,
            PublicationState::Publishing,
            "and the state recovers once publication resumes. Degraded that never \
             clears would stick for the life of the connection after the first \
             camera reconnect, which is a signal an operator learns to ignore"
        );
    }

    /// Evidence from a publication that met a decoder binary mid-replacement.
    #[cfg(target_os = "linux")]
    #[derive(Default)]
    struct SwapEvidence {
        refused: Vec<AdapterErrorKind>,
        published_after_swap: usize,
    }

    /// Starts a real decoder on every access unit, so the error the publication
    /// loop sees is the one the operating system actually produced.
    ///
    /// Nothing here names a failure. The fixture is held open for writing while
    /// the test begins, which is what a deployment rewriting the decoder binary
    /// looks like to `exec`, and the resulting `io::Error` goes through the
    /// same classification and the same mapping the adapter uses in
    /// production. Only the room is absent, because a `Session` holds a
    /// connected `Room` and there is no server here.
    #[cfg(target_os = "linux")]
    struct DeploymentSwapAdapter {
        program: String,
        evidence: Arc<Mutex<SwapEvidence>>,
    }

    #[cfg(target_os = "linux")]
    struct NullSink;

    #[cfg(target_os = "linux")]
    impl crate::decode::DecodedFrameSink for NullSink {
        fn accept(&mut self, _yuv420p: &[u8], _size: crate::decode::FrameSize) {}
    }

    #[cfg(target_os = "linux")]
    impl PublicationAdapter for DeploymentSwapAdapter {
        fn timestamp_authority(&self) -> TimestampAuthority {
            TimestampAuthority::Media
        }

        fn connect(&mut self) -> AdapterFuture<'_, ()> {
            Box::pin(async { Ok(()) })
        }

        fn send_access_unit<'a>(
            &'a mut self,
            _access_unit: EncodedAccessUnit<'a>,
        ) -> AdapterFuture<'a, SendOutcome> {
            Box::pin(async move {
                let size = crate::decode::FrameSize::new(320, 240)
                    .unwrap_or_else(|error| panic!("{error}"));
                match crate::decode::H264ToI420::spawn(
                    &self.program,
                    size,
                    NullSink,
                    Arc::new(crate::decode::DecodeCounters::default()),
                ) {
                    Ok(decoder) => {
                        decoder.shutdown().await;
                        lock(&self.evidence).published_after_swap += 1;
                        Ok(SendOutcome::Published)
                    }
                    Err(error) => {
                        let kind = crate::transcode::start_failure_kind(error);
                        lock(&self.evidence).refused.push(kind);
                        Err(crate::adapter::AdapterError::new(kind))
                    }
                }
            })
        }

        fn disconnect(&mut self) -> AdapterFuture<'_, ()> {
            Box::pin(async { Ok(()) })
        }
    }

    /// A camera whose decoder binary is replaced underneath it keeps publishing.
    ///
    /// This is the failure the classification exists for, assembled end to end:
    /// a real `exec` refused by the kernel, the real mapping into the adapter
    /// contract, and the real publication loop deciding what to do about it.
    /// Before the classification existed the loop reached `Terminal` here and
    /// the camera stayed dark until something else restarted it, which is why
    /// the assertion below is about the state and not only the counters.
    ///
    /// Linux only: macOS permits the `exec` outright, so there is no refusal to
    /// recover from. Linux is the deployment target.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_decoder_binary_replaced_mid_publication_recovers_on_its_own() {
        let fixture = std::env::temp_dir().join(format!("swapped-decoder-{}", std::process::id()));
        std::fs::write(&fixture, "#!/bin/sh\nexec sleep 300\n")
            .unwrap_or_else(|error| panic!("write the fixture: {error}"));
        std::fs::set_permissions(
            &fixture,
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o755),
        )
        .unwrap_or_else(|error| panic!("mark the fixture runnable: {error}"));
        // The deployment, in progress: the file exists and is executable, and
        // the writer has not finished with it.
        let deployment = std::fs::OpenOptions::new()
            .write(true)
            .open(&fixture)
            .unwrap_or_else(|error| panic!("hold the fixture open: {error}"));

        let capacity = QueueCapacity::new(4).unwrap_or_else(|error| panic!("{error}"));
        let (distributor, subscriptions) = MediaDistributor::new(capacity, capacity);
        drop(subscriptions.recorder);
        let evidence = Arc::new(Mutex::new(SwapEvidence::default()));
        let (owner, health) = LivePublicationOwner::new(
            config(),
            DeploymentSwapAdapter {
                program: fixture.to_string_lossy().into_owned(),
                evidence: Arc::clone(&evidence),
            },
            subscriptions.live,
        );
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = tokio::spawn(owner.run(shutdown_rx));

        distributor.publish(keyframe());
        for _ in 0..500 {
            if !lock(&evidence).refused.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let refusals = lock(&evidence).refused.clone();
        assert!(
            !refusals.is_empty(),
            "the decoder must have been refused while its binary was held open"
        );
        assert!(
            refusals
                .iter()
                .all(|kind| *kind == AdapterErrorKind::Transport),
            "a binary being replaced is a fault of the moment: {refusals:?}"
        );

        // The deployment finishes.
        drop(deployment);
        for _ in 0..1000 {
            if lock(&evidence).published_after_swap > 0 {
                break;
            }
            distributor.publish(keyframe());
            tokio::time::sleep(Duration::from_millis(1)).await;
        }

        shutdown_tx.send_replace(true);
        let exit = task.await.unwrap_or_else(|error| panic!("{error}"));
        let _ = std::fs::remove_file(&fixture);

        assert!(
            lock(&evidence).published_after_swap > 0,
            "the camera must publish again once the binary is readable"
        );
        // The stop reason, not the state: a publication that gives up still
        // ends in `Stopped`, so asserting on the state would have passed
        // whatever happened here. `Terminal` is the one that needs an operator
        // to notice a camera went dark over a deployment that already
        // finished, and `a_non_retryable_send_failure_stops_publication_
        // instead_of_reconnecting` is what that looks like.
        assert_eq!(
            exit.reason,
            PublicationStopReason::Requested,
            "this publication must end because the test asked it to, not \
             because it gave up on a decoder that came back"
        );
        assert!(
            health.snapshot().counters.reconnects > 0,
            "recovery is the reconnect, so it should be visible as one"
        );
    }

    #[tokio::test]
    async fn retry_owns_disconnect_and_starts_a_fresh_timestamp_epoch() {
        let capacity = QueueCapacity::new(4).unwrap_or_else(|error| panic!("{error}"));
        let (distributor, subscriptions) = MediaDistributor::new(capacity, capacity);
        drop(subscriptions.recorder);
        let evidence = Arc::new(Mutex::new(RestartEvidence::default()));
        let (owner, health) = LivePublicationOwner::new(
            config(),
            RestartOnceAdapter(Arc::clone(&evidence)),
            subscriptions.live,
        );
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = tokio::spawn(owner.run(shutdown_rx));

        distributor.publish(keyframe());
        for _ in 0..100 {
            if health.snapshot().counters.connection_attempts >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        distributor.publish(keyframe());
        for _ in 0..100 {
            if lock(&evidence).sends.len() >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }

        shutdown_tx.send_replace(true);
        let exit = task.await.unwrap_or_else(|error| panic!("{error}"));
        let evidence = lock(&evidence);
        assert_eq!(evidence.sends, vec![(1, 0), (2, 0)]);
        assert!(evidence.connects >= 2);
        assert!(evidence.disconnects >= 2);
        assert_eq!(exit.snapshot.counters.reconnects, 1);
        assert_eq!(
            exit.snapshot.counters.timestamp_authority,
            TimestampAuthority::Media
        );
    }

    /// Connects, then refuses every access unit with a non-retryable error --
    /// the shape of an adapter whose publication path is unavailable in this
    /// build rather than momentarily unreachable.
    struct UnavailableOnSendAdapter(Arc<AtomicU64>);

    impl PublicationAdapter for UnavailableOnSendAdapter {
        fn timestamp_authority(&self) -> TimestampAuthority {
            TimestampAuthority::Media
        }
        fn connect(&mut self) -> AdapterFuture<'_, ()> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Box::pin(async { Ok(()) })
        }
        fn send_access_unit<'a>(
            &'a mut self,
            _access_unit: EncodedAccessUnit<'a>,
        ) -> AdapterFuture<'a, SendOutcome> {
            Box::pin(async { Err(AdapterError::new(AdapterErrorKind::Unavailable)) })
        }
        fn disconnect(&mut self) -> AdapterFuture<'_, ()> {
            Box::pin(async { Ok(()) })
        }
    }

    /// The send path used to report every failure as `Transport`, which is
    /// retryable, so an adapter that could never publish reconnected on every
    /// backoff interval and reported itself merely `Degraded`. It must stop
    /// once, stay stopped, and say so -- while leaving recording alone.
    #[tokio::test]
    async fn a_non_retryable_send_failure_stops_publication_instead_of_reconnecting() {
        let capacity = QueueCapacity::new(4).unwrap_or_else(|error| panic!("{error}"));
        let (distributor, subscriptions) = MediaDistributor::new(capacity, capacity);
        drop(subscriptions.recorder);
        let connects = Arc::new(AtomicU64::new(0));
        let (owner, health) = LivePublicationOwner::new(
            config(),
            UnavailableOnSendAdapter(Arc::clone(&connects)),
            subscriptions.live,
        );
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = tokio::spawn(owner.run(shutdown_rx));

        distributor.publish(keyframe());

        // No shutdown is sent: the run must end on its own. A retry loop would
        // hang here rather than fail, so the join is the assertion.
        let exit = task.await.unwrap_or_else(|error| panic!("{error}"));

        assert_eq!(
            exit.reason,
            PublicationStopReason::Terminal,
            "a publication that cannot publish stops terminally, it does not degrade"
        );
        assert_eq!(
            exit.snapshot.state,
            PublicationState::Stopped,
            "stopping terminally still ends in the stopped state"
        );
        assert_eq!(
            exit.snapshot.last_failure,
            Some(PublicationFailureCategory::AdapterUnavailable),
            "the reported category must survive the send path, not become Transport"
        );
        assert_eq!(
            connects.load(Ordering::Relaxed),
            1,
            "it must not reconnect after a failure reconnecting cannot fix"
        );
        assert_eq!(exit.snapshot.counters.access_units, 0);
        assert_eq!(exit.snapshot.counters.send_failures, 1);
        assert_eq!(health.snapshot().state, PublicationState::Stopped);
    }
}
