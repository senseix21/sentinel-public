use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info};

use crate::clip_extractor::ClipExtractor;
use crate::models::{ClipRequest, SharedState, StreamStatus};
use crate::stream_detector::StreamDetector;
use crate::stream_forwarder::StreamForwarder;

#[derive(Clone)]
#[allow(dead_code)]
pub struct ApiState {
    shared_state: Arc<Mutex<SharedState>>,
    recordings_dir: PathBuf,
    clip_extractor: Arc<ClipExtractor>,
    forwarder: Arc<Mutex<StreamForwarder>>,
    stream_detector: Arc<StreamDetector>,
}

pub struct ApiServer {
    shared_state: Arc<Mutex<SharedState>>,
    recordings_dir: PathBuf,
    api_addr: String,
    clip_extractor: Arc<ClipExtractor>,
    forwarder: Arc<Mutex<StreamForwarder>>,
    stream_detector: Arc<StreamDetector>,
}

impl ApiServer {
    pub fn new(
        shared_state: Arc<Mutex<SharedState>>,
        recordings_dir: PathBuf,
        api_addr: String,
        clip_extractor: ClipExtractor,
        forwarder: StreamForwarder,
        stream_detector: StreamDetector,
    ) -> Self {
        Self {
            shared_state,
            recordings_dir,
            api_addr,
            clip_extractor: Arc::new(clip_extractor),
            forwarder: Arc::new(Mutex::new(forwarder)),
            stream_detector: Arc::new(stream_detector),
        }
    }

    pub async fn start(&self) -> anyhow::Result<()> {
        let app = Router::new()
            .route("/status", get(get_status))
            .route("/recordings", get(list_recordings))
            .route("/stream/status", get(get_stream_state))
            .route("/stream/detect", post(detect_stream_encoding))
            .route("/clips", post(extract_clip))
            .route("/clips/{clip_id}", get(get_clip_info))
            // Stream forwarding endpoints
            .route("/forwarding/status", get(get_forwarding_status))
            .route("/forwarding/start", post(start_forwarding))
            .route("/forwarding/stop", post(stop_forwarding))
            .route("/forwarding/config", get(get_forwarding_config))
            .route("/forwarding/config", post(set_forwarding_config))
            .with_state(ApiState {
                shared_state: self.shared_state.clone(),
                recordings_dir: self.recordings_dir.clone(),
                clip_extractor: self.clip_extractor.clone(),
                forwarder: self.forwarder.clone(),
                stream_detector: self.stream_detector.clone(),
            });

        info!("Starting API server on {}", self.api_addr);

        let listener = tokio::net::TcpListener::bind(&self.api_addr)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to bind to {}: {}", self.api_addr, e))?;

        axum::serve(listener, app)
            .await
            .map_err(|e| anyhow::anyhow!("API server error: {}", e))?;

        Ok(())
    }
}

async fn get_status() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "running",
        "service": "cam_rtsp",
        "version": "0.1.0"
    }))
}

async fn get_stream_state(State(state): State<ApiState>) -> impl IntoResponse {
    let shared_state = state.shared_state.lock().await;

    let stream_status = StreamStatus {
        is_streaming: shared_state.is_streaming,
        stream_start_time: shared_state.stream_start_time,
        recording_count: shared_state.recordings.len(),
        last_recording_time: shared_state.recordings.values().map(|r| r.start_time).max(),
    };

    Json(stream_status)
}

async fn list_recordings(
    State(state): State<ApiState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    // Scan for existing recordings and update shared state (async version)
    if let Err(e) = state.clip_extractor.scan_and_update_recordings_async(&state.shared_state).await {
        error!("Failed to scan recordings: {}", e);
    }

    match state.clip_extractor.list_recordings() {
        Ok(recordings) => {
            info!("Found {} recordings", recordings.len());
            Ok(Json(serde_json::json!({
                "recordings": recordings,
                "count": recordings.len()
            })))
        }
        Err(e) => {
            error!("Failed to list recordings: {}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "Failed to list recordings",
                    "message": e.to_string()
                })),
            ))
        }
    }
}

#[derive(Deserialize)]
struct ClipQueryParams {
    stream_id: Option<String>,
    start_time: Option<chrono::DateTime<chrono::Utc>>,
    end_time: Option<chrono::DateTime<chrono::Utc>>,
    output_path: Option<String>,
}

