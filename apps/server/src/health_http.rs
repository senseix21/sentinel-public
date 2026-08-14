//! Loopback-only bounded health, metrics, and Phase 3 camera API adapter.

use std::{fmt::Write as _, io, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use serde::{Deserialize, Serialize};
use surveillance_camera::connection::ConnectionState;
use surveillance_common::types::CameraId;
use surveillance_config::settings::{
    CameraCredentialsSettings, OneCameraConfig, OneCameraSettings,
};
use surveillance_database::PostgresRecordingRepository;
use surveillance_livekit::token::DevelopmentTokenService;
use surveillance_storage::recordings::{
    PlaybackPlan, RecordingCursor, RecordingResultLimit, RecordingSearch, RecordingTimeRange,
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
use crate::{
    metadata_writer::MetadataWriterHandle, phase4_metrics::Phase4Metrics,
    playback::PlaybackAssembler,
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
#[allow(
    clippy::too_many_arguments,
    reason = "the process owner passes each independently owned adapter boundary explicitly"
)]
pub async fn serve_health(
    address: SocketAddr,
    recording_root: PathBuf,
    registry: CameraHealthRegistry,
    supervisor: CameraSupervisorHandle,
    recordings: Option<PostgresRecordingRepository>,
    playback: Option<PlaybackAssembler>,
    metadata: Option<MetadataWriterHandle>,
    phase4: Option<Phase4Metrics>,
    viewer_tokens: Option<DevelopmentTokenService>,
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
                if let Err(error) = Box::pin(handle_client(stream, &recording_root, &registry, &supervisor, recordings.as_ref(), playback.as_ref(), metadata.as_ref(), phase4.as_ref(), viewer_tokens.as_ref())).await {
                    tracing::debug!(%peer, %error, "API client request discarded");
                }
            }
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "adapter dependencies retain singular owners"
)]
async fn handle_client(
    mut stream: TcpStream,
    recording_root: &std::path::Path,
    registry: &CameraHealthRegistry,
    supervisor: &CameraSupervisorHandle,
    recordings: Option<&PostgresRecordingRepository>,
    playback: Option<&PlaybackAssembler>,
    metadata: Option<&MetadataWriterHandle>,
    phase4: Option<&Phase4Metrics>,
    viewer_tokens: Option<&DevelopmentTokenService>,
) -> io::Result<()> {
    let request = read_request(&mut stream).await?;
    if let (Some(repository), Some(assembler), Some((camera_id, query))) =
        (recordings, playback, playback_target(&request))
    {
        return Box::pin(stream_playback(
            &mut stream,
            camera_id,
            query,
            repository,
            assembler,
            phase4,
        ))
        .await;
    }
    let response = route_with_recordings(
        &request,
        recording_root,
        registry,
        supervisor,
        recordings,
        metadata,
        phase4,
        viewer_tokens,
    )
    .await;
    tokio::time::timeout(IO_TIMEOUT, stream.write_all(response.as_bytes()))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "API response timed out"))??;
    Ok(())
}

fn playback_target(request: &[u8]) -> Option<(CameraId, &str)> {
    let header_end = find_header_end(request)?;
    let head = std::str::from_utf8(&request[..header_end]).ok()?;
    let mut line = head.lines().next()?.split_whitespace();
    if line.next()? != "GET" {
        return None;
    }
    let target = line.next()?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let parts: Vec<_> = path.trim_matches('/').split('/').collect();
    if parts.len() != 5
        || parts[0] != "api"
        || parts[1] != "cameras"
        || parts[3] != "recordings"
        || parts[4] != "playback"
    {
        return None;
    }
    CameraId::new(parts[2])
        .ok()
        .map(|camera_id| (camera_id, query))
}

