//! Loopback-only bounded health, metrics, and Phase 3 camera API adapter.

use std::{fmt::Write as _, io, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use serde::{Deserialize, Serialize};
use surveillance_camera::connection::ConnectionState;
use surveillance_common::types::CameraId;
use surveillance_config::settings::{
    CameraCredentialsSettings, OneCameraConfig, OneCameraSettings,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
};

use crate::camera_supervisor::{
    CameraHealthRegistry, CameraStatusSnapshot, CameraSupervisorHandle, PipelineState,
    SupervisorRequestError,
};

const MAX_REQUEST_BYTES: usize = 16 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// Runs one bounded sequential loopback adapter until requested shutdown.
///
/// Listener/client failure is returned to its process owner and never sends a
/// camera-supervisor shutdown command.
///
/// # Errors
///
/// Returns an I/O error for a non-loopback bind, listener failure, or shutdown
/// channel failure. Individual malformed clients are contained and discarded.
pub async fn serve_health(
    address: SocketAddr,
    recording_root: PathBuf,
    registry: CameraHealthRegistry,
    supervisor: CameraSupervisorHandle,
    mut shutdown: watch::Receiver<bool>,
) -> io::Result<()> {
    if !address.ip().is_loopback() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Phase 3 camera API must bind to a loopback address",
        ));
    }
    let listener = TcpListener::bind(address).await?;
    tracing::info!(%address, "loopback health and camera API listening");
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow_and_update() {
                    return Ok(());
                }
            }
            accepted = listener.accept() => {
                let (stream, peer) = accepted?;
                if let Err(error) = handle_client(stream, &recording_root, &registry, &supervisor).await {
                    tracing::debug!(%peer, %error, "API client request discarded");
                }
            }
        }
    }
}

async fn handle_client(
    mut stream: TcpStream,
    recording_root: &std::path::Path,
    registry: &CameraHealthRegistry,
    supervisor: &CameraSupervisorHandle,
) -> io::Result<()> {
    let request = read_request(&mut stream).await?;
    let response = route(&request, recording_root, registry, supervisor).await;
    tokio::time::timeout(IO_TIMEOUT, stream.write_all(response.as_bytes()))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "API response timed out"))??;
    Ok(())
}

async fn read_request(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut request = Vec::with_capacity(1_024);
    loop {
        if request.len() == MAX_REQUEST_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "API request exceeds configured bound",
            ));
        }
        let mut chunk = [0_u8; 1_024];
        let remaining = MAX_REQUEST_BYTES - request.len();
        let read =
            tokio::time::timeout(IO_TIMEOUT, stream.read(&mut chunk[..remaining.min(1_024)]))
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "API request timed out"))??;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..read]);
        if let Some(header_end) = find_header_end(&request) {
            let content_length = content_length(&request[..header_end])?;
            let total = header_end
                .checked_add(4)
                .and_then(|value| value.checked_add(content_length))
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid request size")
                })?;
            if total > MAX_REQUEST_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "API request body exceeds configured bound",
                ));
            }
            if request.len() >= total {
                request.truncate(total);
                return Ok(request);
            }
        }
    }
    Ok(request)
}

fn find_header_end(request: &[u8]) -> Option<usize> {
    request.windows(4).position(|window| window == b"\r\n\r\n")
}

fn content_length(headers: &[u8]) -> io::Result<usize> {
    let headers = std::str::from_utf8(headers)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "request headers are not UTF-8"))?;
    for line in headers.lines().skip(1) {
        if let Some(value) = line
            .split_once(':')
            .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .map(|(_, value)| value.trim())
        {
            return value
                .parse()
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid content length"));
        }
    }
    Ok(0)
}