async fn extract_clip(
    State(state): State<ApiState>,
    Query(params): Query<ClipQueryParams>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    // Validate required parameters
    let Some(start_time) = params.start_time else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Missing required parameter: start_time"
            })),
        ));
    };

    let Some(end_time) = params.end_time else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Missing required parameter: end_time"
            })),
        ));
    };

    // Validate time range
    if end_time <= start_time {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "end_time must be after start_time"
            })),
        ));
    }

    let stream_id = params
        .stream_id
        .unwrap_or_else(|| "default_stream".to_string());

    // Create clip request - use direct file lookup instead of metadata
    let request = ClipRequest {
        stream_id,
        start_time,
        end_time,
        output_path: params.output_path,
    };

    // Extract clip using direct file lookup
    match state
        .clip_extractor
        .extract_clip_direct(request)
    {
        Ok(clip_response) => {
            info!("Successfully extracted clip: {}", clip_response.clip_id);
            Ok(Json(serde_json::json!({
                "success": true,
                "clip": clip_response
            })))
        }
        Err(e) => {
            error!("Failed to extract clip: {}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "Failed to extract clip",
                    "message": e.to_string()
                })),
            ))
        }
    }
}

async fn get_clip_info(_state: State<ApiState>, Path(clip_id): Path<String>) -> impl IntoResponse {
    // In a real implementation, you would store clip information
    // For now, just return basic info
    Json(serde_json::json!({
        "clip_id": clip_id,
        "status": "extracted",
        "note": "This is a placeholder. In a real implementation, clip metadata would be stored."
    }))
}

// Stream forwarding API endpoints
async fn get_forwarding_status(
    State(state): State<ApiState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let forwarder = state.forwarder.lock().await;
    let status = forwarder.get_status().await;

    Ok(Json(serde_json::json!({
        "success": true,
        "forwarding": status.is_forwarding(),
        "target_url": status.target_url(),
        "timestamp": status.timestamp()
    })))
}

async fn start_forwarding(
    State(state): State<ApiState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let source_stream = "rtsp://139.162.13.249:8554/mystream".to_string();

    let mut forwarder = state.forwarder.lock().await;
    match forwarder.start_forwarding(source_stream).await {
        Ok(_) => {
            info!("Stream forwarding started successfully");
            Ok(Json(serde_json::json!({
                "success": true,
                "message": "Stream forwarding started",
                "timestamp": chrono::Utc::now()
            })))
        }
        Err(e) => {
            error!("Failed to start stream forwarding: {}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "success": false,
                    "error": "Failed to start stream forwarding",
                    "message": e.to_string()
                })),
            ))
        }
    }
}

async fn stop_forwarding(
    State(state): State<ApiState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let mut forwarder = state.forwarder.lock().await;
    match forwarder.stop_forwarding().await {
        Ok(_) => {
            info!("Stream forwarding stopped successfully");
            Ok(Json(serde_json::json!({
                "success": true,
                "message": "Stream forwarding stopped",
                "timestamp": chrono::Utc::now()
            })))
        }
        Err(e) => {
            error!("Failed to stop stream forwarding: {}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "success": false,
                    "error": "Failed to stop stream forwarding",
                    "message": e.to_string()
                })),
            ))
        }
    }
}

async fn get_forwarding_config(
    State(_state): State<ApiState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    // For now, return empty config. In a real implementation, you'd store this
    Ok(Json(serde_json::json!({
        "success": true,
        "config": {
            "target_url": "",
            "target_auth": "",
            "source_stream": "rtsp://139.162.13.249:8554/mystream",
            "enabled": false
        }
    })))
}

#[derive(Deserialize)]
struct ForwardingConfigBody {
    target_url: String,
    target_auth: Option<String>,
    source_stream: String,
    enabled: bool,
}

async fn set_forwarding_config(
    State(state): State<ApiState>,
    Json(config): Json<ForwardingConfigBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let auth = config.target_auth.clone().unwrap_or_default();

    let mut forwarder = state.forwarder.lock().await;
    forwarder.set_target(config.target_url.clone(), auth);

    info!("Stream forwarding config updated: target={}", config.target_url);

    Ok(Json(serde_json::json!({
        "success": true,
        "message": "Stream forwarding configuration updated",
        "config": {
            "target_url": config.target_url,
            "target_auth": config.target_auth,
            "source_stream": config.source_stream,
            "enabled": config.enabled
        },
        "timestamp": chrono::Utc::now()
    })))
}

// Stream encoding detection API endpoint
async fn detect_stream_encoding(
    State(state): State<ApiState>,
    Json(request): Json<DetectStreamRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    info!("Detecting stream encoding for: {}", request.rtsp_url);

    match state.stream_detector.auto_detect(&request.rtsp_url).await {
        Ok(stream_info) => {
            info!("Detected stream: {} using {}", stream_info.codec_name, stream_info.method_used);
            Ok(Json(serde_json::json!({
                "success": true,
                "stream_info": stream_info
            })))
        }
        Err(e) => {
            error!("Failed to detect stream encoding: {}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "success": false,
                    "error": "Failed to detect stream encoding",
                    "message": e.to_string()
                })),
            ))
        }
    }
}

#[derive(Deserialize)]
struct DetectStreamRequest {
    rtsp_url: String,
}