async fn stream_playback(
    stream: &mut TcpStream,
    camera_id: CameraId,
    query: &str,
    repository: &PostgresRecordingRepository,
    assembler: &PlaybackAssembler,
    metrics: Option<&Phase4Metrics>,
) -> io::Result<()> {
    let parameters: std::collections::BTreeMap<_, _> =
        url::form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect();
    let Some((start, end, range)) = parse_range(&parameters) else {
        if let Some(metrics) = metrics {
            metrics.range_rejected();
        }
        return stream
            .write_all(json_error("400 Bad Request", "invalid_recording_range").as_bytes())
            .await;
    };
    let search = RecordingSearch::new(
        camera_id,
        range,
        RecordingResultLimit::new(1_000).map_err(|_| io::Error::other("playback limit invalid"))?,
        None,
    );
    let Ok(rows) = repository.find_overlapping_completed(&search).await else {
        if let Some(metrics) = metrics {
            metrics.query_failed();
        }
        return stream
            .write_all(
                json_error("503 Service Unavailable", "recording_index_unavailable").as_bytes(),
            )
            .await;
    };
    if let Some(metrics) = metrics {
        metrics.query(rows.len());
    }
    let Ok(plan) = PlaybackPlan::new(range, rows) else {
        return stream
            .write_all(json_error("409 Conflict", "recording_range_not_contiguous").as_bytes())
            .await;
    };
    let Ok(mut playback) = assembler.start(&plan).await else {
        if let Some(metrics) = metrics {
            metrics.playback_failed();
        }
        return stream
            .write_all(json_error("409 Conflict", "recording_media_unavailable").as_bytes())
            .await;
    };
    if let Some(metrics) = metrics {
        metrics.playback_started();
    }
    let headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nTransfer-Encoding: chunked\r\nX-Playback-Accuracy: segment-aligned\r\nX-Requested-Start: {start}\r\nX-Requested-End: {end}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(headers.as_bytes()).await?;
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    loop {
        let read = playback.stdout_mut().read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        let chunk_header = format!("{read:x}\r\n");
        let write = async {
            stream.write_all(chunk_header.as_bytes()).await?;
            stream.write_all(&buffer[..read]).await?;
            stream.write_all(b"\r\n").await
        };
        if let Ok(result) = tokio::time::timeout(IO_TIMEOUT, write).await {
            result?;
        } else {
            playback.cancel().await;
            if let Some(metrics) = metrics {
                metrics.playback_cancelled();
                metrics.playback_closed();
            }
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "playback client stalled",
            ));
        }
    }
    if playback.finish().await.is_err() {
        if let Some(metrics) = metrics {
            metrics.playback_failed();
            metrics.playback_closed();
        }
        return Err(io::Error::other("playback assembly failed"));
    }
    if let Some(metrics) = metrics {
        metrics.playback_completed();
        metrics.playback_closed();
    }
    stream.write_all(b"0\r\n\r\n").await
}

