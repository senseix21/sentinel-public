use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::process::{Command, Child};
use tracing::{info, warn};
use crate::frame_overlay::{FrameOverlayConfig, create_centered_red_rectangle};
use crate::stream_detector::{StreamDetector, StreamInfo};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncoderConfig {
    pub source_stream: String,
    pub target_url: String,
    pub preset: String,
    pub crf: u8,
    pub tune: Option<String>,
    pub audio_codec: String,
    pub rtsp_transport: String,
    pub frame_rate: Option<f64>,
    pub force_fps: bool,
    pub overlay_config: Option<FrameOverlayConfig>,
}

impl Default for EncoderConfig {
    fn default() -> Self {
        Self {
            source_stream: String::new(),
            target_url: String::new(),
            preset: "ultrafast".to_string(),
            crf: 28,
            tune: Some("zerolatency".to_string()),
            audio_codec: "aac".to_string(),
            rtsp_transport: "tcp".to_string(),
            frame_rate: None,
            force_fps: false,
            overlay_config: None,
        }
    }
}

impl EncoderConfig {
    pub fn new(source_stream: String, target_url: String) -> Self {
        Self {
            source_stream,
            target_url,
            ..Default::default()
        }
    }

    pub fn with_preset(mut self, preset: String) -> Self {
        self.preset = preset;
        self
    }

    pub fn with_crf(mut self, crf: u8) -> Self {
        self.crf = crf;
        self
    }

    pub fn with_tune(mut self, tune: Option<String>) -> Self {
        self.tune = tune;
        self
    }

    pub fn with_audio_codec(mut self, audio_codec: String) -> Self {
        self.audio_codec = audio_codec;
        self
    }

    pub fn with_rtsp_transport(mut self, transport: String) -> Self {
        self.rtsp_transport = transport;
        self
    }

    pub fn with_frame_rate(mut self, frame_rate: f64) -> Self {
        self.frame_rate = Some(frame_rate);
        self
    }

    pub fn with_fps(mut self, frame_rate: f64, force: bool) -> Self {
        self.frame_rate = Some(frame_rate);
        self.force_fps = force;
        self
    }

    pub fn force_fps(mut self, frame_rate: f64) -> Self {
        self.frame_rate = Some(frame_rate);
        self.force_fps = true;
        self
    }

    pub fn set_frame_rate(&mut self, frame_rate: f64) {
        self.frame_rate = Some(frame_rate);
    }

    pub fn set_force_fps(&mut self, force: bool) {
        self.force_fps = force;
    }

    pub fn with_overlay_config(mut self, overlay_config: FrameOverlayConfig) -> Self {
        self.overlay_config = Some(overlay_config);
        self
    }

    pub fn enable_red_rectangle_overlay(mut self) -> Self {
        self.overlay_config = Some(create_centered_red_rectangle());
        self
    }

    pub fn set_overlay_config(&mut self, overlay_config: Option<FrameOverlayConfig>) {
        self.overlay_config = overlay_config;
    }

    /// Detect video dimensions from the source stream
    async fn detect_video_dimensions(&self) -> Result<(Option<u32>, Option<u32>)> {
        let detector = StreamDetector::new();
        let stream_info = detector.auto_detect(&self.source_stream).await
            .context("Failed to detect video dimensions")?;

        Ok((stream_info.width, stream_info.height))
    }

