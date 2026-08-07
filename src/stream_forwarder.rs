use anyhow::{Context, Result};
use chrono::Utc;
use tokio::process::{Command, Child};
use tracing::{error, info, warn};

use crate::h265_encoder::EncoderConfig;

pub struct StreamForwarder {
    child: Option<Child>,
    target_url: String,
    target_auth: String,
    is_forwarding: bool,
    enable_h265: bool,
    h265_config: Option<EncoderConfig>,
}

impl StreamForwarder {
    pub fn new() -> Self {
        Self {
            child: None,
            target_url: String::new(),
            target_auth: String::new(),
            is_forwarding: false,
            enable_h265: false,
            h265_config: None,
        }
    }

    pub fn with_h265_support(enable: bool, config: Option<EncoderConfig>) -> Self {
        Self {
            child: None,
            target_url: String::new(),
            target_auth: String::new(),
            is_forwarding: false,
            enable_h265: enable,
            h265_config: config,
        }
    }

    pub fn with_config(_config: crate::stream_forwarder::ForwarderConfig) -> Self {
        Self::new()
    }

    pub fn set_target(&mut self, url: String, auth: String) {
        self.target_url = url;
        self.target_auth = auth;
    }

    pub fn set_h265_config(&mut self, config: EncoderConfig) {
        self.h265_config = Some(config);
        self.enable_h265 = true;
    }

    pub fn set_h265_enabled(&mut self, enable: bool) {
        self.enable_h265 = enable;
        if !enable {
            self.h265_config = None;
        }
    }

    pub fn is_h265_enabled(&self) -> bool {
        self.enable_h265
    }

    pub fn is_forwarding(&self) -> bool {
        self.is_forwarding
    }

    pub async fn start_forwarding(&mut self, source_stream: String) -> Result<()> {
        if self.target_url.is_empty() {
            return Err(anyhow::anyhow!("Target URL not configured for forwarding"));
        }

        if self.is_forwarding {
            warn!("Stream forwarding is already active");
            return Ok(());
        }

        if self.enable_h265 && self.h265_config.is_some() {
            self.start_h265_forwarding(source_stream).await
        } else {
            self.start_native_forwarding(source_stream).await
        }
    }

    async fn start_h265_forwarding(&mut self, source_stream: String) -> Result<()> {
        info!("Starting H.265 stream forwarding to: {}", self.target_url);
        info!("Using H.265 encoder with configuration");

        // Use the H.265 encoder config to build the proper FFmpeg command with video dimension detection
        if let Some(ref h265_config) = self.h265_config {
            // Use the H.265 encoder config to build the proper FFmpeg command with video dimensions
            let mut ffmpeg_cmd = Command::new("ffmpeg");
            let args = h265_config.build_ffmpeg_command_with_dimensions().await
                .context("Failed to build FFmpeg command with video dimensions")?;

            // Build and execute the command
            ffmpeg_cmd.args(&args);

            let command_str = format!("ffmpeg {}", args.join(" "));
            info!("Executing: {}", command_str);

            // Start the ffmpeg process
            let child = ffmpeg_cmd.spawn()
                .context("Failed to start H.265 ffmpeg process")?;

            // Store the child process
            self.child = Some(child);
            self.is_forwarding = true;

            info!("✅ H.265 stream forwarding started successfully using ffmpeg to: {}", self.target_url);
        } else {
            return Err(anyhow::anyhow!("H.265 configuration not available"));
        }

        Ok(())
    }

    async fn start_native_forwarding(&mut self, source_stream: String) -> Result<()> {
        info!("Starting native stream forwarding to: {}", self.target_url);
        info!("Using ffmpeg: {} -> {}", source_stream, self.target_url);

        // Build ffmpeg command for native encoding
        let mut ffmpeg_cmd = Command::new("ffmpeg");

        // Set up the command with standard parameters
        ffmpeg_cmd
            .args(&["-rtsp_transport", "tcp"])
            .args(&["-i", &source_stream])
            .args(&["-c:v", "copy"])  // Copy original codec
            .args(&["-c:a", "copy"])  // Copy original audio
            .args(&["-f", "rtsp"])
            .arg(&self.target_url);

        info!("Executing: ffmpeg -rtsp_transport tcp -i {} -c:v copy -c:a copy -f rtsp {}", source_stream, self.target_url);

        // Start the ffmpeg process
        let child = ffmpeg_cmd.spawn()
            .context("Failed to start ffmpeg process")?;

        // Store the child process
        self.child = Some(child);
        self.is_forwarding = true;

        info!("✅ Native stream forwarding started successfully using ffmpeg to: {}", self.target_url);
        Ok(())
    }

    pub async fn stop_forwarding(&mut self) -> Result<()> {
        if let Some(mut child) = self.child.take() {
            info!("Stopping stream forwarding");

            // Kill the ffmpeg process
            match child.kill().await {
                Ok(_) => {
                    info!("✅ Stream forwarding stopped");
                }
                Err(e) => {
                    warn!("Failed to kill ffmpeg process: {}", e);
                }
            }

            self.is_forwarding = false;
        } else {
            warn!("No active forwarding process to stop");
        }
        Ok(())
    }

    pub async fn get_status(&self) -> crate::stream_forwarder::ForwarderStatus {
        crate::stream_forwarder::ForwarderStatus {
            is_forwarding: self.is_forwarding,
            target_url: self.target_url.clone(),
            timestamp: Utc::now(),
        }
    }

    pub async fn restart_forwarding(&mut self, source_stream: String) -> Result<()> {
        info!("Restarting stream forwarding");
        self.stop_forwarding().await?;
        self.start_forwarding(source_stream).await
    }
}

impl Drop for StreamForwarder {
    fn drop(&mut self) {
        if self.is_forwarding {
            if let Some(mut child) = self.child.take() {
                let _ = child.kill();
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ForwarderConfig {
    pub target_url: String,
    pub target_auth: String,
    pub source_stream: String,
}

#[derive(Debug, Clone)]
pub struct ForwarderStatus {
    pub is_forwarding: bool,
    pub target_url: String,
    pub timestamp: chrono::DateTime<Utc>,
}

#[allow(dead_code)]
impl ForwarderStatus {
    pub fn is_forwarding(&self) -> bool {
        self.is_forwarding
    }

    pub fn target_url(&self) -> &str {
        &self.target_url
    }

    pub fn timestamp(&self) -> chrono::DateTime<Utc> {
        self.timestamp
    }
}