async fn route(
    request: &[u8],
    recording_root: &std::path::Path,
    registry: &CameraHealthRegistry,
    supervisor: &CameraSupervisorHandle,
) -> String {
    let Some(header_end) = find_header_end(request) else {
        return json_error("400 Bad Request", "invalid_request");
    };
    let Ok(head) = std::str::from_utf8(&request[..header_end]) else {
        return json_error("400 Bad Request", "invalid_request");
    };
    let Some(line) = head.lines().next() else {
        return json_error("400 Bad Request", "invalid_request");
    };
    let mut parts = line.split_whitespace();
    let (Some(method), Some(path), Some(_version)) = (parts.next(), parts.next(), parts.next())
    else {
        return json_error("400 Bad Request", "invalid_request");
    };
    let body = &request[header_end + 4..];
    match (method, path) {
        ("GET", "/live") => response("200 OK", "text/plain; charset=utf-8", "live\n"),
        ("GET", "/ready") => readiness_response(&registry.snapshots()),
        ("GET", "/metrics") => response(
            "200 OK",
            "text/plain; version=0.0.4; charset=utf-8",
            &render_metrics(&registry.snapshots()),
        ),
        ("GET", "/api/cameras") => json_response("200 OK", &camera_list(&registry.snapshots())),
        ("POST", "/api/cameras") => add_camera(body, recording_root, supervisor).await,
        _ => route_camera(method, path, registry, supervisor).await,
    }
}

async fn route_camera(
    method: &str,
    path: &str,
    registry: &CameraHealthRegistry,
    supervisor: &CameraSupervisorHandle,
) -> String {
    let segments: Vec<_> = path.trim_matches('/').split('/').collect();
    if segments.len() < 3 || segments[0] != "api" || segments[1] != "cameras" {
        return json_error("404 Not Found", "not_found");
    }
    let Ok(camera_id) = CameraId::new(segments[2]) else {
        return json_error("400 Bad Request", "invalid_camera_id");
    };
    match (method, segments.as_slice()) {
        ("GET", ["api", "cameras", _] | ["api", "cameras", _, "health"]) => {
            registry.snapshot(&camera_id).map_or_else(
                || json_error("404 Not Found", "camera_not_found"),
                |snapshot| json_response("200 OK", &CameraResponse::from(&snapshot)),
            )
        }
        ("POST", ["api", "cameras", _, "start"]) => {
            operation_response(supervisor.start(camera_id).await, "200 OK")
        }
        ("POST", ["api", "cameras", _, "stop"]) => {
            operation_response(supervisor.stop(camera_id).await, "200 OK")
        }
        ("DELETE", ["api", "cameras", _]) => {
            operation_response(supervisor.remove(camera_id).await, "204 No Content")
        }
        _ => json_error("404 Not Found", "not_found"),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddCameraRequest {
    camera_id: String,
    rtsp_endpoint: String,
    username: Option<String>,
    password: Option<String>,
    width: u32,
    height: u32,
}

async fn add_camera(
    body: &[u8],
    recording_root: &std::path::Path,
    supervisor: &CameraSupervisorHandle,
) -> String {
    let Ok(request) = serde_json::from_slice::<AddCameraRequest>(body) else {
        return json_error("400 Bad Request", "invalid_camera_configuration");
    };
    let credentials = match (request.username, request.password) {
        (None, None) => None,
        (Some(username), Some(password)) => Some(CameraCredentialsSettings { username, password }),
        _ => return json_error("400 Bad Request", "incomplete_camera_credentials"),
    };
    let settings = OneCameraSettings::h264_mvp(
        request.camera_id,
        request.rtsp_endpoint,
        credentials,
        request.width,
        request.height,
        recording_root.to_path_buf(),
    );
    let Ok(config) = OneCameraConfig::validate(settings) else {
        return json_error("400 Bad Request", "invalid_camera_configuration");
    };
    operation_response(supervisor.add(Arc::new(config)).await, "201 Created")
}

fn operation_response(result: Result<(), SupervisorRequestError>, success: &str) -> String {
    match result {
        Ok(()) if success.starts_with("204") => response(success, "application/json", ""),
        Ok(()) => json_response(success, &OperationResponse { status: "ok" }),
        Err(SupervisorRequestError::Operation(error)) => match error {
            crate::camera_supervisor::SupervisorOperationError::DuplicateCamera => {
                json_error("409 Conflict", "camera_already_exists")
            }
            crate::camera_supervisor::SupervisorOperationError::UnknownCamera => {
                json_error("404 Not Found", "camera_not_found")
            }
            crate::camera_supervisor::SupervisorOperationError::RegistryFull => {
                json_error("409 Conflict", "camera_capacity_reached")
            }
            _ => json_error("409 Conflict", "camera_operation_failed"),
        },
        Err(SupervisorRequestError::Full | SupervisorRequestError::TimedOut) => {
            json_error("503 Service Unavailable", "camera_supervisor_busy")
        }
        Err(SupervisorRequestError::Closed) => {
            json_error("503 Service Unavailable", "camera_supervisor_unavailable")
        }
    }
}

#[derive(Serialize)]
struct OperationResponse<'a> {
    status: &'a str,
}

#[derive(Serialize)]
struct ErrorResponse<'a> {
    error: &'a str,
}