    /// Build FFmpeg command with video dimension detection for overlays
    pub async fn build_ffmpeg_command_with_dimensions(&self) -> Result<Vec<String>> {
        let mut args = Vec::new();

        // Low latency flags for live streaming
        args.push("-fflags".to_string());
        args.push("nobuffer".to_string());
        args.push("-flags".to_string());
        args.push("low_delay".to_string());

        // Transport protocol
        args.push("-rtsp_transport".to_string());
        args.push(self.rtsp_transport.clone());

        // Input stream
        args.push("-i".to_string());
        args.push(self.source_stream.clone());

        // Video codec - H.265 with x265
        args.push("-c:v".to_string());
        args.push("libx265".to_string());

        // Video preset
        args.push("-preset".to_string());
        args.push(self.preset.clone());

        // CRF (Constant Rate Factor)
        args.push("-crf".to_string());
        args.push(self.crf.to_string());

        // Audio codec
        args.push("-c:a".to_string());
        args.push(self.audio_codec.clone());

        let mut video_filters = Vec::new();

        // Frame rate control
        if let Some(fps) = self.frame_rate {
            if self.force_fps {
                // Force the frame rate to a specific value
                args.push("-r".to_string());
                args.push(fps.to_string());
            }

            // Add frame rate filter
            video_filters.push(format!("fps={}", fps));
        }

        // Get video dimensions for overlay positioning
        let video_dimensions = self.detect_video_dimensions().await;
        let (video_width, video_height) = match video_dimensions {
            Ok(dimensions) => dimensions,
            Err(e) => {
                warn!("Failed to detect video dimensions, overlay may be mispositioned: {}", e);
                (None, None)
            }
        };

        // Add overlay filters if configured
        if let Some(ref overlay_config) = self.overlay_config {
            if let Some(overlay_filter) = overlay_config.build_video_filter(video_width, video_height) {
                video_filters.push(overlay_filter);
            }
        }

        // Add combined video filter if any filters are needed
        if !video_filters.is_empty() {
            args.push("-vf".to_string());
            args.push(video_filters.join(","));
        }

        // H.265 tuning parameters - only include if tune is set and supported
        if let Some(ref tune) = self.tune {
            if !tune.is_empty() && tune != "zerolatency" {
                // Only include x265-params for valid tune options (zerolatency causes issues)
                args.push("-x265-params".to_string());
                args.push(format!("tune={}", tune));
            }
        }

        // Add tune parameter separately for zerolatency (more compatible)
        if let Some(ref tune) = self.tune {
            if tune == "zerolatency" {
                args.push("-tune".to_string());
                args.push("zerolatency".to_string());
            }
        }

        // Output format
        args.push("-f".to_string());
        args.push("rtsp".to_string());

        // Output URL
        args.push(self.target_url.clone());

        Ok(args)
    }

    pub fn build_ffmpeg_command(&self) -> Vec<String> {
        let mut args = Vec::new();

        // Low latency flags for live streaming
        args.push("-fflags".to_string());
        args.push("nobuffer".to_string());
        args.push("-flags".to_string());
        args.push("low_delay".to_string());

        // Transport protocol
        args.push("-rtsp_transport".to_string());
        args.push(self.rtsp_transport.clone());

        // Input stream
        args.push("-i".to_string());
        args.push(self.source_stream.clone());

        // Video codec - H.265 with x265
        args.push("-c:v".to_string());
        args.push("libx265".to_string());

        // Preset
        args.push("-preset".to_string());
        args.push(self.preset.clone());

        // CRF (quality/size tradeoff)
        args.push("-crf".to_string());
        args.push(self.crf.to_string());

        // Build combined video filter if needed
        let mut video_filters = Vec::new();

        // Add frame rate limiting filter
        if let Some(fps) = self.frame_rate {
            if self.force_fps {
                // Force the frame rate to a specific value
                args.push("-r".to_string());
                args.push(fps.to_string());
            }

            // Add frame rate filter
            video_filters.push(format!("fps={}", fps));
        }

        // Add overlay filters if configured
        if let Some(ref overlay_config) = self.overlay_config {
            if let Some(overlay_filter) = overlay_config.build_video_filter(None, None) { // TODO: Could get video dimensions
                video_filters.push(overlay_filter);
            }
        }

        // Add combined video filter if any filters are needed
        if !video_filters.is_empty() {
            args.push("-vf".to_string());
            args.push(video_filters.join(","));
        }

        // H.265 tuning parameters - only include if tune is set and supported
        if let Some(ref tune) = self.tune {
            if !tune.is_empty() && tune != "zerolatency" {
                // Only include x265-params for valid tune options (zerolatency causes issues)
                args.push("-x265-params".to_string());
                args.push(format!("tune={}", tune));
            }
        }

        // Add tune parameter separately for zerolatency (more compatible)
        if let Some(ref tune) = self.tune {
            if tune == "zerolatency" {
                args.push("-tune".to_string());
                args.push("zerolatency".to_string());
            }
        }

        // Audio codec
        args.push("-c:a".to_string());
        args.push(self.audio_codec.clone());

        // Output format
        args.push("-f".to_string());
        args.push("rtsp".to_string());

        // Target URL
        args.push(self.target_url.clone());

        args
    }

