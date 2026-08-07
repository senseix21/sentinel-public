use std::path::{Path, PathBuf};
use std::process::Command;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use tracing::{info, error};

use crate::models::{Recording, ClipRequest, ClipResponse};

#[derive(Clone)]
pub struct ClipExtractor {
    recordings_dir: PathBuf,
    ffmpeg_path: Option<PathBuf>,
}

impl ClipExtractor {
    pub fn new(recordings_dir: PathBuf) -> Self {
        // Try to find ffmpeg in the system PATH
        let ffmpeg_path = which::which("ffmpeg").ok();

        Self {
            recordings_dir,
            ffmpeg_path,
        }
    }

    pub fn extract_clip(
        &self,
        request: ClipRequest,
        source_recordings: Vec<Recording>,
    ) -> Result<ClipResponse> {
        if source_recordings.is_empty() {
            return Err(anyhow::anyhow!("No source recordings found for the specified time range"));
        }

        let output_path = if let Some(ref path) = request.output_path {
            PathBuf::from(path)
        } else {
            self.recordings_dir.join(format!(
                "clip_{}.mkv",
                Utc::now().format("%Y%m%d_%H%M%S")
            ))
        };

        info!(
            "Extracting clip from {:?} to {:?} to output path: {}",
            request.start_time,
            request.end_time,
            output_path.display()
        );

        // Note: extract_clip method is deprecated, use extract_clip_direct instead
        // This method kept for backward compatibility but needs to be implemented
        return Err(anyhow::anyhow!("extract_clip method deprecated, use extract_clip_direct instead"));
    }

    pub fn extract_clip_direct(&self, request: ClipRequest) -> Result<ClipResponse> {
        let output_path = if let Some(ref path) = request.output_path {
            PathBuf::from(path)
        } else {
            self.recordings_dir.join(format!(
                "clip_{}.mkv",
                Utc::now().format("%Y%m%d_%H%M%S")
            ))
        };

        info!(
            "Extracting clip from {:?} to {:?} to output path: {}",
            request.start_time,
            request.end_time,
            output_path.display()
        );

        // Use ffmpeg to extract the clip with direct file lookup
        self.ffmpeg_extract_clip_direct(&request, &output_path)?;

        // Calculate duration
        let duration_seconds = (request.end_time - request.start_time).num_seconds() as f64;

        // Create clip response
        let clip_response = ClipResponse {
            clip_id: uuid::Uuid::new_v4().to_string(),
            clip_path: output_path,
            duration_seconds,
            source_recordings: vec!["direct_file_lookup".to_string()], // Placeholder
            created_at: Utc::now(),
        };

        Ok(clip_response)
    }

    fn ffmpeg_extract_clip_direct(
        &self,
        request: &ClipRequest,
        output_path: &Path,
    ) -> Result<()> {
  // Find the most recent recording file that contains the start time
        // Look for files within 1 hour before the requested start time
        let file_start_time = self.find_closest_recording_file(&request.start_time)?;

        let actual_file_path = self.recordings_dir.join(format!("stream_{}.mkv",
            file_start_time.format("%Y%m%d_%H%M%S")));

        // Use ffprobe to get the actual file duration
        let clip_duration = self.get_file_duration(&actual_file_path)?;

        // Calculate offsets
        let start_offset = (request.start_time - file_start_time).num_seconds() as f64;
        let mut end_offset = (request.end_time - file_start_time).num_seconds() as f64;

        // Clamp end offset to file duration
        if end_offset > clip_duration {
            end_offset = clip_duration;
        }

        let extract_duration = end_offset - start_offset;

        if extract_duration <= 0.0 {
            return Err(anyhow::anyhow!("Invalid duration for extraction"));
        }

        // DEBUG: Show all timing information
        info!("🎬 TIMING DEBUG:");
        info!("  Requested: {} to {} ({}s)", request.start_time, request.end_time, (request.end_time - request.start_time).num_seconds());
        info!("  File found: {}", actual_file_path.display());
        info!("  File start time: {}", file_start_time);
        info!("  File duration: {:.3}s", clip_duration);
        info!("  Start offset: {:.3}s", start_offset);
        info!("  End offset: {:.3}s", end_offset);
        info!("  Final extract duration: {:.3}s", extract_duration);

        // Verify the extraction makes sense
        let requested_duration = (request.end_time - request.start_time).num_seconds() as f64;
        if (extract_duration - requested_duration).abs() > 1.0 {
            info!("  ⚠️  WARNING: Extracting {:.3}s instead of requested {:.3}s", extract_duration, requested_duration);
        }

        // Build ffmpeg command for lossless extraction
        let mut cmd = if let Some(ref ffmpeg) = self.ffmpeg_path {
            Command::new(ffmpeg)
        } else {
            Command::new("ffmpeg")
        };

        cmd.arg("-ss")
           .arg(start_offset.to_string())
           .arg("-i")
           .arg(&actual_file_path)
           .arg("-t")
           .arg(extract_duration.to_string())
           .arg("-c:v")
           .arg("copy")
           .arg("-c:a")
           .arg("copy")
           .arg("-y")
           .arg(output_path);

        info!("Executing ffmpeg command: {:?}", cmd);

        // Execute the command
        let output = cmd.output().context("Failed to execute ffmpeg")?;

        if !output.status.success() {
            error!("FFmpeg failed with output: {}", String::from_utf8_lossy(&output.stderr));
            return Err(anyhow::anyhow!("FFmpeg failed: {}", String::from_utf8_lossy(&output.stderr)));
        }

        info!("Successfully extracted clip to: {}", output_path.display());
        Ok(())
    }