#[derive(Serialize)]
struct CameraResponse {
    camera_id: String,
    pipeline_state: &'static str,
    connection_state: Option<&'static str>,
    connection_attempts: u64,
    connection_epochs: u64,
    reconnects: u64,
    terminal_failures: u64,
    last_connected_unix_ms: Option<u64>,
    last_media_unix_ms: Option<u64>,
    last_actionable_failure: Option<String>,
    recorder_queue_depth: Option<usize>,
    recorder_queue_capacity: Option<usize>,
    recorder_rejected: u64,
    live_queue_depth: Option<usize>,
    live_queue_capacity: Option<usize>,
    live_dropped: u64,
    recorder_completed_segments: u64,
    recorder_failed_segments: u64,
}

impl From<&CameraStatusSnapshot> for CameraResponse {
    fn from(snapshot: &CameraStatusSnapshot) -> Self {
        let connection = snapshot.health.as_ref().map(|health| &health.connection);
        let counters =
            connection.map(surveillance_camera::connection::ConnectionSnapshot::counters);
        let operational = snapshot.health.as_ref().map(|health| health.camera);
        Self {
            camera_id: snapshot.camera_id.to_string(),
            pipeline_state: pipeline_state_name(snapshot.pipeline_state),
            connection_state: connection.map(|value| connection_state_name(value.state())),
            connection_attempts: counters.map_or(0, |value| value.attempts),
            connection_epochs: counters.map_or(0, |value| value.connected),
            reconnects: counters.map_or(0, |value| value.reconnects),
            terminal_failures: counters.map_or(0, |value| value.terminal_failures),
            last_connected_unix_ms: operational
                .and_then(|value| value.last_connected_at)
                .map(unix_ms),
            last_media_unix_ms: operational
                .and_then(|value| value.last_media_at)
                .map(unix_ms),
            last_actionable_failure: operational
                .and_then(|value| value.last_actionable_failure)
                .map(|value| format!("{value:?}"))
                .or_else(|| snapshot.last_failure.map(|value| format!("{value:?}"))),
            recorder_queue_depth: snapshot
                .health
                .as_ref()
                .map(|value| value.recorder_queue.depth),
            recorder_queue_capacity: snapshot
                .health
                .as_ref()
                .map(|value| value.recorder_queue.capacity),
            recorder_rejected: snapshot
                .health
                .as_ref()
                .map_or(0, |value| value.recorder_queue.rejected_full),
            live_queue_depth: snapshot.health.as_ref().map(|value| value.live_queue.depth),
            live_queue_capacity: snapshot
                .health
                .as_ref()
                .map(|value| value.live_queue.capacity),
            live_dropped: snapshot
                .health
                .as_ref()
                .map_or(0, |value| value.live_queue.dropped_oldest),
            recorder_completed_segments: snapshot
                .health
                .as_ref()
                .map_or(0, |value| value.recorder.completed_segments),
            recorder_failed_segments: snapshot
                .health
                .as_ref()
                .map_or(0, |value| value.recorder.failed_segments),
        }
    }
}

fn camera_list(snapshots: &[CameraStatusSnapshot]) -> Vec<CameraResponse> {
    snapshots.iter().map(CameraResponse::from).collect()
}

fn readiness_response(snapshots: &[CameraStatusSnapshot]) -> String {
    let ready = !snapshots.is_empty()
        && snapshots.iter().all(|snapshot| {
            snapshot.pipeline_state == PipelineState::Running
                && snapshot.health.as_ref().is_some_and(|health| {
                    matches!(health.connection.state(), ConnectionState::Connected { .. })
                })
        });
    if ready {
        response("200 OK", "text/plain; charset=utf-8", "ready\n")
    } else {
        response(
            "503 Service Unavailable",
            "text/plain; charset=utf-8",
            "one or more cameras are degraded\n",
        )
    }
}

