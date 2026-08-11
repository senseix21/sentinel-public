//! Camera connection lifecycle with state-specific ownership.

use std::{
    error::Error,
    fmt,
    num::{NonZeroU32, NonZeroU64},
    time::{Duration, SystemTime},
};

use surveillance_common::types::CameraId;

/// A successful upstream session generation for one camera process lifetime.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ConnectionEpoch(NonZeroU64);

impl ConnectionEpoch {
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// A cumulative connection-attempt number for one camera lifecycle owner.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ConnectAttempt(NonZeroU32);

impl ConnectAttempt {
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

/// A validated nonzero delay before the next connection attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReconnectDelay(Duration);

impl ReconnectDelay {
    /// Validates a reconnect delay.
    ///
    /// # Errors
    ///
    /// Returns [`ReconnectDelayError`] when the delay is zero.
    pub fn new(delay: Duration) -> Result<Self, ReconnectDelayError> {
        if delay.is_zero() {
            return Err(ReconnectDelayError);
        }
        Ok(Self(delay))
    }

    #[must_use]
    pub const fn get(self) -> Duration {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReconnectDelayError;

impl fmt::Display for ReconnectDelayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("reconnect delay must be greater than zero")
    }
}

impl Error for ReconnectDelayError {}

/// Failures for which reconnect is meaningful.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryableCameraFailure {
    NetworkUnavailable,
    Timeout,
    EndOfStream,
    ServerUnavailable,
    TransportProtocol,
}

/// Failures requiring configuration/operator action rather than blind retry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalCameraFailure {
    AuthenticationRejected,
    InvalidConfiguration,
    UnsupportedMedia,
}

/// Saturating lifecycle counters suitable for later metric export.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ConnectionCounters {
    /// Connection attempts begun, including reconnect attempts.
    pub attempts: u64,
    /// Sessions successfully established.
    pub connected: u64,
    /// Connected sessions lost and scheduled for retry.
    pub reconnects: u64,
    /// Explicit clean stops.
    pub clean_stops: u64,
    /// Operator-actionable terminal failures.
    pub terminal_failures: u64,
}

/// Credential-free lifecycle state suitable for health and metrics boundaries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionState {
    Disconnected,
    Connecting {
        attempt: ConnectAttempt,
    },
    Connected {
        epoch: ConnectionEpoch,
        connected_at: SystemTime,
    },
    Reconnecting {
        failure: RetryableCameraFailure,
        delay: ReconnectDelay,
    },
    Stopped {
        reason: StopReason,
    },
}

/// A point-in-time, camera-scoped lifecycle view.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectionSnapshot {
    camera_id: CameraId,
    state: ConnectionState,
    counters: ConnectionCounters,
}

impl ConnectionSnapshot {
    #[must_use]
    pub fn camera_id(&self) -> &CameraId {
        &self.camera_id
    }

    #[must_use]
    pub const fn state(&self) -> ConnectionState {
        self.state
    }

    #[must_use]
    pub const fn counters(&self) -> ConnectionCounters {
        self.counters
    }
}

#[derive(Debug)]
struct LifecycleCore {
    camera_id: CameraId,
    last_epoch: u64,
    last_attempt: u32,
    counters: ConnectionCounters,
}

impl LifecycleCore {
    fn begin_attempt(&mut self) -> Result<(ConnectAttempt, ConnectionEpoch), LifecycleExhausted> {
        let attempt = self
            .last_attempt
            .checked_add(1)
            .and_then(NonZeroU32::new)
            .ok_or(LifecycleExhausted::Attempts)?;
        let epoch = self
            .last_epoch
            .checked_add(1)
            .and_then(NonZeroU64::new)
            .ok_or(LifecycleExhausted::Epochs)?;
        self.last_attempt = attempt.get();
        self.counters.attempts = self.counters.attempts.saturating_add(1);
        Ok((ConnectAttempt(attempt), ConnectionEpoch(epoch)))
    }
}

/// Initial state: no upstream session exists.
#[derive(Debug)]
pub struct DisconnectedCamera {
    core: LifecycleCore,
}

impl DisconnectedCamera {
    #[must_use]
    pub fn new(camera_id: CameraId) -> Self {
        Self {
            core: LifecycleCore {
                camera_id,
                last_epoch: 0,
                last_attempt: 0,
                counters: ConnectionCounters::default(),
            },
        }
    }