fn parse_range(
    parameters: &std::collections::BTreeMap<String, String>,
) -> Option<(jiff::Timestamp, jiff::Timestamp, RecordingTimeRange)> {
    let start = parameters.get("start")?.parse::<jiff::Timestamp>().ok()?;
    let end = parameters.get("end")?.parse::<jiff::Timestamp>().ok()?;
    let range = RecordingTimeRange::new(
        std::time::SystemTime::from(start),
        std::time::SystemTime::from(end),
        Duration::from_secs(24 * 60 * 60),
    )
    .ok()?;
    Some((start, end, range))
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

#[cfg(test)]
async fn route(
    request: &[u8],
    recording_root: &std::path::Path,
    registry: &CameraHealthRegistry,
    supervisor: &CameraSupervisorHandle,
) -> String {
    route_with_recordings(
        request,
        recording_root,
        registry,
        supervisor,
        None,
        None,
        None,
        None,
    )
    .await
}

#[allow(
    clippy::too_many_arguments,
    reason = "API adapter receives explicit independently owned boundaries"
)]
async fn route_with_recordings(
    request: &[u8],
    recording_root: &std::path::Path,
    registry: &CameraHealthRegistry,
    supervisor: &CameraSupervisorHandle,
    recordings: Option<&PostgresRecordingRepository>,
    metadata: Option<&MetadataWriterHandle>,
    phase4: Option<&Phase4Metrics>,
    viewer_tokens: Option<&DevelopmentTokenService>,
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
    let (Some(method), Some(target), Some(_version)) = (parts.next(), parts.next(), parts.next())
    else {
        return json_error("400 Bad Request", "invalid_request");
    };
    let body = &request[header_end + 4..];
    let (path, query) = target
        .split_once('?')
        .map_or((target, ""), |(path, query)| (path, query));
    match (method, path) {
        ("GET", "/live") => response("200 OK", "text/plain; charset=utf-8", "live\n"),
        ("GET", "/ready") => {
            readiness_response(&registry.snapshots(), recordings, metadata, phase4).await
        }
        ("GET", "/metrics") => response(
            "200 OK",
            "text/plain; version=0.0.4; charset=utf-8",
            &render_metrics(
                &registry.snapshots(),
                recordings,
                metadata,
                phase4,
                Some(&supervisor.event_bus()),
            ),
        ),
        ("GET", "/api/cameras") => json_response("200 OK", &camera_list(&registry.snapshots())),
        ("POST", "/api/cameras") => add_camera(body, recording_root, supervisor).await,
        _ => {
            route_camera(
                method,
                path,
                query,
                recording_root,
                registry,
                supervisor,
                recordings,
                viewer_tokens,
            )
            .await
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "camera routing keeps resource dependencies explicit"
)]
async fn route_camera(
    method: &str,
    path: &str,
    query: &str,
    recording_root: &std::path::Path,
    registry: &CameraHealthRegistry,
    supervisor: &CameraSupervisorHandle,
    recordings: Option<&PostgresRecordingRepository>,
    viewer_tokens: Option<&DevelopmentTokenService>,
) -> String {
    let segments: Vec<_> = path.trim_matches('/').split('/').collect();
    if segments.len() < 3 || segments[0] != "api" || segments[1] != "cameras" {
        return json_error("404 Not Found", "not_found");
    }
    let Ok(camera_id) = CameraId::new(segments[2]) else {
        return json_error("400 Bad Request", "invalid_camera_id");
    };
    if method == "GET" && segments.as_slice() == ["api", "cameras", segments[2], "recordings"] {
        return recording_search(camera_id, query, recording_root, recordings).await;
    }
    if method == "GET" && segments.len() == 5 && segments[3] == "recordings" {
        let Ok(recording_id) = surveillance_common::types::RecordingId::new(segments[4]) else {
            return json_error("400 Bad Request", "invalid_recording_id");
        };
        return recording_detail(camera_id, recording_id, recording_root, recordings).await;
    }
    if method == "POST" && segments.as_slice() == ["api", "cameras", segments[2], "viewer-token"] {
        if registry.snapshot(&camera_id).is_none() {
            return json_error("404 Not Found", "camera_not_found");
        }
        let Some(service) = viewer_tokens else {
            return json_error(
                "503 Service Unavailable",
                "livekit_token_issuer_unavailable",
            );
        };
        return match service.issue_for_camera(&camera_id) {
            Ok(access) => json_response(
                "200 OK",
                &ViewerAccessResponse {
                    endpoint: access.endpoint(),
                    room: access.room(),
                    token: access.token(),
                    expires_in_seconds: access.expires_in_seconds(),
                    development_only: true,
                },
            ),
            Err(_) => json_error("503 Service Unavailable", "livekit_token_issue_failed"),
        };
    }
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

#[derive(Serialize)]
struct ViewerAccessResponse<'a> {
    endpoint: &'a str,
    room: &'a str,
    token: &'a str,
    expires_in_seconds: u64,
    development_only: bool,
}

#[derive(Serialize)]
struct RecordingSearchResponse {
    recordings: Vec<RecordingResponse>,
    next_cursor: Option<String>,
}

#[derive(Serialize)]
struct RecordingResponse {
    recording_id: String,
    camera_id: String,
    started_at: String,
    ended_at: String,
    codec: &'static str,
    width: u32,
    height: u32,
    size_bytes: u64,
    status: &'static str,
    playback: String,
}

async fn recording_search(
    camera_id: CameraId,
    query: &str,
    recording_root: &std::path::Path,
    repository: Option<&PostgresRecordingRepository>,
) -> String {
    let Some(repository) = repository else {
        return json_error("503 Service Unavailable", "recording_index_unavailable");
    };
    let parameters: std::collections::BTreeMap<_, _> =
        url::form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect();
    let (Some(start), Some(end)) = (parameters.get("start"), parameters.get("end")) else {
        return json_error("400 Bad Request", "missing_recording_range");
    };
    let Ok(start) = start.parse::<jiff::Timestamp>() else {
        return json_error("400 Bad Request", "invalid_recording_start");
    };
    let Ok(end) = end.parse::<jiff::Timestamp>() else {
        return json_error("400 Bad Request", "invalid_recording_end");
    };
    let (start, end) = (
        std::time::SystemTime::from(start),
        std::time::SystemTime::from(end),
    );
    let Ok(range) = RecordingTimeRange::new(start, end, Duration::from_secs(24 * 60 * 60)) else {
        return json_error("400 Bad Request", "invalid_recording_range");
    };
    let limit = match parameters.get("limit") {
        Some(value) => value.parse::<usize>().ok(),
        None => Some(100),
    };
    let Some(limit) = limit.and_then(|value| RecordingResultLimit::new(value).ok()) else {
        return json_error("400 Bad Request", "invalid_recording_limit");
    };
    let cursor = match parameters.get("cursor") {
        Some(value) => match decode_recording_cursor(value) {
            Ok(cursor) => Some(cursor),
            Err(()) => return json_error("400 Bad Request", "invalid_recording_cursor"),
        },
        None => None,
    };
    let search = RecordingSearch::new(camera_id, range, limit, cursor);
    match repository.find_overlapping_completed(&search).await {
        Ok(rows) => {
            if !recordings_are_present(repository, recording_root, &rows).await {
                return json_error("409 Conflict", "recording_storage_inconsistent");
            }
            let next_cursor = (rows.len() == limit.get())
                .then(|| {
                    rows.last()
                        .map(|row| encode_recording_cursor(row.started_at(), row.recording_id()))
                })
                .flatten();
            let recordings = rows.iter().map(recording_response).collect();
            json_response(
                "200 OK",
                &RecordingSearchResponse {
                    recordings,
                    next_cursor,
                },
            )
        }
        Err(error) if error.retryable() => {
            json_error("503 Service Unavailable", "recording_index_unavailable")
        }
        Err(_) => json_error("500 Internal Server Error", "recording_query_failed"),
    }
}

async fn recording_detail(
    camera_id: CameraId,
    recording_id: surveillance_common::types::RecordingId,
    recording_root: &std::path::Path,
    repository: Option<&PostgresRecordingRepository>,
) -> String {
    let Some(repository) = repository else {
        return json_error("503 Service Unavailable", "recording_index_unavailable");
    };
    match repository.find_completed(&camera_id, &recording_id).await {
        Ok(Some(row))
            if recordings_are_present(repository, recording_root, std::slice::from_ref(&row))
                .await =>
        {
            json_response("200 OK", &recording_response(&row))
        }
        Ok(Some(_)) => json_error("409 Conflict", "recording_storage_inconsistent"),
        Ok(None) => json_error("404 Not Found", "recording_not_found"),
        Err(error) if error.retryable() => {
            json_error("503 Service Unavailable", "recording_index_unavailable")
        }
        Err(_) => json_error("500 Internal Server Error", "recording_query_failed"),
    }
}

async fn recordings_are_present(
    repository: &PostgresRecordingRepository,
    root: &std::path::Path,
    rows: &[surveillance_storage::metadata::CompletedRecording],
) -> bool {
    let mut all_present = true;
    for row in rows {
        let path = root.join(row.locator().as_path());
        let present = tokio::fs::symlink_metadata(path)
            .await
            .is_ok_and(|metadata| {
                metadata.file_type().is_file() && !metadata.file_type().is_symlink()
            });
        if !present {
            let _ = repository.mark_inconsistent(row.recording_id()).await;
            all_present = false;
        }
    }
    all_present
}

fn recording_response(
    row: &surveillance_storage::metadata::CompletedRecording,
) -> RecordingResponse {
    RecordingResponse {
        recording_id: row.recording_id().as_str().to_owned(),
        camera_id: row.camera_id().as_str().to_owned(),
        started_at: jiff::Timestamp::try_from(row.started_at())
            .map_or_else(|_| "invalid".to_owned(), |value| value.to_string()),
        ended_at: jiff::Timestamp::try_from(row.ended_at())
            .map_or_else(|_| "invalid".to_owned(), |value| value.to_string()),
        codec: "h264",
        width: row.width(),
        height: row.height(),
        size_bytes: row.size_bytes(),
        status: "completed",
        playback: format!(
            "/api/cameras/{}/recordings/playback?start={}&end={}",
            row.camera_id(),
            jiff::Timestamp::try_from(row.started_at())
                .map_or_else(|_| "invalid".to_owned(), |value| value.to_string()),
            jiff::Timestamp::try_from(row.ended_at())
                .map_or_else(|_| "invalid".to_owned(), |value| value.to_string()),
        ),
    }
}

fn encode_recording_cursor(
    started_at: std::time::SystemTime,
    recording_id: &surveillance_common::types::RecordingId,
) -> String {
    let timestamp = jiff::Timestamp::try_from(started_at).map_or(0, jiff::Timestamp::as_nanosecond);
    format!("{timestamp:x}.{}", recording_id.as_str())
}

fn decode_recording_cursor(value: &str) -> Result<RecordingCursor, ()> {
    let (timestamp, recording_id) = value.split_once('.').ok_or(())?;
    let timestamp = i128::from_str_radix(timestamp, 16).map_err(|_| ())?;
    let timestamp = jiff::Timestamp::from_nanosecond(timestamp).map_err(|_| ())?;
    let started_at = std::time::SystemTime::from(timestamp);
    let recording_id =
        surveillance_common::types::RecordingId::new(recording_id).map_err(|_| ())?;
    Ok(RecordingCursor::new(started_at, recording_id))
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
    #[serde(default = "default_detection_enabled")]
    detection_enabled: bool,
}

const fn default_detection_enabled() -> bool {
    true
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
    let mut settings = OneCameraSettings::h264_mvp(
        request.camera_id,
        request.rtsp_endpoint,
        credentials,
        request.width,
        request.height,
        recording_root.to_path_buf(),
    );
    settings.detection_enabled = request.detection_enabled;
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
    detection_enabled: bool,
    detection_state: Option<String>,
    detection_queue_depth: Option<usize>,
    detection_queue_capacity: Option<usize>,
    detection_dropped: u64,
    detection_inferred: u64,
    detections_published: u64,
    recorder_completed_segments: u64,
    recorder_failed_segments: u64,
    live_publication_enabled: bool,
    live_publication_state: Option<String>,
    live_publication_failure: Option<String>,
    live_publication_timestamp_authority: Option<&'static str>,
    live_published_access_units: u64,
    live_published_bytes: u64,
}

impl From<&CameraStatusSnapshot> for CameraResponse {
    #[allow(
        clippy::too_many_lines,
        reason = "flat credential-free API projection keeps bounded health fields explicit"
    )]
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
            detection_enabled: snapshot.health.as_ref().is_some_and(|value| {
                !matches!(
                    value.detection.state,
                    surveillance_detector::detector::DetectionState::Disabled
                )
            }),
            detection_state: snapshot
                .health
                .as_ref()
                .map(|value| format!("{:?}", value.detection.state)),
            detection_queue_depth: snapshot
                .health
                .as_ref()
                .map(|value| value.detection_queue.depth),
            detection_queue_capacity: snapshot
                .health
                .as_ref()
                .map(|value| value.detection_queue.capacity),
            detection_dropped: snapshot
                .health
                .as_ref()
                .map_or(0, |value| value.detection_queue.dropped_oldest),
            detection_inferred: snapshot
                .health
                .as_ref()
                .map_or(0, |value| value.detection.counters.inferred),
            detections_published: snapshot
                .health
                .as_ref()
                .map_or(0, |value| value.detection.counters.events_published),
            recorder_completed_segments: snapshot
                .health
                .as_ref()
                .map_or(0, |value| value.recorder.completed_segments),
            recorder_failed_segments: snapshot
                .health
                .as_ref()
                .map_or(0, |value| value.recorder.failed_segments),
            live_publication_enabled: snapshot
                .health
                .as_ref()
                .is_some_and(|value| value.publication.enabled),
            live_publication_state: snapshot
                .health
                .as_ref()
                .map(|value| format!("{:?}", value.publication.state)),
            live_publication_failure: snapshot
                .health
                .as_ref()
                .and_then(|value| value.publication.last_failure)
                .map(|value| format!("{value:?}")),
            live_publication_timestamp_authority: snapshot.health.as_ref().map(|value| {
                timestamp_authority_name(value.publication.counters.timestamp_authority)
            }),
            live_published_access_units: snapshot
                .health
                .as_ref()
                .map_or(0, |value| value.publication.counters.access_units),
            live_published_bytes: snapshot
                .health
                .as_ref()
                .map_or(0, |value| value.publication.counters.bytes),
        }
    }
}

