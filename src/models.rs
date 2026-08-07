use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recording {
    pub id: String,
    pub stream_id: String,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub file_path: PathBuf,
    pub file_size_bytes: u64,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub duration_seconds: f64,
}

impl Recording {
    pub fn new(
        stream_id: &str,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
        file_path: PathBuf,
        file_size_bytes: u64,
        video_codec: Option<String>,
        audio_codec: Option<String>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            stream_id: stream_id.to_string(),
            start_time,
            end_time,
            file_path,
            file_size_bytes,
            video_codec,
            audio_codec,
            duration_seconds: (end_time.timestamp() - start_time.timestamp()) as f64,
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SharedState {
    pub recordings: HashMap<String, Recording>,
    pub is_streaming: bool,
    pub stream_start_time: Option<DateTime<Utc>>,
    pub last_recorded_file: Option<PathBuf>,
}

#[allow(dead_code)]
impl SharedState {
    pub fn new() -> Self {
        Self {
            recordings: HashMap::new(),
            is_streaming: false,
            stream_start_time: None,
            last_recorded_file: None,
        }
    }

    pub fn add_recording(&mut self, recording: Recording) {
        self.recordings.insert(recording.id.clone(), recording);
    }

    pub fn get_recordings_in_range(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Vec<Recording> {
        self.recordings
            .values()
            .filter(|rec| rec.start_time <= end && rec.end_time >= start)
            .cloned()
            .collect()
    }

    pub fn get_recording_by_id(&self, id: &str) -> Option<Recording> {
        self.recordings.get(id).cloned()
    }

    pub fn start_streaming(&mut self) {
        self.is_streaming = true;
        self.stream_start_time = Some(Utc::now());
    }

    pub fn stop_streaming(&mut self) {
        self.is_streaming = false;
        self.stream_start_time = None;
    }

    pub fn update_last_recorded_file(&mut self, file_path: PathBuf) {
        self.last_recorded_file = Some(file_path);
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClipRequest {
    pub stream_id: String,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub output_path: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClipResponse {
    pub clip_id: String,
    pub clip_path: PathBuf,
    pub duration_seconds: f64,
    pub source_recordings: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct StreamStatus {
    pub is_streaming: bool,
    pub stream_start_time: Option<DateTime<Utc>>,
    pub recording_count: usize,
    pub last_recording_time: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForwarderConfig {
    pub target_url: String,
    pub target_auth: String,
    pub source_stream: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ForwarderStatus {
    pub is_forwarding: bool,
    pub target_url: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct SystemStatus {
    pub stream_status: StreamStatus,
    pub forwarder_status: ForwarderStatus,
    pub recording_count: usize,
}