    /// Begins the first connection attempt.
    ///
    /// # Errors
    ///
    /// Returns [`LifecycleExhausted`] if attempt or epoch identity space is
    /// exhausted.
    pub fn begin_connect(mut self) -> Result<ConnectingCamera, LifecycleExhausted> {
        let (attempt, candidate_epoch) = self.core.begin_attempt()?;
        Ok(ConnectingCamera {
            core: self.core,
            attempt,
            candidate_epoch,
        })
    }

    #[must_use]
    pub fn camera_id(&self) -> &CameraId {
        &self.core.camera_id
    }

    #[must_use]
    pub fn snapshot(&self) -> ConnectionSnapshot {
        snapshot(&self.core, ConnectionState::Disconnected)
    }
}

/// A connection attempt in progress. It owns the epoch reserved for success.
#[derive(Debug)]
pub struct ConnectingCamera {
    core: LifecycleCore,
    attempt: ConnectAttempt,
    candidate_epoch: ConnectionEpoch,
}

impl ConnectingCamera {
    #[must_use]
    pub const fn attempt(&self) -> ConnectAttempt {
        self.attempt
    }

    #[must_use]
    pub fn snapshot(&self) -> ConnectionSnapshot {
        snapshot(
            &self.core,
            ConnectionState::Connecting {
                attempt: self.attempt,
            },
        )
    }

    /// Establishes a connected state that necessarily owns an active session.
    #[must_use]
    pub fn connected<S>(mut self, session: S, connected_at: SystemTime) -> ConnectedCamera<S> {
        self.core.last_epoch = self.candidate_epoch.get();
        self.core.counters.connected = self.core.counters.connected.saturating_add(1);
        ConnectedCamera {
            core: self.core,
            session,
            epoch: self.candidate_epoch,
            connected_at,
        }
    }

    /// Schedules a retry after a transient connection failure.
    #[must_use]
    pub fn retry_after(
        self,
        failure: RetryableCameraFailure,
        delay: ReconnectDelay,
    ) -> ReconnectingCamera {
        ReconnectingCamera {
            core: self.core,
            failure,
            delay,
        }
    }

    /// Stops after a permanent connection failure.
    #[must_use]
    pub fn fail(mut self, failure: TerminalCameraFailure) -> StoppedCamera {
        self.core.counters.terminal_failures =
            self.core.counters.terminal_failures.saturating_add(1);
        StoppedCamera {
            core: self.core,
            reason: StopReason::Terminal(failure),
        }
    }

    /// Cancels a connection attempt cleanly.
    #[must_use]
    pub fn stop(mut self) -> StoppedCamera {
        self.core.counters.clean_stops = self.core.counters.clean_stops.saturating_add(1);
        StoppedCamera {
            core: self.core,
            reason: StopReason::Requested,
        }
    }
}

/// Connected state. A connected camera without an owned session is
/// unrepresentable.
#[derive(Debug)]
pub struct ConnectedCamera<S> {
    core: LifecycleCore,
    session: S,
    epoch: ConnectionEpoch,
    connected_at: SystemTime,
}

impl<S> ConnectedCamera<S> {
    #[must_use]
    pub const fn epoch(&self) -> ConnectionEpoch {
        self.epoch
    }

    #[must_use]
    pub const fn connected_at(&self) -> SystemTime {
        self.connected_at
    }

    #[must_use]
    pub const fn session(&self) -> &S {
        &self.session
    }

    #[must_use]
    pub fn session_mut(&mut self) -> &mut S {
        &mut self.session
    }

    #[must_use]
    pub const fn counters(&self) -> ConnectionCounters {
        self.core.counters
    }

    #[must_use]
    pub fn snapshot(&self) -> ConnectionSnapshot {
        snapshot(
            &self.core,
            ConnectionState::Connected {
                epoch: self.epoch,
                connected_at: self.connected_at,
            },
        )
    }

    /// Drops the failed session and schedules an independent reconnect.
    #[must_use]
    pub fn lost(
        mut self,
        failure: RetryableCameraFailure,
        delay: ReconnectDelay,
    ) -> ReconnectingCamera {
        self.core.counters.reconnects = self.core.counters.reconnects.saturating_add(1);
        ReconnectingCamera {
            core: self.core,
            failure,
            delay,
        }
    }