fn camera_list(snapshots: &[CameraStatusSnapshot]) -> Vec<CameraResponse> {
    snapshots.iter().map(CameraResponse::from).collect()
}

async fn readiness_response(
    snapshots: &[CameraStatusSnapshot],
    recordings: Option<&PostgresRecordingRepository>,
    metadata: Option<&MetadataWriterHandle>,
    phase4: Option<&Phase4Metrics>,
) -> String {
    let ready = !snapshots.is_empty()
        && snapshots.iter().all(|snapshot| {
            snapshot.pipeline_state == PipelineState::Running
                && snapshot.health.as_ref().is_some_and(|health| {
                    matches!(health.connection.state(), ConnectionState::Connected { .. })
                })
        });
    let database_ready = match recordings {
        Some(repository) => repository.health().await.is_ok(),
        None => true,
    };
    let metadata_ready = metadata.is_none_or(|handle| {
        let snapshot = handle.snapshot();
        snapshot.rejected_full == 0 && snapshot.permanent_failures == 0
    });
    let storage_ready = phase4.is_none_or(|metrics| {
        metrics.snapshot().storage_pressure
            < surveillance_storage::retention::StoragePressure::Critical
    });
    if ready && database_ready && metadata_ready && storage_ready {
        response("200 OK", "text/plain; charset=utf-8", "ready\n")
    } else {
        response(
            "503 Service Unavailable",
            "text/plain; charset=utf-8",
            if !database_ready {
                "recording index is degraded; media recording may continue\n"
            } else if !metadata_ready {
                "recording metadata delivery is degraded; filesystem reconciliation remains available\n"
            } else if !storage_ready {
                "recording storage pressure is critical\n"
            } else {
                "one or more cameras are degraded\n"
            },
        )
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "the flat exporter keeps bounded metric names and labels directly auditable"
)]
fn render_metrics(
    snapshots: &[CameraStatusSnapshot],
    recordings: Option<&PostgresRecordingRepository>,
    metadata: Option<&MetadataWriterHandle>,
    phase4: Option<&Phase4Metrics>,
    event_bus: Option<&surveillance_events::bus::EventBus>,
) -> String {
    let mut output = String::with_capacity(snapshots.len().saturating_mul(1_024));
    if let Some(event_bus) = event_bus {
        let events = event_bus.metrics();
        let _ = writeln!(
            output,
            "surveillance_event_bus_subscribers {}",
            events.subscribers
        );
        let _ = writeln!(
            output,
            "surveillance_event_publications_total {}",
            events.publications
        );
        let _ = writeln!(
            output,
            "surveillance_event_deliveries_total {}",
            events.outcomes.accepted
        );
        let _ = writeln!(
            output,
            "surveillance_event_rejected_full_total {}",
            events.outcomes.rejected_full
        );
        let _ = writeln!(
            output,
            "surveillance_event_dropped_newest_total {}",
            events.outcomes.dropped_newest
        );
        let _ = writeln!(
            output,
            "surveillance_event_replaced_oldest_total {}",
            events.outcomes.replaced_oldest
        );
        let _ = writeln!(
            output,
            "surveillance_event_coalesced_total {}",
            events.outcomes.coalesced
        );
        for (kind, value) in [
            "camera_state_changed",
            "recording_segment_completed",
            "recording_segment_failed",
            "storage_pressure_changed",
            "person_detection_observed",
            "detector_state_changed",
        ]
        .into_iter()
        .zip(events.event_kinds)
        {
            let _ = writeln!(
                output,
                "surveillance_event_publications_by_kind_total{{kind=\"{kind}\"}} {value}"
            );
        }
        for (class, value) in [
            "critical_lifecycle",
            "latest_state",
            "telemetry",
            "detection",
        ]
        .into_iter()
        .zip(events.delivery_classes)
        {
            let _ = writeln!(
                output,
                "surveillance_event_publications_by_class_total{{class=\"{class}\"}} {value}"
            );
        }
    }
    if let Some(repository) = recordings {
        let pool = repository.pool_snapshot();
        let _ = writeln!(output, "surveillance_database_pool_size {}", pool.size);
        let _ = writeln!(output, "surveillance_database_pool_idle {}", pool.idle);
        let _ = writeln!(
            output,
            "surveillance_database_pool_maximum {}",
            pool.maximum
        );
    }
    if let Some(metadata) = metadata {
        let writer = metadata.snapshot();
        let _ = writeln!(
            output,
            "surveillance_metadata_queue_capacity {}",
            writer.queue_capacity
        );
        let _ = writeln!(
            output,
            "surveillance_metadata_accepted_total {}",
            writer.accepted
        );
        let _ = writeln!(
            output,
            "surveillance_metadata_rejected_full_total {}",
            writer.rejected_full
        );
        let _ = writeln!(
            output,
            "surveillance_metadata_persisted_total {}",
            writer.persisted
        );
        let _ = writeln!(
            output,
            "surveillance_metadata_retryable_failures_total {}",
            writer.retryable_failures
        );
    }
    if let Some(metrics) = phase4 {
        let value = metrics.snapshot();
        for (name, metric_value) in [
            (
                "surveillance_reconciliation_passes_total",
                value.reconciliation_passes,
            ),
            (
                "surveillance_reconciliation_failures_total",
                value.reconciliation_failures,
            ),
            (
                "surveillance_reconciliation_examined_total",
                value.reconciliation_examined,
            ),
            (
                "surveillance_reconciliation_imported_total",
                value.reconciliation_imported,
            ),
            (
                "surveillance_reconciliation_incomplete_total",
                value.reconciliation_incomplete,
            ),
            (
                "surveillance_reconciliation_inconsistent_total",
                value.reconciliation_inconsistent,
            ),
            (
                "surveillance_recording_queries_total",
                value.recording_queries,
            ),
            (
                "surveillance_recording_query_failures_total",
                value.recording_query_failures,
            ),
            (
                "surveillance_recording_query_results_total",
                value.recording_query_results,
            ),
            (
                "surveillance_recording_ranges_rejected_total",
                value.recording_ranges_rejected,
            ),
            ("surveillance_playback_active", value.playback_active),
            (
                "surveillance_playback_started_total",
                value.playback_started,
            ),
            (
                "surveillance_playback_completed_total",
                value.playback_completed,
            ),
            (
                "surveillance_playback_cancelled_total",
                value.playback_cancelled,
            ),
            (
                "surveillance_playback_failures_total",
                value.playback_failures,
            ),
            (
                "surveillance_playback_buffer_bytes",
                value.playback_buffer_bytes,
            ),
            (
                "surveillance_retention_passes_total",
                value.retention_passes,
            ),
            (
                "surveillance_retention_failures_total",
                value.retention_failures,
            ),
            (
                "surveillance_retention_candidates_total",
                value.retention_candidates,
            ),
            (
                "surveillance_retention_deleted_total",
                value.retention_deleted,
            ),
            (
                "surveillance_retention_delete_failures_total",
                value.retention_delete_failures,
            ),
            (
                "surveillance_retention_bytes_reclaimed_total",
                value.retention_bytes_reclaimed,
            ),
            (
                "surveillance_storage_total_bytes",
                value.storage_total_bytes,
            ),
            ("surveillance_storage_free_bytes", value.storage_free_bytes),
            (
                "surveillance_storage_pressure",
                value.storage_pressure as u64,
            ),
        ] {
            let _ = writeln!(output, "{name} {metric_value}");
        }
    }
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
        queue_metrics(&mut output, camera, "detection", snapshot.detection_queue);
        let detection = snapshot.detection;
        metric(
            &mut output,
            "surveillance_detector_enabled",
            camera,
            u8::from(!matches!(
                detection.state,
                surveillance_detector::detector::DetectionState::Disabled
            )),
        );
        metric(
            &mut output,
            "surveillance_detector_state",
            camera,
            match detection.state {
                surveillance_detector::detector::DetectionState::Disabled => 0_u8,
                surveillance_detector::detector::DetectionState::Starting => 1,
                surveillance_detector::detector::DetectionState::Running => 2,
                surveillance_detector::detector::DetectionState::Degraded => 3,
                surveillance_detector::detector::DetectionState::Terminal => 4,
                surveillance_detector::detector::DetectionState::Stopping => 5,
                surveillance_detector::detector::DetectionState::Stopped => 6,
            },
        );
        for (name, value) in [
            (
                "surveillance_detector_encoded_considered_total",
                detection.sampler.considered,
            ),
            (
                "surveillance_detector_sampled_total",
                detection.sampler.sampled,
            ),
            (
                "surveillance_detector_skipped_by_rate_total",
                detection.sampler.skipped_by_rate,
            ),
            (
                "surveillance_detector_duplicate_timestamps_total",
                detection.sampler.duplicate,
            ),
            (
                "surveillance_detector_resets_total",
                detection.sampler.resets,
            ),
            (
                "surveillance_detector_decode_attempts_total",
                detection.counters.decode_attempts,
            ),
            (
                "surveillance_detector_decoded_total",
                detection.counters.decoded,
            ),
            (
                "surveillance_detector_decode_failures_total",
                detection.counters.decode_failed,
            ),
            (
                "surveillance_detector_inference_attempts_total",
                detection.counters.inference_attempts,
            ),
            (
                "surveillance_detector_inferred_total",
                detection.counters.inferred,
            ),
            (
                "surveillance_detector_detections_produced_total",
                detection.counters.detections_produced,
            ),
            (
                "surveillance_detector_inference_failures_total",
                detection.counters.inference_failed,
            ),
            (
                "surveillance_detector_events_published_total",
                detection.counters.events_published,
            ),
            (
                "surveillance_detector_event_delivery_losses_total",
                detection.counters.event_delivery_losses,
            ),
            (
                "surveillance_detector_decode_duration_micros_total",
                detection.counters.decode_duration_micros,
            ),
            (
                "surveillance_detector_inference_duration_micros_total",
                detection.counters.inference_duration_micros,
            ),
            (
                "surveillance_detector_end_to_event_duration_micros_total",
                detection.counters.end_to_event_duration_micros,
            ),
            (
                "surveillance_detector_end_to_event_latency_bucket_0_10ms_total",
                detection.counters.latency_le_10ms,
            ),
            (
                "surveillance_detector_end_to_event_latency_bucket_10_50ms_total",
                detection.counters.latency_le_50ms,
            ),
            (
                "surveillance_detector_end_to_event_latency_bucket_50_100ms_total",
                detection.counters.latency_le_100ms,
            ),
            (
                "surveillance_detector_end_to_event_latency_bucket_100_250ms_total",
                detection.counters.latency_le_250ms,
            ),
            (
                "surveillance_detector_end_to_event_latency_bucket_250_500ms_total",
                detection.counters.latency_le_500ms,
            ),
            (
                "surveillance_detector_end_to_event_latency_bucket_500_1000ms_total",
                detection.counters.latency_le_1000ms,
            ),
            (
                "surveillance_detector_end_to_event_latency_bucket_over_1000ms_total",
                detection.counters.latency_over_1000ms,
            ),
        ] {
            metric(&mut output, name, camera, value);
        }
        let publication = snapshot.publication;
        metric(
            &mut output,
            "surveillance_livekit_enabled",
            camera,
            u8::from(publication.enabled),
        );
        metric(
            &mut output,
            "surveillance_livekit_state",
            camera,
            publication_state_value(publication.state),
        );
        metric(
            &mut output,
            "surveillance_livekit_media_timestamp_authority",
            camera,
            u8::from(matches!(
                publication.counters.timestamp_authority,
                surveillance_livekit::adapter::TimestampAuthority::Media
            )),
        );
        for (name, value) in [
            (
                "surveillance_livekit_connection_attempts_total",
                publication.counters.connection_attempts,
            ),
            (
                "surveillance_livekit_connection_successes_total",
                publication.counters.connection_successes,
            ),
            (
                "surveillance_livekit_reconnects_total",
                publication.counters.reconnects,
            ),
            (
                "surveillance_livekit_access_units_total",
                publication.counters.access_units,
            ),
            (
                "surveillance_livekit_bytes_total",
                publication.counters.bytes,
            ),
            (
                "surveillance_livekit_keyframes_total",
                publication.counters.keyframes,
            ),
            (
                "surveillance_livekit_parameter_set_waits_total",
                publication.counters.parameter_set_waits,
            ),
            (
                "surveillance_livekit_stale_drops_total",
                publication.counters.stale_drops,
            ),
            (
                "surveillance_livekit_media_rejections_total",
                publication.counters.media_rejections,
            ),
            (
                "surveillance_livekit_send_failures_total",
                publication.counters.send_failures,
            ),
            (
                "surveillance_livekit_shutdown_failures_total",
                publication.counters.shutdown_failures,
            ),
        ] {
            metric(&mut output, name, camera, value);
        }
    }
    output
}

