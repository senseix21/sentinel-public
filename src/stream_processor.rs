use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::process::{Command, Child};
use tracing::{error, info, warn};

use crate::stream_detector::StreamInfo;
use crate::stream_detector::StreamDetector;
use crate::h265_encoder::EncoderConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForwardingStrategy {
    pub strategy: StrategyType,
    pub source_stream: String,
    pub target_url: String,
    pub h265_config: Option<EncoderConfig>,
    pub original_codec: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StrategyType {
    NativeForwarding,
    H265Reencoding,
}

pub struct StreamProcessor {
    child: Option<Child>,
    current_strategy: Option<ForwardingStrategy>,
    h265_encoder_enabled: bool,
}

impl StreamProcessor {
    pub fn new() -> Self {
        Self {
            child: None,
            current_strategy: None,
            h265_encoder_enabled: false,
        }
    }

    pub fn with_h265_support(enable: bool) -> Self {
        Self {
            child: None,
            current_strategy: None,
            h265_encoder_enabled: enable,
        }
    }

    pub fn is_h265_enabled(&self) -> bool {
        self.h265_encoder_enabled
    }

    pub async fn detect_and_configure_forwarding(
        &mut self,
        source_stream: String,
        target_url: String,
        detector: &crate::stream_detector::StreamDetector,
    ) -> Result<ForwardingStrategy> {
        if source_stream.is_empty() {
            return Err(anyhow::anyhow!("Source stream not configured"));
        }

        if target_url.is_empty() {
            return Err(anyhow::anyhow!("Target URL not configured"));
        }

        info!("🔍 Detecting stream encoding for: {}", source_stream);

        // Step 1: Detect stream encoding
        let stream_info = detector.auto_detect(&source_stream).await
            .context("Failed to detect stream encoding")?;

        info!("📊 Detected stream: Codec={}, Type={}, FPS={:?}, Bitrate={:?}",
            stream_info.codec_name,
            stream_info.codec_type,
            stream_info.frame_rate,
            stream_info.bit_rate
        );

        // Step 2: Determine forwarding strategy
        let strategy = self.determine_strategy(&stream_info).await?;

        info!("🎯 Selected strategy: {:?}", strategy.strategy);

        // Step 3: Configure strategy
        let forwarding_strategy = ForwardingStrategy {
            strategy: strategy.strategy.clone(),
            source_stream: source_stream.clone(),
            target_url: target_url.clone(),
            h265_config: strategy.h265_config,
            original_codec: stream_info.codec_name.clone(),
        };

        self.current_strategy = Some(forwarding_strategy.clone());

        Ok(forwarding_strategy)
    }

    async fn determine_strategy(
        &self,
        stream_info: &StreamInfo,
    ) -> Result<ForwardingStrategy> {
        if !self.h265_encoder_enabled {
            // H.265 encoding disabled - always use native forwarding
            return Ok(ForwardingStrategy {
                strategy: StrategyType::NativeForwarding,
                source_stream: String::new(),
                target_url: String::new(),
                h265_config: None,
                original_codec: stream_info.codec_name.clone(),
            });
        }

        match stream_info.codec_name.as_str() {
            "h264" => {
                // H.264 source - re-encode to H.265
                info!("🔄 H.264 detected - will re-encode to H.265");

                // Create H.265 config with default settings
                let h265_config = EncoderConfig::new(
                    String::new(), // Will be set when starting
                    String::new(), // Will be set when starting
                )
                .with_preset("ultrafast".to_string())
                .with_crf(28)
                .with_tune(Some("zerolatency".to_string()))
                .with_rtsp_transport("tcp".to_string());

                // Add frame rate detection if available
                if let Some(fps) = &stream_info.frame_rate {
                    info!("🎯 Detected original FPS: {}", fps);
                    // Could use this to set frame rate limiting if desired
                }

                Ok(ForwardingStrategy {
                    strategy: StrategyType::H265Reencoding,
                    source_stream: String::new(),
                    target_url: String::new(),
                    h265_config: Some(h265_config),
                    original_codec: "h264".to_string(),
                })
            }
            "hevc" | "h265" => {
                // H.265 source - forward natively
                info!("✅ H.265 detected - will forward natively");
                Ok(ForwardingStrategy {
                    strategy: StrategyType::NativeForwarding,
                    source_stream: String::new(),
                    target_url: String::new(),
                    h265_config: None,
                    original_codec: "h265".to_string(),
                })
            }
            "vp8" | "vp9" => {
                // VP8/VP9 source - forward natively
                info!("✅ {} detected - will forward natively", stream_info.codec_name);
                Ok(ForwardingStrategy {
                    strategy: StrategyType::NativeForwarding,
                    source_stream: String::new(),
                    target_url: String::new(),
                    h265_config: None,
                    original_codec: stream_info.codec_name.clone(),
                })
            }
            _ => {
                // Unknown codec - forward natively with warning
                warn!("⚠️ Unknown codec '{}' detected - will forward natively", stream_info.codec_name);
                Ok(ForwardingStrategy {
                    strategy: StrategyType::NativeForwarding,
                    source_stream: String::new(),
                    target_url: String::new(),
                    h265_config: None,
                    original_codec: stream_info.codec_name.clone(),
                })
            }
        }
    }

    pub async fn start_forwarding(&mut self, source_stream: String, target_url: String) -> Result<()> {
        if let Some(ref strategy) = self.current_strategy {
            let mut strategy_copy = strategy.clone();
            strategy_copy.source_stream = source_stream.clone();
            strategy_copy.target_url = target_url.clone();

            match strategy_copy.strategy {
                StrategyType::H265Reencoding => {
                    self.start_h265_forwarding(&strategy_copy).await
                }
                StrategyType::NativeForwarding => {
                    self.start_native_forwarding(source_stream, target_url).await
                }
            }
        } else {
            error!("❌ No strategy configured. Call detect_and_configure_forwarding first.");
            Err(anyhow::anyhow!("No strategy configured"))
        }
    }

    async fn start_h265_forwarding(&mut self, strategy: &ForwardingStrategy) -> Result<()> {
        info!("🚀 Starting H.265 re-encoding forwarder");
        info!("📡 Source: {}", strategy.source_stream);
        info!("🎯 Target: {}", strategy.target_url);
        info!("📊 Original codec: {}", strategy.original_codec);

        if let Some(ref config) = strategy.h265_config {
            // Build H.265 FFmpeg command
            let ffmpeg_command = self.build_h265_ffmpeg_command(config).await;

            info!("🔧 Executing H.265 FFmpeg command:");
            info!("   {}", ffmpeg_command.join(" "));

            let mut ffmpeg_cmd = Command::new("ffmpeg");
            ffmpeg_cmd.args(&ffmpeg_command);

            match ffmpeg_cmd.spawn()
                .context("Failed to start H.265 FFmpeg process") {
                Ok(child) => {
                    self.child = Some(child);
                    info!("✅ H.265 re-encoding forwarder started successfully");
                    Ok(())
                }
                Err(e) => {
                    error!("❌ Failed to start H.265 re-encoding: {}", e);
                    Err(e)
                }
            }
        } else {
            error!("❌ H.265 strategy configured but no H.265 config available");
            Err(anyhow::anyhow!("H.265 config missing"))
        }
    }

    async fn start_native_forwarding(&mut self, source_stream: String, target_url: String) -> Result<()> {
        info!("🚀 Starting native stream forwarder");
        info!("📡 Source: {}", source_stream);
        info!("🎯 Target: {}", target_url);
        info!("📊 Strategy: Native forwarding (no re-encoding)");

        // Build native FFmpeg command (copy codec)
        let ffmpeg_command = vec![
            "-rtsp_transport".to_string(),
            "tcp".to_string(),
            "-i".to_string(),
            source_stream.clone(),
            "-c:v".to_string(),
            "copy".to_string(), // Copy original video codec
            "-c:a".to_string(),
            "copy".to_string(), // Copy original audio codec
            "-f".to_string(),
            "rtsp".to_string(),
            target_url.clone(),
        ];

        info!("🔧 Executing native FFmpeg command:");
        info!("   {}", ffmpeg_command.join(" "));

        let mut ffmpeg_cmd = Command::new("ffmpeg");
        ffmpeg_cmd.args(&ffmpeg_command);

        match ffmpeg_cmd.spawn()
            .context("Failed to start native FFmpeg process") {
            Ok(child) => {
                self.child = Some(child);
                info!("✅ Native stream forwarder started successfully");
                Ok(())
            }
            Err(e) => {
                error!("❌ Failed to start native forwarding: {}", e);
                Err(e)
            }
        }
    }

    async fn build_h265_ffmpeg_command(&self, config: &EncoderConfig) -> Vec<String> {
        let mut args = Vec::new();

        // Transport protocol
        args.push("-rtsp_transport".to_string());
        args.push("tcp".to_string());

        // Input stream
        args.push("-i".to_string());
        args.push(config.source_stream.clone());

        // Video codec - H.265 with x265
        args.push("-c:v".to_string());
        args.push("libx265".to_string());

        // Preset
        args.push("-preset".to_string());
        args.push(config.preset.clone());

        // CRF (quality/size tradeoff)
        args.push("-crf".to_string());
        args.push(config.crf.to_string());

        // H.265 tuning parameters
        if let Some(ref tune) = config.tune {
            args.push("-x265-params".to_string());
            args.push(format!("tune={}", tune));
        }

        let mut video_filters = Vec::new();

        // Frame rate limiting if configured
        if let Some(fps) = config.frame_rate {
            if config.force_fps {
                // Force the frame rate to a specific value
                args.push("-r".to_string());
                args.push(fps.to_string());
            }

            // Set the output frame rate using video filter
            video_filters.push(format!("fps={}", fps));
        }

        // Add overlay filters if configured
        if let Some(ref overlay_config) = config.overlay_config {
            // Try to detect video dimensions for proper positioning
            let detector = crate::stream_detector::StreamDetector::new();
            if let Ok(stream_info) = detector.auto_detect(&config.source_stream).await {
                if let Some(overlay_filter) = overlay_config.build_video_filter(stream_info.width, stream_info.height) {
                    video_filters.push(overlay_filter);
                }
            } else {
                // Fallback if detection fails
                if let Some(overlay_filter) = overlay_config.build_video_filter(None, None) {
                    video_filters.push(overlay_filter);
                }
            }
        }

        // Add combined video filter if any filters are needed
        if !video_filters.is_empty() {
            args.push("-vf".to_string());
            args.push(video_filters.join(","));
        }

        // Audio codec
        args.push("-c:a".to_string());
        args.push(config.audio_codec.clone());

        // Output format
        args.push("-f".to_string());
        args.push("rtsp".to_string());

        // Target URL
        args.push(config.target_url.clone());

        args
    }

    pub async fn stop_forwarding(&mut self) -> Result<()> {
        if let Some(mut child) = self.child.take() {
            info!("🛑 Stopping stream forwarder");

            // Kill the ffmpeg process
            match child.kill().await {
                Ok(_) => {
                    info!("✅ Stream forwarder stopped");
                }
                Err(e) => {
                    warn!("⚠️ Failed to kill ffmpeg process: {}", e);
                }
            }
        } else {
            warn!("⚠️ No active forwarder process to stop");
        }

        self.current_strategy = None;
        Ok(())
    }

    pub fn get_current_strategy(&self) -> Option<&ForwardingStrategy> {
        self.current_strategy.as_ref()
    }

    pub fn is_forwarding(&self) -> bool {
        self.child.is_some()
    }

    pub async fn restart_forwarding(&mut self, source_stream: String, target_url: String) -> Result<()> {
        info!("🔄 Restarting stream forwarder");
        self.stop_forwarding().await?;
        self.start_forwarding(source_stream, target_url).await
    }
}

impl Drop for StreamProcessor {
    fn drop(&mut self) {
        if self.is_forwarding() {
            if let Some(mut child) = self.child.take() {
                let _ = child.kill();
            }
        }
    }
}