    /// Returns the failed session to its supervisor for explicit asynchronous
    /// teardown while advancing the lifecycle to reconnecting.
    #[must_use]
    pub fn lost_with_session(
        mut self,
        failure: RetryableCameraFailure,
        delay: ReconnectDelay,
    ) -> (ReconnectingCamera, S) {
        self.core.counters.reconnects = self.core.counters.reconnects.saturating_add(1);
        (
            ReconnectingCamera {
                core: self.core,
                failure,
                delay,
            },
            self.session,
        )
    }

    /// Drops the active session and stops cleanly.
    #[must_use]
    pub fn stop(mut self) -> StoppedCamera {
        self.core.counters.clean_stops = self.core.counters.clean_stops.saturating_add(1);
        StoppedCamera {
            core: self.core,
            reason: StopReason::Requested,
        }
    }

    /// Returns the active session for explicit asynchronous teardown while
    /// advancing the lifecycle to a clean stop.
    #[must_use]
    pub fn stop_with_session(mut self) -> (StoppedCamera, S) {
        self.core.counters.clean_stops = self.core.counters.clean_stops.saturating_add(1);
        (
            StoppedCamera {
                core: self.core,
                reason: StopReason::Requested,
            },
            self.session,
        )
    }
}

/// Waiting state between transiently failed attempts.
#[derive(Debug)]
pub struct ReconnectingCamera {
    core: LifecycleCore,
    failure: RetryableCameraFailure,
    delay: ReconnectDelay,
}

impl ReconnectingCamera {
    #[must_use]
    pub const fn failure(&self) -> RetryableCameraFailure {
        self.failure
    }

    #[must_use]
    pub const fn delay(&self) -> ReconnectDelay {
        self.delay
    }

    #[must_use]
    pub const fn counters(&self) -> ConnectionCounters {
        self.core.counters
    }

    #[must_use]
    pub fn snapshot(&self) -> ConnectionSnapshot {
        snapshot(
            &self.core,
            ConnectionState::Reconnecting {
                failure: self.failure,
                delay: self.delay,
            },
        )
    }

    /// Begins the next attempt and reserves the next epoch on success.
    ///
    /// # Errors
    ///
    /// Returns a stopped lifecycle retaining the exhaustion reason if attempt
    /// or epoch identity space is exhausted.
    pub fn retry_now(mut self) -> Result<ConnectingCamera, StoppedCamera> {
        let (attempt, candidate_epoch) = match self.core.begin_attempt() {
            Ok(identity) => identity,
            Err(error) => {
                return Err(StoppedCamera {
                    core: self.core,
                    reason: StopReason::LifecycleExhausted(error),
                });
            }
        };
        Ok(ConnectingCamera {
            core: self.core,
            attempt,
            candidate_epoch,
        })
    }

    #[must_use]
    pub fn stop(mut self) -> StoppedCamera {
        self.core.counters.clean_stops = self.core.counters.clean_stops.saturating_add(1);
        StoppedCamera {
            core: self.core,
            reason: StopReason::Requested,
        }
    }
}

/// Terminal lifecycle state.
#[derive(Debug)]
pub struct StoppedCamera {
    core: LifecycleCore,
    reason: StopReason,
}

impl StoppedCamera {
    #[must_use]
    pub const fn reason(&self) -> StopReason {
        self.reason
    }

    #[must_use]
    pub const fn counters(&self) -> ConnectionCounters {
        self.core.counters
    }

    #[must_use]
    pub fn camera_id(&self) -> &CameraId {
        &self.core.camera_id
    }

    #[must_use]
    pub fn snapshot(&self) -> ConnectionSnapshot {
        snapshot(
            &self.core,
            ConnectionState::Stopped {
                reason: self.reason,
            },
        )
    }
}

fn snapshot(core: &LifecycleCore, state: ConnectionState) -> ConnectionSnapshot {
    ConnectionSnapshot {
        camera_id: core.camera_id.clone(),
        state,
        counters: core.counters,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StopReason {
    Requested,
    Terminal(TerminalCameraFailure),
    LifecycleExhausted(LifecycleExhausted),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleExhausted {
    Attempts,
    Epochs,
}

impl fmt::Display for LifecycleExhausted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Attempts => formatter.write_str("camera connection attempt space exhausted"),
            Self::Epochs => formatter.write_str("camera connection epoch space exhausted"),
        }
    }
}

impl Error for LifecycleExhausted {}