    pub fn list_recordings(&self) -> Result<Vec<Recording>> {
        let mut recordings = Vec::new();

        if let Ok(entries) = std::fs::read_dir(&self.recordings_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("mkv") {
                    if let Some(recording) = self.parse_recording_from_path(&path) {
                        recordings.push(recording);
                    }
                }
            }
        }

        recordings.sort_by(|a, b| a.start_time.cmp(&b.start_time));
        Ok(recordings)
    }

    pub fn scan_and_update_recordings(&self, shared_state: &std::sync::Arc<tokio::sync::Mutex<crate::models::SharedState>>) -> Result<()> {
        let recordings = self.list_recordings()?;
        let mut state = shared_state.blocking_lock();

        for recording in recordings {
            if !state.recordings.contains_key(&recording.id) {
                state.add_recording(recording);
            }
        }

        Ok(())
    }

  // Alternative async method that doesn't use blocking_lock
  pub async fn scan_and_update_recordings_async(&self, shared_state: &std::sync::Arc<tokio::sync::Mutex<crate::models::SharedState>>) -> Result<()> {
        let recordings = self.list_recordings().unwrap_or_default();
        let mut state = shared_state.lock().await;

        for recording in recordings {
            if !state.recordings.contains_key(&recording.id) {
                state.add_recording(recording);
            }
        }

        drop(state); // Explicitly drop to unlock
        Ok(())
    }

    fn parse_recording_from_path(&self, path: &Path) -> Option<Recording> {
        let filename = path.file_name()?.to_str()?;
        let parts: Vec<&str> = filename.split('_').collect();

        if parts.len() >= 2 && parts[0] == "stream" {
            // Try to parse timestamp from filename
            let timestamp_part = &parts[1..parts.len() - 1].join("_");
            if let Ok(timestamp) = Self::parse_timestamp(timestamp_part) {
                let metadata = std::fs::metadata(path).ok()?;
                return Some(Recording::new(
                    "default_stream",
                    timestamp,
                    timestamp + chrono::Duration::seconds(3600), // Default 1 hour duration
                    path.to_path_buf(),
                    metadata.len(),
                    Some("h264".to_string()),
                    Some("aac".to_string()),
                ));
            }
        }

        None
    }

    fn parse_timestamp(timestamp_str: &str) -> Result<DateTime<Utc>> {
        // Parse timestamp in format like "20250101_120000"
        if let Ok(date_time) = chrono::NaiveDateTime::parse_from_str(timestamp_str, "%Y%m%d_%H%M%S") {
            return Ok(DateTime::from_naive_utc_and_offset(date_time, Utc));
        }

        Err(anyhow::anyhow!("Failed to parse timestamp: {}", timestamp_str))
    }

    fn get_file_duration(&self, file_path: &Path) -> Result<f64> {
        // Use absolute path from .env file if available, otherwise fall back to system path
        let ffprobe_path = match std::env::var("FFPROBE_PATH") {
            Ok(path) => path,
            Err(_) => "ffprobe".to_string(),
        };

        let mut cmd = Command::new(&ffprobe_path);

        // Use the exact command that works in terminal
        cmd.arg("-v")
           .arg("error")
           .arg("-show_format")
           .arg("-show_streams")
           .arg(file_path);

        let output = cmd.output().context("Failed to execute ffprobe")?;

        if !output.status.success() {
            return Err(anyhow::anyhow!("ffprobe failed: {}", String::from_utf8_lossy(&output.stderr)));
        }

        let output_str = String::from_utf8_lossy(&output.stdout);

        // Parse the duration from the output
        for line in output_str.lines() {
            if line.starts_with("duration=") {
                let duration_str = &line[9..]; // Remove "duration="
                if let Ok(duration) = duration_str.parse::<f64>() {
                    return Ok(duration);
                }
            }
        }

        Err(anyhow::anyhow!("Failed to find duration in ffprobe output"))
    }

    fn find_closest_recording_file(&self, target_time: &DateTime<Utc>) -> Result<DateTime<Utc>> {
        // Look for stream files in the recordings directory
        let mut candidates = Vec::new();

        if let Ok(entries) = std::fs::read_dir(&self.recordings_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("mkv") {
                    let filename = path.file_name().unwrap().to_str().unwrap();

                    // Parse only stream files
                    if filename.starts_with("stream_") && filename.ends_with(".mkv") {
                        let timestamp_str = &filename[7..filename.len() - 4]; // Remove "stream_" and ".mkv"
                        if let Ok(file_start_time) = Self::parse_timestamp(timestamp_str) {
                            // Check if this recording file is within 1 hour before the target time
                            let time_diff = (*target_time - file_start_time).num_seconds();
                            if time_diff >= 0 && time_diff <= 3600 { // Within 1 hour before target
                                candidates.push((file_start_time, path));
                            }
                        }
                    }
                }
            }
        }

        if candidates.is_empty() {
            return Err(anyhow::anyhow!("No recording files found within 1 hour before target time"));
        }

        // Sort by start time (newest first) and pick the most recent one
        candidates.sort_by(|a, b| b.0.cmp(&a.0));
        let closest = &candidates[0];

        info!("🎯 FOUND CLOSEST RECORDING:");
        info!("  Target time: {}", target_time.format("%Y-%m-%d %H:%M:%S"));
        info!("  Selected file: {}", closest.1.display());
        info!("  File start time: {}", closest.0.format("%Y-%m-%d %H:%M:%S"));
        info!("  Time difference: {}s", (*target_time - closest.0).num_seconds());
        info!("  Candidates found: {}", candidates.len());

        Ok(closest.0)
    }
}