//! Singular ownership and restart policy for one H.264 camera session.

use std::{error::Error, fmt, num::NonZeroU32, sync::Arc, time::Duration};

use surveillance_common::types::CameraId;
use surveillance_media::{
    distribution::{MediaDistributor, PublishOutcome},
    packet::H264AccessUnit,
};
use tokio::sync::watch;

use crate::health::{camera_health_channel, CameraHealthPublisher};
use crate::{
    connection::{ConnectionSnapshot, DisconnectedCamera, ReconnectDelay, RetryableCameraFailure},
    rtsp::{connect_h264, RtspConnectError, RtspReadOutcome, RtspSourceConfig},
};

/// Validated bounded exponential-backoff parameters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReconnectPolicy {
    initial: Duration,
    maximum: Duration,
    jitter_basis_points: u16,
    teardown_timeout: Duration,
    stable_session_reset: Duration,
}

impl ReconnectPolicy {
    /// Creates a reconnect policy. Jitter is symmetric and at most 100%.
    ///
    /// # Errors
    ///
    /// Rejects zero durations, a maximum below the initial delay, or jitter
    /// above 10,000 basis points.
    pub fn new(
        initial: Duration,
        maximum: Duration,
        jitter_basis_points: u16,
        teardown_timeout: Duration,
        stable_session_reset: Duration,
    ) -> Result<Self, ReconnectPolicyError> {
        if initial.is_zero() {
            return Err(ReconnectPolicyError::ZeroInitial);
        }
        if maximum < initial {
            return Err(ReconnectPolicyError::MaximumBelowInitial);
        }
        if jitter_basis_points > 10_000 {
            return Err(ReconnectPolicyError::JitterAboveOneHundredPercent);
        }
        if teardown_timeout.is_zero() {
            return Err(ReconnectPolicyError::ZeroTeardownTimeout);
        }
        if stable_session_reset.is_zero() {
            return Err(ReconnectPolicyError::ZeroStableSessionReset);
        }
        Ok(Self {
            initial,
            maximum,
            jitter_basis_points,
            teardown_timeout,
            stable_session_reset,
        })
    }

    /// Calculates a capped delay from a one-based consecutive failure count
    /// and a uniformly distributed sample in `0..=10_000`.
    ///
    /// # Errors
    ///
    /// Returns an error when the sample is outside its documented range.
    pub fn delay(
        self,
        consecutive_failures: NonZeroU32,
        jitter_sample: u16,
    ) -> Result<ReconnectDelay, ReconnectPolicyError> {
        if jitter_sample > 10_000 {
            return Err(ReconnectPolicyError::InvalidJitterSample);
        }
        let exponent = consecutive_failures.get().saturating_sub(1).min(127);
        let nominal = self
            .initial
            .as_nanos()
            .checked_shl(exponent)
            .unwrap_or(u128::MAX)
            .min(self.maximum.as_nanos());
        let jitter = u128::from(self.jitter_basis_points);
        let factor =
            10_000_u128 - jitter + (2_u128 * jitter * u128::from(jitter_sample) / 10_000_u128);
        let nanos = (nominal.saturating_mul(factor) / 10_000_u128)
            .min(self.maximum.as_nanos())
            .max(1);
        let nanos = u64::try_from(nanos).unwrap_or(u64::MAX);
        ReconnectDelay::new(Duration::from_nanos(nanos))
            .map_err(|_| ReconnectPolicyError::ZeroInitial)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReconnectPolicyError {
    ZeroInitial,
    MaximumBelowInitial,
    JitterAboveOneHundredPercent,
    ZeroTeardownTimeout,
    ZeroStableSessionReset,
    InvalidJitterSample,
}

impl fmt::Display for ReconnectPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid reconnect policy: {self:?}")
    }
}

impl Error for ReconnectPolicyError {}

/// Source of an independent bounded jitter sample for each failed attempt.
pub trait JitterSource {
    fn sample_basis_points(&mut self) -> u16;
}

/// Operational counters returned when the owned supervisor exits.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SupervisorCounters {
    pub access_units: u64,
    pub packet_loss_events: u64,
    pub lost_packets: u64,
    pub live_evictions: u64,
    pub teardown_failures: u64,
}

/// Why the one-camera owner stopped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupervisorStopReason {
    Requested,
    TerminalConnection,
    RecorderBackpressure,
    LifecycleExhausted,
}