fn render_metrics(snapshots: &[CameraStatusSnapshot]) -> String {
    let mut output = String::with_capacity(snapshots.len().saturating_mul(1_024));
    for status in snapshots {
        let camera = status.camera_id.as_str();
        metric(
            &mut output,
            "surveillance_camera_pipeline_state",
            camera,
            pipeline_state_value(status.pipeline_state),
        );
        let Some(snapshot) = &status.health else {
            continue;
        };
        let connection = snapshot.connection.counters();
        metric(
            &mut output,
            "surveillance_camera_connected",
            camera,
            u8::from(matches!(
                snapshot.connection.state(),
                ConnectionState::Connected { .. }
            )),
        );
        metric(
            &mut output,
            "surveillance_camera_connection_attempts_total",
            camera,
            connection.attempts,
        );
        metric(
            &mut output,
            "surveillance_camera_connection_epochs_total",
            camera,
            connection.connected,
        );
        metric(
            &mut output,
            "surveillance_camera_reconnects_total",
            camera,
            connection.reconnects,
        );
        metric(
            &mut output,
            "surveillance_camera_terminal_failures_total",
            camera,
            connection.terminal_failures,
        );
        metric(
            &mut output,
            "surveillance_camera_access_units_total",
            camera,
            snapshot.camera.access_units,
        );
        metric(
            &mut output,
            "surveillance_camera_packet_loss_events_total",
            camera,
            snapshot.camera.packet_loss_events,
        );
        metric(
            &mut output,
            "surveillance_camera_lost_packets_total",
            camera,
            snapshot.camera.lost_packets,
        );
        metric(
            &mut output,
            "surveillance_camera_live_evictions_total",
            camera,
            snapshot.camera.live_evictions,
        );
        metric(
            &mut output,
            "surveillance_camera_teardown_failures_total",
            camera,
            snapshot.camera.teardown_failures,
        );
        metric(
            &mut output,
            "surveillance_recorder_segments_completed_total",
            camera,
            snapshot.recorder.completed_segments,
        );
        metric(
            &mut output,
            "surveillance_recorder_segments_failed_total",
            camera,
            snapshot.recorder.failed_segments,
        );
        queue_metrics(&mut output, camera, "recorder", snapshot.recorder_queue);
        queue_metrics(&mut output, camera, "live", snapshot.live_queue);
    }
    output
}

fn metric(output: &mut String, name: &str, camera: &str, value: impl std::fmt::Display) {
    let _ = writeln!(output, "{name}{{camera_id=\"{camera}\"}} {value}");
}

fn queue_metrics(
    output: &mut String,
    camera: &str,
    consumer: &str,
    queue: surveillance_media::distribution::QueueMetrics,
) {
    for (suffix, value) in [
        ("capacity", queue.capacity as u64),
        ("depth", queue.depth as u64),
        ("accepted_total", queue.accepted),
        ("received_total", queue.received),
        ("rejected_full_total", queue.rejected_full),
        ("dropped_oldest_total", queue.dropped_oldest),
        ("dropped_on_close_total", queue.dropped_on_close),
    ] {
        let _ = writeln!(output, "surveillance_media_queue_{suffix}{{camera_id=\"{camera}\",consumer=\"{consumer}\"}} {value}");
    }
}

const fn pipeline_state_name(state: PipelineState) -> &'static str {
    match state {
        PipelineState::Configured => "configured",
        PipelineState::Starting => "starting",
        PipelineState::Running => "running",
        PipelineState::Stopping => "stopping",
        PipelineState::Stopped => "stopped",
        PipelineState::Degraded => "degraded",
        PipelineState::Terminal => "terminal",
    }
}

const fn pipeline_state_value(state: PipelineState) -> u8 {
    match state {
        PipelineState::Configured => 0,
        PipelineState::Starting => 1,
        PipelineState::Running => 2,
        PipelineState::Stopping => 3,
        PipelineState::Stopped => 4,
        PipelineState::Degraded => 5,
        PipelineState::Terminal => 6,
    }
}

const fn connection_state_name(state: ConnectionState) -> &'static str {
    match state {
        ConnectionState::Disconnected => "disconnected",
        ConnectionState::Connecting { .. } => "connecting",
        ConnectionState::Connected { .. } => "connected",
        ConnectionState::Reconnecting { .. } => "reconnecting",
        ConnectionState::Stopped { .. } => "stopped",
    }
}

