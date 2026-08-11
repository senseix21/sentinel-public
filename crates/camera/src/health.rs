//! Bounded latest-value per-camera lifecycle health and counters.

use std::sync::{
    atomic::{AtomicU64, AtomicU8, Ordering},
    Arc,
};
use std::time::{Duration, SystemTime};

use surveillance_common::types::CameraId;
use tokio::sync::watch;

use crate::connection::{
    ConnectionSnapshot, ConnectionState, DisconnectedCamera, RetryableCameraFailure, StopReason,
    TerminalCameraFailure,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionableCameraFailure {
    NetworkUnavailable,
    Timeout,
    EndOfStream,
    ServerUnavailable,
    TransportProtocol,
    AuthenticationRejected,
    InvalidConfiguration,
    UnsupportedMedia,
    LifecycleExhausted,
}

pub struct CameraHealthPublisher {
    sender: watch::Sender<ConnectionSnapshot>,
    metrics: Arc<CameraMetricState>,
}

impl CameraHealthPublisher {
    pub(crate) fn publish(&self, snapshot: ConnectionSnapshot) {
        match snapshot.state() {
            ConnectionState::Connected { connected_at, .. } => {
                self.metrics
                    .last_connected_millis
                    .store(system_time_millis(connected_at), Ordering::Relaxed);
            }
            ConnectionState::Reconnecting { failure, .. } => self
                .metrics
                .last_failure
                .store(retryable_failure_code(failure), Ordering::Relaxed),
            ConnectionState::Stopped {
                reason: StopReason::Terminal(failure),
            } => self
                .metrics
                .last_failure
                .store(terminal_failure_code(failure), Ordering::Relaxed),
            ConnectionState::Stopped {
                reason: StopReason::LifecycleExhausted(_),
            } => self.metrics.last_failure.store(9, Ordering::Relaxed),
            ConnectionState::Disconnected
            | ConnectionState::Connecting { .. }
            | ConnectionState::Stopped {
                reason: StopReason::Requested,
            } => {}
        }
        self.sender.send_replace(snapshot);
    }

    pub(crate) fn access_unit(&self) {
        self.metrics.access_units.fetch_add(1, Ordering::Relaxed);
        self.metrics
            .last_media_millis
            .store(system_time_millis(SystemTime::now()), Ordering::Relaxed);
    }

    pub(crate) fn packet_loss(&self, lost_packets: u16) {
        self.metrics
            .packet_loss_events
            .fetch_add(1, Ordering::Relaxed);
        self.metrics
            .lost_packets
            .fetch_add(u64::from(lost_packets), Ordering::Relaxed);
    }

    pub(crate) fn live_eviction(&self) {
        self.metrics.live_evictions.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn teardown_failure(&self) {
        self.metrics
            .teardown_failures
            .fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Clone)]
pub struct CameraHealthReader {
    receiver: watch::Receiver<ConnectionSnapshot>,
    metrics: Arc<CameraMetricState>,
}

impl CameraHealthReader {
    #[must_use]
    pub fn snapshot(&self) -> ConnectionSnapshot {
        self.receiver.borrow().clone()
    }

    #[must_use]
    pub fn operational_metrics(&self) -> CameraOperationalMetrics {
        CameraOperationalMetrics {
            access_units: self.metrics.access_units.load(Ordering::Relaxed),
            packet_loss_events: self.metrics.packet_loss_events.load(Ordering::Relaxed),
            lost_packets: self.metrics.lost_packets.load(Ordering::Relaxed),
            live_evictions: self.metrics.live_evictions.load(Ordering::Relaxed),
            teardown_failures: self.metrics.teardown_failures.load(Ordering::Relaxed),
            last_connected_at: optional_system_time(
                self.metrics.last_connected_millis.load(Ordering::Relaxed),
            ),
            last_media_at: optional_system_time(
                self.metrics.last_media_millis.load(Ordering::Relaxed),
            ),
            last_actionable_failure: failure_from_code(
                self.metrics.last_failure.load(Ordering::Relaxed),
            ),
        }
    }

    /// Waits for a lifecycle transition.
    ///
    /// # Errors
    ///
    /// Returns an error after the publisher is dropped.
    pub async fn changed(&mut self) -> Result<ConnectionSnapshot, watch::error::RecvError> {
        self.receiver.changed().await?;
        Ok(self.snapshot())
    }
}

#[must_use]
pub fn camera_health_channel(camera_id: CameraId) -> (CameraHealthPublisher, CameraHealthReader) {
    let initial = DisconnectedCamera::new(camera_id).snapshot();
    let (sender, receiver) = watch::channel(initial);
    let metrics = Arc::new(CameraMetricState::default());
    (
        CameraHealthPublisher {
            sender,
            metrics: Arc::clone(&metrics),
        },
        CameraHealthReader { receiver, metrics },
    )
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CameraOperationalMetrics {
    pub access_units: u64,
    pub packet_loss_events: u64,
    pub lost_packets: u64,
    pub live_evictions: u64,
    pub teardown_failures: u64,
    pub last_connected_at: Option<SystemTime>,
    pub last_media_at: Option<SystemTime>,
    pub last_actionable_failure: Option<ActionableCameraFailure>,
}

#[derive(Default)]
struct CameraMetricState {
    access_units: AtomicU64,
    packet_loss_events: AtomicU64,
    lost_packets: AtomicU64,
    live_evictions: AtomicU64,
    teardown_failures: AtomicU64,
    last_connected_millis: AtomicU64,
    last_media_millis: AtomicU64,
    last_failure: AtomicU8,
}

fn system_time_millis(value: SystemTime) -> u64 {
    value
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
        .max(1)
}

fn optional_system_time(millis: u64) -> Option<SystemTime> {
    (millis != 0).then(|| SystemTime::UNIX_EPOCH + Duration::from_millis(millis))
}

const fn retryable_failure_code(failure: RetryableCameraFailure) -> u8 {
    match failure {
        RetryableCameraFailure::NetworkUnavailable => 1,
        RetryableCameraFailure::Timeout => 2,
        RetryableCameraFailure::EndOfStream => 3,
        RetryableCameraFailure::ServerUnavailable => 4,
        RetryableCameraFailure::TransportProtocol => 5,
    }
}

const fn terminal_failure_code(failure: TerminalCameraFailure) -> u8 {
    match failure {
        TerminalCameraFailure::AuthenticationRejected => 6,
        TerminalCameraFailure::InvalidConfiguration => 7,
        TerminalCameraFailure::UnsupportedMedia => 8,
    }
}

const fn failure_from_code(code: u8) -> Option<ActionableCameraFailure> {
    match code {
        1 => Some(ActionableCameraFailure::NetworkUnavailable),
        2 => Some(ActionableCameraFailure::Timeout),
        3 => Some(ActionableCameraFailure::EndOfStream),
        4 => Some(ActionableCameraFailure::ServerUnavailable),
        5 => Some(ActionableCameraFailure::TransportProtocol),
        6 => Some(ActionableCameraFailure::AuthenticationRejected),
        7 => Some(ActionableCameraFailure::InvalidConfiguration),
        8 => Some(ActionableCameraFailure::UnsupportedMedia),
        9 => Some(ActionableCameraFailure::LifecycleExhausted),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{camera_health_channel, ActionableCameraFailure};
    use crate::connection::{ConnectionState, ReconnectDelay, RetryableCameraFailure};
    use surveillance_common::types::CameraId;

    #[tokio::test]
    async fn retains_latest_transition_counters_and_closes_with_publisher() {
        let camera_id =
            CameraId::new("camera-01").unwrap_or_else(|error| panic!("valid test ID: {error}"));
        let (publisher, mut reader) = camera_health_channel(camera_id.clone());
        let connecting = crate::connection::DisconnectedCamera::new(camera_id)
            .begin_connect()
            .unwrap_or_else(|_| panic!("fresh lifecycle must connect"));
        publisher.publish(connecting.snapshot());
        publisher.access_unit();
        publisher.packet_loss(3);
        assert!(matches!(
            reader.changed().await.map(|snapshot| snapshot.state()),
            Ok(ConnectionState::Connecting { .. })
        ));
        let metrics = reader.operational_metrics();
        assert_eq!(metrics.access_units, 1);
        assert_eq!(metrics.packet_loss_events, 1);
        assert_eq!(metrics.lost_packets, 3);
        drop(publisher);
        assert!(reader.changed().await.is_err());
    }

    #[test]
    fn retains_last_connected_media_and_bounded_failure_category() {
        let camera_id =
            CameraId::new("camera-02").unwrap_or_else(|error| panic!("valid test ID: {error}"));
        let (publisher, reader) = camera_health_channel(camera_id.clone());
        let connected = crate::connection::DisconnectedCamera::new(camera_id)
            .begin_connect()
            .unwrap_or_else(|_| panic!("connect transition"))
            .connected((), std::time::SystemTime::now());
        publisher.publish(connected.snapshot());
        publisher.access_unit();
        let delay = ReconnectDelay::new(std::time::Duration::from_secs(1))
            .unwrap_or_else(|error| panic!("valid delay: {error}"));
        let reconnecting = connected.lost(RetryableCameraFailure::Timeout, delay);
        publisher.publish(reconnecting.snapshot());
        let metrics = reader.operational_metrics();
        assert!(metrics.last_connected_at.is_some());
        assert!(metrics.last_media_at.is_some());
        assert_eq!(
            metrics.last_actionable_failure,
            Some(ActionableCameraFailure::Timeout)
        );
    }
}