/// Final ownership and evidence returned by the supervisor task.
pub struct SupervisorExit {
    pub reason: SupervisorStopReason,
    pub snapshot: ConnectionSnapshot,
    pub counters: SupervisorCounters,
    pub distributor: MediaDistributor<H264AccessUnit>,
}

/// Runs one camera until requested shutdown, terminal connection failure, or
/// recorder overload. The returned future owns the only upstream session.
#[allow(
    clippy::too_many_lines,
    reason = "the linear typestate transitions keep session ownership and cancellation auditable"
)]
pub async fn run_h264_camera<J: JitterSource>(
    camera_id: CameraId,
    source: &RtspSourceConfig,
    distributor: MediaDistributor<H264AccessUnit>,
    policy: ReconnectPolicy,
    jitter: &mut J,
    shutdown: watch::Receiver<bool>,
) -> SupervisorExit {
    let (health, _reader) = camera_health_channel(camera_id.clone());
    run_h264_camera_observed(
        camera_id,
        source,
        distributor,
        policy,
        jitter,
        shutdown,
        health,
    )
    .await
}

/// Runs one camera while publishing its latest lifecycle snapshot through a
/// constant-memory health channel.
#[allow(
    clippy::too_many_lines,
    reason = "the linear typestate transitions keep session ownership and cancellation auditable"
)]
pub async fn run_h264_camera_observed<J: JitterSource>(
    camera_id: CameraId,
    source: &RtspSourceConfig,
    distributor: MediaDistributor<H264AccessUnit>,
    policy: ReconnectPolicy,
    jitter: &mut J,
    mut shutdown: watch::Receiver<bool>,
    health: CameraHealthPublisher,
) -> SupervisorExit {
    let log_camera_id = camera_id.clone();
    tracing::info!(camera_id = %log_camera_id, state = "starting", "camera supervisor started");
    let disconnected = DisconnectedCamera::new(camera_id);
    let initial_snapshot = disconnected.snapshot();
    health.publish(initial_snapshot.clone());
    let Ok(mut connecting) = disconnected.begin_connect() else {
        return observed_exit(
            SupervisorStopReason::LifecycleExhausted,
            initial_snapshot,
            SupervisorCounters::default(),
            distributor,
            &health,
        );
    };
    health.publish(connecting.snapshot());
    let mut counters = SupervisorCounters::default();
    let mut consecutive_failures = 0_u32;

    loop {
        if shutdown_requested(&shutdown) {
            return observed_exit(
                SupervisorStopReason::Requested,
                connecting.stop().snapshot(),
                counters,
                distributor,
                &health,
            );
        }
        let connection = tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || shutdown_requested(&shutdown) {
                    return observed_exit(SupervisorStopReason::Requested, connecting.stop().snapshot(), counters, distributor, &health);
                }
                continue;
            }
            connection = connect_h264(source) => connection,
        };

        let session = match connection {
            Ok(session) => session,
            Err(RtspConnectError::Terminal { failure, .. }) => {
                tracing::error!(camera_id = %log_camera_id, ?failure, state = "terminal", "camera connection rejected");
                let stopped = connecting.fail(failure);
                return observed_exit(
                    SupervisorStopReason::TerminalConnection,
                    stopped.snapshot(),
                    counters,
                    distributor,
                    &health,
                );
            }
            Err(error) => {
                let failure = retryable_connect_failure(&error);
                consecutive_failures = consecutive_failures.saturating_add(1);
                let Some(failure_count) = NonZeroU32::new(consecutive_failures) else {
                    return observed_exit(
                        SupervisorStopReason::LifecycleExhausted,
                        connecting.stop().snapshot(),
                        counters,
                        distributor,
                        &health,
                    );
                };
                let Ok(delay) = policy.delay(failure_count, jitter.sample_basis_points()) else {
                    return observed_exit(
                        SupervisorStopReason::LifecycleExhausted,
                        connecting.stop().snapshot(),
                        counters,
                        distributor,
                        &health,
                    );
                };
                let reconnecting = connecting.retry_after(failure, delay);
                health.publish(reconnecting.snapshot());
                tracing::warn!(camera_id = %log_camera_id, ?failure, ?delay, state = "reconnecting", "camera connection failed; retry scheduled");
                match wait_for_retry(delay, &mut shutdown).await {
                    RetryWait::Elapsed => match reconnecting.retry_now() {
                        Ok(next) => {
                            connecting = next;
                            continue;
                        }
                        Err(stopped) => {
                            return observed_exit(
                                SupervisorStopReason::LifecycleExhausted,
                                stopped.snapshot(),
                                counters,
                                distributor,
                                &health,
                            );
                        }
                    },
                    RetryWait::Shutdown => {
                        return observed_exit(
                            SupervisorStopReason::Requested,
                            reconnecting.stop().snapshot(),
                            counters,
                            distributor,
                            &health,
                        );
                    }
                }
            }
        };

        let mut connected = connecting.connected(session, std::time::SystemTime::now());
        let connected_monotonic = tokio::time::Instant::now();
        health.publish(connected.snapshot());
        tracing::info!(camera_id = %log_camera_id, state = "connected", "camera session connected");
        loop {
            let outcome = tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || shutdown_requested(&shutdown) {
                        let (stopped, session) = connected.stop_with_session();
                        if session.shutdown(policy.teardown_timeout).await.is_err() {
                            counters.teardown_failures = counters.teardown_failures.saturating_add(1);
                            health.teardown_failure();
                        }
                        return observed_exit(SupervisorStopReason::Requested, stopped.snapshot(), counters, distributor, &health);
                    }
                    continue;
                }
                outcome = connected.session_mut().next() => outcome,
            };

            let retry_failure = match outcome {
                Ok(RtspReadOutcome::AccessUnit(received)) => {
                    let report = distributor.publish(Arc::clone(&received.access_unit));
                    counters.access_units = counters.access_units.saturating_add(1);
                    health.access_unit();
                    if report.live == PublishOutcome::ReplacedOldest {
                        counters.live_evictions = counters.live_evictions.saturating_add(1);
                        health.live_eviction();
                    }
                    if report.recorder == PublishOutcome::Full {
                        tracing::error!(camera_id = %log_camera_id, state = "stopping", "recorder queue saturated; stopping camera ingest");
                        let (stopped, session) = connected.stop_with_session();
                        if session.shutdown(policy.teardown_timeout).await.is_err() {
                            counters.teardown_failures =
                                counters.teardown_failures.saturating_add(1);
                            health.teardown_failure();
                        }
                        return observed_exit(
                            SupervisorStopReason::RecorderBackpressure,
                            stopped.snapshot(),
                            counters,
                            distributor,
                            &health,
                        );
                    }
                    continue;
                }
                Ok(RtspReadOutcome::PacketLoss { lost_packets }) => {
                    counters.packet_loss_events = counters.packet_loss_events.saturating_add(1);
                    counters.lost_packets = counters
                        .lost_packets
                        .saturating_add(u64::from(lost_packets));
                    health.packet_loss(lost_packets);
                    continue;
                }
                Ok(RtspReadOutcome::EndOfStream) => RetryableCameraFailure::EndOfStream,
                Err(_) => RetryableCameraFailure::TransportProtocol,
            };

            let Some(failure_count) = next_session_failure_count(
                &mut consecutive_failures,
                connected_monotonic.elapsed(),
                policy.stable_session_reset,
            ) else {
                let (stopped, session) = connected.stop_with_session();
                drop(session);
                return observed_exit(
                    SupervisorStopReason::LifecycleExhausted,
                    stopped.snapshot(),
                    counters,
                    distributor,
                    &health,
                );
            };
            let Ok(delay) = policy.delay(failure_count, jitter.sample_basis_points()) else {
                let (stopped, session) = connected.stop_with_session();
                drop(session);
                return observed_exit(
                    SupervisorStopReason::LifecycleExhausted,
                    stopped.snapshot(),
                    counters,
                    distributor,
                    &health,
                );
            };
            let (reconnecting, session) = connected.lost_with_session(retry_failure, delay);
            health.publish(reconnecting.snapshot());
            tracing::warn!(camera_id = %log_camera_id, ?retry_failure, ?delay, state = "reconnecting", "camera session lost; retry scheduled");
            if session.shutdown(policy.teardown_timeout).await.is_err() {
                counters.teardown_failures = counters.teardown_failures.saturating_add(1);
                health.teardown_failure();
            }
            match wait_for_retry(delay, &mut shutdown).await {
                RetryWait::Elapsed => match reconnecting.retry_now() {
                    Ok(next) => {
                        connecting = next;
                        break;
                    }
                    Err(stopped) => {
                        return observed_exit(
                            SupervisorStopReason::LifecycleExhausted,
                            stopped.snapshot(),
                            counters,
                            distributor,
                            &health,
                        );
                    }
                },
                RetryWait::Shutdown => {
                    return observed_exit(
                        SupervisorStopReason::Requested,
                        reconnecting.stop().snapshot(),
                        counters,
                        distributor,
                        &health,
                    );
                }
            }
        }
    }
}