#[cfg(test)]
mod tests {
    use super::{
        ConnectionState, DisconnectedCamera, ReconnectDelay, ReconnectDelayError,
        RetryableCameraFailure, StopReason, TerminalCameraFailure,
    };
    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        time::{Duration, SystemTime},
    };
    use surveillance_common::types::CameraId;

    #[derive(Debug)]
    struct SessionDropProbe(Arc<AtomicUsize>);

    impl Drop for SessionDropProbe {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn disconnected() -> DisconnectedCamera {
        let camera_id =
            CameraId::new("camera-01").unwrap_or_else(|error| panic!("valid camera ID: {error}"));
        DisconnectedCamera::new(camera_id)
    }

    fn delay() -> ReconnectDelay {
        ReconnectDelay::new(Duration::from_secs(1))
            .unwrap_or_else(|error| panic!("valid reconnect delay: {error}"))
    }

    #[test]
    fn validates_nonzero_reconnect_delay() {
        assert_eq!(
            ReconnectDelay::new(Duration::ZERO),
            Err(ReconnectDelayError)
        );
    }

    #[test]
    fn successful_reconnect_advances_attempt_and_epoch() {
        let drops = Arc::new(AtomicUsize::new(0));
        let connecting = disconnected()
            .begin_connect()
            .unwrap_or_else(|error| panic!("first attempt available: {error}"));
        assert_eq!(connecting.attempt().get(), 1);
        assert!(matches!(
            connecting.snapshot().state(),
            ConnectionState::Connecting { attempt } if attempt.get() == 1
        ));
        let connected = connecting.connected(
            SessionDropProbe(Arc::clone(&drops)),
            SystemTime::UNIX_EPOCH + Duration::from_secs(10),
        );
        assert_eq!(connected.epoch().get(), 1);
        assert!(matches!(
            connected.snapshot().state(),
            ConnectionState::Connected { epoch, .. } if epoch.get() == 1
        ));

        let reconnecting = connected.lost(RetryableCameraFailure::EndOfStream, delay());
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(reconnecting.counters().reconnects, 1);
        let connecting = reconnecting
            .retry_now()
            .unwrap_or_else(|error| panic!("second attempt available: {error:?}"));
        assert_eq!(connecting.attempt().get(), 2);
        let connected = connecting.connected(
            SessionDropProbe(Arc::clone(&drops)),
            SystemTime::UNIX_EPOCH + Duration::from_secs(20),
        );
        assert_eq!(connected.epoch().get(), 2);
        assert_eq!(connected.counters().attempts, 2);
        assert_eq!(connected.counters().connected, 2);
    }

    #[test]
    fn retryable_connect_failure_never_creates_connected_state() {
        let connecting = disconnected()
            .begin_connect()
            .unwrap_or_else(|error| panic!("first attempt available: {error}"));
        let reconnecting =
            connecting.retry_after(RetryableCameraFailure::NetworkUnavailable, delay());
        assert_eq!(
            reconnecting.failure(),
            RetryableCameraFailure::NetworkUnavailable
        );
        assert_eq!(reconnecting.delay().get(), Duration::from_secs(1));
        assert_eq!(reconnecting.counters().connected, 0);
    }

    #[test]
    fn terminal_failure_stops_without_retrying() {
        let connecting = disconnected()
            .begin_connect()
            .unwrap_or_else(|error| panic!("first attempt available: {error}"));
        let stopped = connecting.fail(TerminalCameraFailure::AuthenticationRejected);
        assert_eq!(
            stopped.reason(),
            StopReason::Terminal(TerminalCameraFailure::AuthenticationRejected)
        );
        assert_eq!(stopped.counters().attempts, 1);
        assert_eq!(stopped.counters().terminal_failures, 1);
        assert_eq!(stopped.counters().reconnects, 0);
    }

    #[test]
    fn clean_stop_drops_connected_session_once() {
        let drops = Arc::new(AtomicUsize::new(0));
        let connecting = disconnected()
            .begin_connect()
            .unwrap_or_else(|error| panic!("first attempt available: {error}"));
        let connected =
            connecting.connected(SessionDropProbe(Arc::clone(&drops)), SystemTime::UNIX_EPOCH);
        let stopped = connected.stop();
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(stopped.reason(), StopReason::Requested);
        assert_eq!(stopped.counters().clean_stops, 1);
    }
}