fn unix_ms(value: std::time::SystemTime) -> u64 {
    value
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn json_response(status: &str, value: &impl Serialize) -> String {
    match serde_json::to_string(value) {
        Ok(body) => response(status, "application/json", &body),
        Err(_) => json_error("500 Internal Server Error", "response_encoding_failed"),
    }
}

fn json_error(status: &str, error: &str) -> String {
    json_response(status, &ErrorResponse { error })
}

fn response(status: &str, content_type: &str, body: &str) -> String {
    format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
}

#[cfg(test)]
mod tests {
    use super::{route, serve_health};
    use crate::camera_supervisor::{CameraSupervisor, PipelineState};
    use std::{net::SocketAddr, num::NonZeroUsize, sync::Arc, time::Duration};
    use surveillance_config::settings::{OneCameraConfig, OneCameraSettings};
    use tokio::sync::watch;

    fn supervisor() -> (
        CameraSupervisor,
        crate::camera_supervisor::CameraSupervisorHandle,
    ) {
        CameraSupervisor::new(
            NonZeroUsize::new(8).unwrap_or_else(|| panic!("command bound")),
            NonZeroUsize::new(10).unwrap_or_else(|| panic!("registry bound")),
            Duration::from_secs(2),
        )
    }

    #[tokio::test]
    async fn camera_api_validates_commands_and_never_returns_source_or_credentials() {
        let (supervisor, handle) = supervisor();
        let task = tokio::spawn(supervisor.run());
        let body = br#"{"camera_id":"camera-21","rtsp_endpoint":"rtsp://127.0.0.1:19221/camera-21","username":"operator","password":"top-secret","width":640,"height":360}"#;
        let request = format!(
            "POST /api/cameras HTTP/1.1\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            std::str::from_utf8(body).unwrap_or_else(|error| panic!("test JSON: {error}"))
        );
        let response = route(
            request.as_bytes(),
            std::path::Path::new("target/phase3-tests"),
            &handle.health_registry(),
            &handle,
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 201"));
        let response = route(
            b"GET /api/cameras/camera-21 HTTP/1.1\r\n\r\n",
            std::path::Path::new("target/phase3-tests"),
            &handle.health_registry(),
            &handle,
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("camera-21"));
        assert!(!response.contains("top-secret"));
        assert!(!response.contains("rtsp://"));
        assert!(!response.contains("recording_root"));

        let invalid = route(
            b"POST /api/cameras/../start HTTP/1.1\r\n\r\n",
            std::path::Path::new("target/phase3-tests"),
            &handle.health_registry(),
            &handle,
        )
        .await;
        assert!(invalid.starts_with("HTTP/1.1 400"));
        handle
            .shutdown()
            .await
            .unwrap_or_else(|error| panic!("shutdown: {error}"));
        task.await
            .unwrap_or_else(|error| panic!("supervisor task: {error}"));
    }

    #[tokio::test]
    async fn api_listener_failure_does_not_cancel_pipeline_supervision() {
        let (supervisor, handle) = supervisor();
        let task = tokio::spawn(supervisor.run());
        let mut settings = OneCameraSettings::h264_mvp(
            "camera-22".to_owned(),
            "rtsp://127.0.0.1:19222/camera-22".to_owned(),
            None,
            640,
            360,
            "target/phase3-tests/camera-22".into(),
        );
        settings.connect_timeout = Duration::from_millis(50);
        let config = Arc::new(
            OneCameraConfig::validate(settings)
                .unwrap_or_else(|error| panic!("valid config: {error}")),
        );
        handle
            .add(Arc::clone(&config))
            .await
            .unwrap_or_else(|error| panic!("add: {error}"));
        handle
            .start(config.camera_id.clone())
            .await
            .unwrap_or_else(|error| panic!("start: {error}"));
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let public_address: SocketAddr = "0.0.0.0:0"
            .parse()
            .unwrap_or_else(|error| panic!("test address: {error}"));
        assert!(serve_health(
            public_address,
            "target/phase3-tests".into(),
            handle.health_registry(),
            handle.clone(),
            shutdown_rx
        )
        .await
        .is_err());
        assert_eq!(
            handle
                .health_registry()
                .snapshot(&config.camera_id)
                .map(|snapshot| snapshot.pipeline_state),
            Some(PipelineState::Running)
        );
        handle
            .shutdown()
            .await
            .unwrap_or_else(|error| panic!("shutdown: {error}"));
        task.await
            .unwrap_or_else(|error| panic!("supervisor task: {error}"));
    }
}