const fn timestamp_authority_name(
    authority: surveillance_livekit::adapter::TimestampAuthority,
) -> &'static str {
    use surveillance_livekit::adapter::TimestampAuthority;
    match authority {
        TimestampAuthority::Media => "media",
        TimestampAuthority::AdapterClock => "adapter_clock",
    }
}

const fn publication_state_value(state: surveillance_livekit::domain::PublicationState) -> u8 {
    use surveillance_livekit::domain::PublicationState as S;
    match state {
        S::Disabled => 0,
        S::Starting => 1,
        S::Connecting => 2,
        S::Publishing => 3,
        S::Backoff => 4,
        S::Degraded => 5,
        S::Terminal => 6,
        S::Stopping => 7,
        S::Stopped => 8,
    }
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
    use super::{
        decode_recording_cursor, encode_recording_cursor, parse_range, route, serve_health,
    };
    use crate::camera_supervisor::{CameraSupervisor, PipelineState};
    use std::{net::SocketAddr, num::NonZeroUsize, sync::Arc, time::Duration};
    use surveillance_config::settings::{OneCameraConfig, OneCameraSettings};
    use tokio::sync::watch;

    #[test]
    fn recording_range_normalizes_offsets_and_cursor_round_trips() {
        let parameters = std::collections::BTreeMap::from([
            ("start".to_owned(), "2026-08-11T12:00:00+06:00".to_owned()),
            ("end".to_owned(), "2026-08-11T06:01:00Z".to_owned()),
        ]);
        let (_, _, range) =
            parse_range(&parameters).unwrap_or_else(|| panic!("offset range must parse"));
        assert_eq!(
            range
                .end()
                .duration_since(range.start())
                .unwrap_or(Duration::ZERO),
            Duration::from_secs(60)
        );
        let recording_id = surveillance_common::types::RecordingId::new("cursor-recording")
            .unwrap_or_else(|error| panic!("recording ID: {error}"));
        let encoded = encode_recording_cursor(range.start(), &recording_id);
        let decoded =
            decode_recording_cursor(&encoded).unwrap_or_else(|()| panic!("cursor must decode"));
        assert_eq!(decoded.started_at(), range.start());
        assert_eq!(decoded.recording_id(), &recording_id);
    }

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
            None,
            None,
            None,
            None,
            None,
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