fn next_session_failure_count(
    consecutive_failures: &mut u32,
    session_duration: Duration,
    stable_session_reset: Duration,
) -> Option<NonZeroU32> {
    if session_duration >= stable_session_reset {
        *consecutive_failures = 0;
    }
    *consecutive_failures = consecutive_failures.saturating_add(1);
    NonZeroU32::new(*consecutive_failures)
}

fn observed_exit(
    reason: SupervisorStopReason,
    snapshot: ConnectionSnapshot,
    counters: SupervisorCounters,
    distributor: MediaDistributor<H264AccessUnit>,
    health: &CameraHealthPublisher,
) -> SupervisorExit {
    health.publish(snapshot.clone());
    SupervisorExit {
        reason,
        snapshot,
        counters,
        distributor,
    }
}

fn retryable_connect_failure(error: &RtspConnectError) -> RetryableCameraFailure {
    match error {
        RtspConnectError::Timeout => RetryableCameraFailure::Timeout,
        RtspConnectError::Retryable { failure, .. } => *failure,
        RtspConnectError::Terminal { .. } => RetryableCameraFailure::TransportProtocol,
    }
}

fn shutdown_requested(shutdown: &watch::Receiver<bool>) -> bool {
    *shutdown.borrow()
}

enum RetryWait {
    Elapsed,
    Shutdown,
}