    pub fn get_command_string(&self) -> String {
        let mut cmd_string = "ffmpeg".to_string();
        for arg in self.build_ffmpeg_command() {
            cmd_string.push_str(&format!(" {}", arg));
        }
        cmd_string
    }
}

#[derive(Debug, Clone)]
pub struct EncoderStatus {
    pub is_encoding: bool,
    pub source_stream: String,
    pub target_url: String,
    pub config: EncoderConfig,
    pub start_time: Option<chrono::DateTime<Utc>>,
    pub last_error: Option<String>,
}

impl EncoderStatus {
    pub fn new(config: EncoderConfig) -> Self {
        Self {
            is_encoding: false,
            source_stream: config.source_stream.clone(),
            target_url: config.target_url.clone(),
            config,
            start_time: None,
            last_error: None,
        }
    }

    pub fn is_encoding(&self) -> bool {
        self.is_encoding
    }

    pub fn target_url(&self) -> &str {
        &self.target_url
    }

    pub fn start_time(&self) -> Option<chrono::DateTime<Utc>> {
        self.start_time
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
}

pub struct H265Encoder {
    child: Option<Child>,
    status: EncoderStatus,
}

impl H265Encoder {
    pub fn new() -> Self {
        Self {
            child: None,
            status: EncoderStatus::new(EncoderConfig::default()),
        }
    }

    pub fn with_config(config: EncoderConfig) -> Self {
        Self {
            child: None,
            status: EncoderStatus::new(config),
        }
    }

    pub fn get_status(&self) -> &EncoderStatus {
        &self.status
    }

    pub fn get_status_mut(&mut self) -> &mut EncoderStatus {
        &mut self.status
    }

    // Frame rate management methods
    pub fn set_frame_rate(&mut self, fps: f64) {
        self.status.config.set_frame_rate(fps);
    }

    pub fn set_force_fps(&mut self, force: bool) {
        self.status.config.set_force_fps(force);
    }

    pub fn with_frame_rate(&mut self, fps: f64) {
        self.status.config = self.status.config.clone().with_frame_rate(fps);
    }

    pub fn with_fps(&mut self, fps: f64, force: bool) {
        self.status.config = self.status.config.clone().with_fps(fps, force);
    }

    pub fn get_frame_rate(&self) -> Option<f64> {
        self.status.config.frame_rate
    }

    pub fn is_forcing_fps(&self) -> bool {
        self.status.config.force_fps
    }

    pub async fn update_frame_rate(&mut self, fps: f64) -> Result<()> {
        if self.status.is_encoding {
            warn!("Cannot update frame rate while encoding is active. Restart encoding with new settings.");
            return Err(anyhow::anyhow!("Cannot update frame rate while encoding is active"));
        }

        self.set_frame_rate(fps);
        info!("Frame rate updated to: {} FPS", fps);
        Ok(())
    }

    pub async fn force_frame_rate(&mut self, fps: f64) -> Result<()> {
        if self.status.is_encoding {
            warn!("Cannot force frame rate while encoding is active. Restart encoding with new settings.");
            return Err(anyhow::anyhow!("Cannot force frame rate while encoding is active"));
        }

        self.set_force_fps(true);
        self.set_frame_rate(fps);
        info!("Frame rate forced to: {} FPS", fps);
        Ok(())
    }

