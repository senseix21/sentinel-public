use anyhow::{Context, Result};
use std::process::Command;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamInfo {
    pub codec_name: String,
    pub codec_type: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub frame_rate: Option<String>,
    pub bit_rate: Option<u64>,
    pub method_used: String,
}

pub struct StreamDetector {
    ffprobe_path: String,
    ffmpeg_path: String,
}

impl StreamDetector {
    pub fn new() -> Self {
        Self {
            ffprobe_path: "ffprobe".to_string(),
            ffmpeg_path: "ffmpeg".to_string(),
        }
    }

    pub fn with_paths(ffprobe_path: String, ffmpeg_path: String) -> Self {
        Self {
            ffprobe_path,
            ffmpeg_path,
        }
    }

    /// Detect stream encoding using ffprobe (preferred method)
    pub async fn detect_with_ffprobe(&self, rtsp_url: &str) -> Result<StreamInfo> {
        let mut cmd = Command::new(&self.ffprobe_path);

        cmd.args(&[
            "-v", "error",
            "-select_streams", "v:0",
            "-show_entries", "stream=codec_name,codec_type,width,height,avg_frame_rate,bit_rate",
            "-of", "default=noprint_wrappers=1",
            rtsp_url,
        ]);

        let output = cmd.output()
            .context("Failed to execute ffprobe command")?;

        if !output.status.success() {
            return Err(anyhow::anyhow!("ffprobe failed: {}", String::from_utf8_lossy(&output.stderr)));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        self.parse_ffprobe_output(&stdout, "ffprobe")
    }

    /// Detect stream encoding using ffmpeg (fallback method)
    pub async fn detect_with_ffmpeg(&self, rtsp_url: &str) -> Result<StreamInfo> {
        let mut cmd = Command::new(&self.ffmpeg_path);

        // Just get the first few seconds to analyze the stream
        cmd.args(&[
            "-i", rtsp_url,
            "-t", "3",
            "-f", "null",
            "-",
        ]);

        let output = cmd.output()
            .context("Failed to execute ffmpeg command")?;

        // ffmpeg returns exit code 1 for successful analysis without actual output
        if !output.status.success() && !output.status.code().unwrap_or(1) == 1 {
            return Err(anyhow::anyhow!("ffmpeg failed: {}", String::from_utf8_lossy(&output.stderr)));
        }

        let stdout = String::from_utf8_lossy(&output.stderr);
        self.parse_ffmpeg_output(&stdout, "ffmpeg")
    }

    /// Auto-detect stream encoding (tries ffprobe first, then ffmpeg as fallback)
    pub async fn auto_detect(&self, rtsp_url: &str) -> Result<StreamInfo> {
        match self.detect_with_ffprobe(rtsp_url).await {
            Ok(info) => Ok(info),
            Err(ffprobe_error) => {
                // Try ffmpeg as fallback
                match self.detect_with_ffmpeg(rtsp_url).await {
                    Ok(info) => Ok(info),
                    Err(ffmpeg_error) => Err(anyhow::anyhow!(
                        "Both ffprobe and ffmpeg failed. ffprobe: {}, ffmpeg: {}",
                        ffprobe_error, ffmpeg_error
                    )),
                }
            }
        }
    }

    fn parse_ffprobe_output(&self, output: &str, method: &str) -> Result<StreamInfo> {
        let mut stream_info = StreamInfo {
            codec_name: String::new(),
            codec_type: String::new(),
            width: None,
            height: None,
            frame_rate: None,
            bit_rate: None,
            method_used: method.to_string(),
        };

        for line in output.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            if let Some((key, value)) = line.split_once('=') {
                match key {
                    "codec_name" => stream_info.codec_name = value.to_string(),
                    "codec_type" => stream_info.codec_type = value.to_string(),
                    "width" => stream_info.width = value.parse().ok(),
                    "height" => stream_info.height = value.parse().ok(),
                    "avg_frame_rate" => stream_info.frame_rate = Some(value.to_string()),
                    "bit_rate" => stream_info.bit_rate = value.parse().ok(),
                    _ => {}
                }
            }
        }

        if stream_info.codec_name.is_empty() {
            return Err(anyhow::anyhow!("Failed to extract codec name from ffprobe output"));
        }

        Ok(stream_info)
    }

    fn parse_ffmpeg_output(&self, output: &str, method: &str) -> Result<StreamInfo> {
        let mut stream_info = StreamInfo {
            codec_name: String::new(),
            codec_type: "video".to_string(),
            width: None,
            height: None,
            frame_rate: None,
            bit_rate: None,
            method_used: method.to_string(),
        };

        for line in output.lines() {
            let line = line.trim();
            if line.contains("Video:") {
                // Parse video stream line like: "Stream #0:0: Video: h264 (High), yuv420p, 640x480, 25 fps"
                let parts: Vec<&str> = line.split(',').collect();
                if parts.len() >= 2 {
                    // Extract codec from first part
                    if let Some(video_part) = parts[0].split("Video: ").nth(1) {
                        if let Some(codec_part) = video_part.split('(').next() {
                            stream_info.codec_name = codec_part.trim().to_string();
                        }
                    }

                    // Extract dimensions and frame rate from subsequent parts
                    for part in &parts[1..] {
                        let part = part.trim();

                        // Parse dimensions (e.g., "640x480")
                        if let Some((width_str, height_str)) = part.split_once('x') {
                            stream_info.width = width_str.parse().ok();
                            stream_info.height = height_str.parse().ok();
                        }

                        // Parse frame rate (e.g., "25 fps")
                        if part.ends_with("fps") {
                            let fps_str = part.strip_suffix(" fps").unwrap_or("");
                            stream_info.frame_rate = Some(fps_str.to_string());
                        }
                    }
                }
                break;
            }
        }

        if stream_info.codec_name.is_empty() {
            return Err(anyhow::anyhow!("Failed to extract codec name from ffmpeg output"));
        }

        Ok(stream_info)
    }
}

impl Default for StreamDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_stream_detector_creation() {
        let detector = StreamDetector::new();
        assert_eq!(detector.ffprobe_path, "ffprobe");
        assert_eq!(detector.ffmpeg_path, "ffmpeg");
    }

    #[tokio::test]
    async fn test_stream_detector_with_paths() {
        let detector = StreamDetector::with_paths("custom_ffprobe".to_string(), "custom_ffmpeg".to_string());
        assert_eq!(detector.ffprobe_path, "custom_ffprobe");
        assert_eq!(detector.ffmpeg_path, "custom_ffmpeg");
    }
}