async fn wait_for_retry(delay: ReconnectDelay, shutdown: &mut watch::Receiver<bool>) -> RetryWait {
    if shutdown_requested(shutdown) {
        return RetryWait::Shutdown;
    }
    let sleep = tokio::time::sleep(delay.get());
    tokio::pin!(sleep);
    loop {
        tokio::select! {
            () = &mut sleep => return RetryWait::Elapsed,
            changed = shutdown.changed() => {
                if changed.is_err() || shutdown_requested(shutdown) {
                    return RetryWait::Shutdown;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{next_session_failure_count, ReconnectPolicy, ReconnectPolicyError};
    use crate::connection::ReconnectDelay;
    use std::{num::NonZeroU32, time::Duration};

    fn failures(value: u32) -> NonZeroU32 {
        NonZeroU32::new(value).unwrap_or_else(|| panic!("test failure count must be nonzero"))
    }

    #[test]
    fn backoff_is_exponential_jittered_and_capped() {
        let policy = ReconnectPolicy::new(
            Duration::from_secs(1),
            Duration::from_secs(8),
            2_000,
            Duration::from_secs(2),
            Duration::from_secs(30),
        )
        .unwrap_or_else(|error| panic!("valid policy: {error}"));
        assert_eq!(
            policy.delay(failures(1), 0).map(ReconnectDelay::get),
            Ok(Duration::from_millis(800))
        );
        assert_eq!(
            policy.delay(failures(2), 5_000).map(ReconnectDelay::get),
            Ok(Duration::from_secs(2))
        );
        assert_eq!(
            policy.delay(failures(4), 10_000).map(ReconnectDelay::get),
            Ok(Duration::from_secs(8))
        );
        assert_eq!(
            policy.delay(failures(30), 5_000).map(ReconnectDelay::get),
            Ok(Duration::from_secs(8))
        );
    }

    #[test]
    fn backoff_rejects_invalid_bounds_and_samples() {
        assert_eq!(
            ReconnectPolicy::new(
                Duration::ZERO,
                Duration::from_secs(1),
                0,
                Duration::from_secs(1),
                Duration::from_secs(30)
            ),
            Err(ReconnectPolicyError::ZeroInitial)
        );
        let policy = ReconnectPolicy::new(
            Duration::from_secs(1),
            Duration::from_secs(2),
            0,
            Duration::from_secs(1),
            Duration::from_secs(30),
        )
        .unwrap_or_else(|error| panic!("valid policy: {error}"));
        assert_eq!(
            policy.delay(failures(1), 10_001),
            Err(ReconnectPolicyError::InvalidJitterSample)
        );
    }

    #[test]
    fn stable_session_resets_consecutive_failure_backoff() {
        let mut consecutive = 4;
        assert_eq!(
            next_session_failure_count(
                &mut consecutive,
                Duration::from_secs(29),
                Duration::from_secs(30)
            )
            .map(NonZeroU32::get),
            Some(5)
        );
        assert_eq!(
            next_session_failure_count(
                &mut consecutive,
                Duration::from_secs(30),
                Duration::from_secs(30)
            )
            .map(NonZeroU32::get),
            Some(1)
        );
    }
}