    pub async fn start_encoding(&mut self, source_stream: String, target_url: String) -> Result<()> {
        let config = EncoderConfig::new(source_stream, target_url);
        self.start_encoding_with_config(config).await
    }

    pub async fn start_encoding_with_config(&mut self, config: EncoderConfig) -> Result<()> {
        if self.status.is_encoding {
            warn!("H.265 encoding is already active");
            return Ok(());
        }

        if config.source_stream.is_empty() {
            return Err(anyhow::anyhow!("Source stream not configured"));
        }

        if config.target_url.is_empty() {
            return Err(anyhow::anyhow!("Target URL not configured"));
        }

        info!("Starting H.265 encoding: {} -> {}", config.source_stream, config.target_url);
        info!("Using preset: {}, CRF: {}", config.preset, config.crf);

        // Build ffmpeg command
        let ffmpeg_args = config.build_ffmpeg_command();
        let ffmpeg_command_string = config.get_command_string();

        info!("Executing: {}", ffmpeg_command_string);

        // Build ffmpeg command
        let mut ffmpeg_cmd = Command::new("ffmpeg");
        ffmpeg_cmd.args(&ffmpeg_args);

        // Start the ffmpeg process
        let child = ffmpeg_cmd.spawn()
            .context("Failed to start ffmpeg process")?;

        // Update status
        self.child = Some(child);
        self.status.is_encoding = true;
        self.status.source_stream = config.source_stream.clone();
        self.status.target_url = config.target_url.clone();
        self.status.config = config;
        self.status.start_time = Some(Utc::now());
        self.status.last_error = None;

        info!("✅ H.265 encoding started successfully");
        Ok(())
    }

    pub async fn stop_encoding(&mut self) -> Result<()> {
        if let Some(mut child) = self.child.take() {
            info!("Stopping H.265 encoding");

            // Kill the ffmpeg process
            match child.kill().await {
                Ok(_) => {
                    info!("✅ H.265 encoding stopped");
                }
                Err(e) => {
                    warn!("Failed to kill ffmpeg process: {}", e);
                    self.status.last_error = Some(e.to_string());
                }
            }

            self.status.is_encoding = false;
            self.status.start_time = None;
        } else {
            warn!("No active encoding process to stop");
        }
        Ok(())
    }

    pub async fn restart_encoding(&mut self, source_stream: String, target_url: String) -> Result<()> {
        info!("Restarting H.265 encoding");
        self.stop_encoding().await?;
        self.start_encoding(source_stream, target_url).await
    }

    pub async fn restart_encoding_with_config(&mut self, config: EncoderConfig) -> Result<()> {
        info!("Restarting H.265 encoding with new config");
        self.stop_encoding().await?;
        self.start_encoding_with_config(config).await
    }

    pub async fn update_config(&mut self, new_config: EncoderConfig) -> Result<()> {
        info!("Updating H.265 encoder config");

        if self.status.is_encoding {
            // If currently encoding, restart with new config
            self.restart_encoding_with_config(new_config).await
        } else {
            // Just update the config
            self.status.config = new_config;
            Ok(())
        }
    }

    pub async fn check_health(&mut self) -> Result<bool> {
        if !self.status.is_encoding {
            return Ok(false);
        }

        if let Some(ref mut child) = self.child {
            // Note: try_status() is not available in tokio::process::Child
            // For now, assume the process is still running if we can't check status
            // In a real implementation, you might want to use other methods to check health
            return Ok(true);
        }

        Ok(false)
    }
}

impl Drop for H265Encoder {
    fn drop(&mut self) {
        if self.status.is_encoding {
            if let Some(mut child) = self.child.take() {
                let _ = child.kill();
            }
        }
    }
}

// Convenience functions for common use cases
pub fn create_h265_encoder(source_stream: String, target_url: String) -> H265Encoder {
    H265Encoder::with_config(EncoderConfig::new(source_stream, target_url))
}

pub fn create_low_latency_h265_encoder(source_stream: String, target_url: String) -> H265Encoder {
    let config = EncoderConfig::new(source_stream, target_url)
        .with_preset("ultrafast".to_string())
        .with_crf(28)
        .with_tune(Some("zerolatency".to_string()))
        .with_audio_codec("aac".to_string())
        .with_rtsp_transport("tcp".to_string());

    H265Encoder::with_config(config)
}

pub fn create_balanced_h265_encoder(source_stream: String, target_url: String) -> H265Encoder {
    let config = EncoderConfig::new(source_stream, target_url)
        .with_preset("fast".to_string())
        .with_crf(23)
        .with_tune(None)
        .with_audio_codec("aac".to_string())
        .with_rtsp_transport("tcp".to_string());

    H265Encoder::with_config(config)
}

pub fn create_fps_limited_encoder(source_stream: String, target_url: String, fps: f64) -> H265Encoder {
    let config = EncoderConfig::new(source_stream, target_url)
        .with_preset("ultrafast".to_string())
        .with_crf(28)
        .with_tune(Some("zerolatency".to_string()))
        .with_audio_codec("aac".to_string())
        .with_rtsp_transport("tcp".to_string())
        .with_frame_rate(fps);

    H265Encoder::with_config(config)
}

pub fn create_fps_limited_encoder_with_overlay(source_stream: String, target_url: String, fps: f64) -> H265Encoder {
    let config = EncoderConfig::new(source_stream, target_url)
        .with_preset("ultrafast".to_string())
        .with_crf(28)
        .with_tune(Some("zerolatency".to_string()))
        .with_audio_codec("aac".to_string())
        .with_rtsp_transport("tcp".to_string())
        .with_frame_rate(fps)
        .enable_red_rectangle_overlay();

    H265Encoder::with_config(config)
}

pub fn create_forced_fps_encoder(source_stream: String, target_url: String, fps: f64) -> H265Encoder {
    let config = EncoderConfig::new(source_stream, target_url)
        .with_preset("ultrafast".to_string())
        .with_crf(28)
        .with_tune(Some("zerolatency".to_string()))
        .with_audio_codec("aac".to_string())
        .with_rtsp_transport("tcp".to_string())
        .with_fps(fps, true);

    H265Encoder::with_config(config)
}

pub fn create_forced_fps_encoder_with_overlay(source_stream: String, target_url: String, fps: f64) -> H265Encoder {
    let config = EncoderConfig::new(source_stream, target_url)
        .with_preset("ultrafast".to_string())
        .with_crf(28)
        .with_tune(Some("zerolatency".to_string()))
        .with_audio_codec("aac".to_string())
        .with_rtsp_transport("tcp".to_string())
        .with_fps(fps, true)
        .enable_red_rectangle_overlay();

    H265Encoder::with_config(config)
}

pub fn create_30fps_encoder(source_stream: String, target_url: String) -> H265Encoder {
    create_fps_limited_encoder(source_stream, target_url, 30.0)
}

pub fn create_25fps_encoder(source_stream: String, target_url: String) -> H265Encoder {
    create_fps_limited_encoder(source_stream, target_url, 25.0)
}

pub fn create_15fps_encoder(source_stream: String, target_url: String) -> H265Encoder {
    create_fps_limited_encoder(source_stream, target_url, 15.0)
}

pub fn create_10fps_encoder(source_stream: String, target_url: String) -> H265Encoder {
    create_fps_limited_encoder(source_stream, target_url, 10.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encoder_config_creation() {
        let config = EncoderConfig::new(
            "rtsp://source:8554/stream".to_string(),
            "rtsp://dest:8554/stream".to_string(),
        );

        assert_eq!(config.source_stream, "rtsp://source:8554/stream");
        assert_eq!(config.target_url, "rtsp://dest:8554/stream");
        assert_eq!(config.preset, "ultrafast");
        assert_eq!(config.crf, 28);
        assert_eq!(config.frame_rate, None);
        assert!(!config.force_fps);
    }

    #[test]
    fn test_frame_rate_configuration() {
        let config = EncoderConfig::new(
            "rtsp://source:8554/stream".to_string(),
            "rtsp://dest:8554/stream".to_string(),
        )
        .with_frame_rate(30.0);

        assert_eq!(config.frame_rate, Some(30.0));
        assert!(!config.force_fps);

        let config2 = config.clone().with_fps(25.0, true);
        assert_eq!(config2.frame_rate, Some(25.0));
        assert!(config2.force_fps);

        let config3 = config2.force_fps(15.0);
        assert_eq!(config3.frame_rate, Some(15.0));
        assert!(config3.force_fps);
    }

    #[test]
    fn test_ffmpeg_command_with_frame_rate() {
        let config = EncoderConfig::new(
            "rtsp://source:8554/stream".to_string(),
            "rtsp://dest:8554/stream".to_string(),
        )
        .with_preset("veryfast".to_string())
        .with_crf(25)
        .with_frame_rate(30.0);

        let args = config.build_ffmpeg_command();

        // Check that frame rate arguments are present
        assert!(args.contains(&"-vf".to_string()));
        assert!(args.contains(&"fps=30".to_string()));

        let config2 = config.clone().with_fps(25.0, true);
        let args2 = config2.build_ffmpeg_command();

        // Check that both -r and -vf are present when forcing FPS
        assert!(args2.contains(&"-r".to_string()));
        assert!(args2.contains(&"25".to_string()));
        assert!(args2.contains(&"-vf".to_string()));
        assert!(args2.contains(&"fps=25".to_string()));
    }

    #[test]
    fn test_ffmpeg_command_building() {
        let config = EncoderConfig::new(
            "rtsp://source:8554/stream".to_string(),
            "rtsp://dest:8554/stream".to_string(),
        )
        .with_preset("veryfast".to_string())
        .with_crf(25)
        .with_audio_codec("aac".to_string());

        let args = config.build_ffmpeg_command();

        // Check that essential arguments are present
        assert!(args.contains(&"-rtsp_transport".to_string()));
        assert!(args.contains(&"tcp".to_string()));
        assert!(args.contains(&"-i".to_string()));
        assert!(args.contains(&"rtsp://source:8554/stream".to_string()));
        assert!(args.contains(&"-c:v".to_string()));
        assert!(args.contains(&"libx265".to_string()));
        assert!(args.contains(&"-preset".to_string()));
        assert!(args.contains(&"veryfast".to_string()));
        assert!(args.contains(&"-crf".to_string()));
        assert!(args.contains(&"25".to_string()));
        assert!(args.contains(&"-c:a".to_string()));
        assert!(args.contains(&"aac".to_string()));
        assert!(args.contains(&"-f".to_string()));
        assert!(args.contains(&"rtsp".to_string()));
        assert!(args.contains(&"rtsp://dest:8554/stream".to_string()));
    }

    #[test]
    fn test_encoder_status() {
        let config = EncoderConfig::new(
            "rtsp://test:8554/stream".to_string(),
            "rtsp://dest:8554/stream".to_string(),
        );

        let mut status = EncoderStatus::new(config.clone());

        assert!(!status.is_encoding());
        assert_eq!(status.source_stream, "rtsp://test:8554/stream");
        assert_eq!(status.target_url(), "rtsp://dest:8554/stream");
        assert_eq!(status.start_time(), None);
        assert_eq!(status.last_error(), None);

        // Simulate encoding started
        status.is_encoding = true;
        status.start_time = Some(Utc::now());

        assert!(status.is_encoding());
        assert!(status.start_time().is_some());
    }
}