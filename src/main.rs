use clap::Parser;
use dotenv::dotenv;
use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info};
use tracing_subscriber;

pub mod api;
pub mod clip_extractor;
pub mod file_recorder;
pub mod frame_overlay;
pub mod h265_encoder;
pub mod mediamtx;
pub mod models;
pub mod stream_detector;
pub mod stream_forwarder;
pub mod stream_processor;
pub mod stream_receiver;

use crate::clip_extractor::ClipExtractor;
use crate::stream_detector::StreamDetector;
use crate::stream_forwarder::StreamForwarder;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long, default_value = "139.162.13.249:8554")]
    rtsp_server_addr: String,

    #[arg(
        short = 'r',
        long,
        default_value = "rtsp://139.162.13.249:8554/mystream"
    )]
    stream_url: String,

    #[arg(short = 'd', long, default_value = "./recordings")]
    recordings_dir: String,

    #[arg(short = 'a', long, default_value = "0.0.0.0:3000")]
    api_addr: String,

    #[arg(long, default_value = "debug")]
    log_level: String,

    #[arg(short = 'm', long, default_value = "./mediamtx.yml")]
    mediamtx_config: String,

    #[arg(long)]
    enable_mediamtx: bool,

    // Stream forwarding configuration
    #[arg(long, default_value = "")]
    forward_target_url: String,

    #[arg(long, default_value = "")]
    forward_target_auth: String,

    #[arg(long)]
    enable_forwarding: bool,

    #[arg(long)]
    auto_start_forwarding: bool,

    // H.265 encoding configuration
    #[arg(long, default_value = "false")]
    enable_h265: bool,

    #[arg(long, default_value = "ultrafast")]
    h265_preset: String,

    #[arg(long, default_value = "28")]
    h265_crf: u8,

    #[arg(long, default_value = "zerolatency")]
    h265_tune: Option<String>,

    // Frame rate limiting configuration
    #[arg(long)]
    frame_rate: Option<f64>,

    #[arg(long, default_value = "false")]
    force_frame_rate: bool,

    // Frame overlay configuration
    #[arg(long, default_value = "false")]
    enable_frame_overlay: bool,

    #[arg(long, default_value = "100")]
    overlay_width: u32,

    #[arg(long, default_value = "100")]
    overlay_height: u32,

    #[arg(long)]
    overlay_x: Option<u32>,

    #[arg(long)]
    overlay_y: Option<u32>,

    #[arg(long, default_value = "red")]
    overlay_color: String,

    #[arg(long, default_value = "3")]
    overlay_line_width: u32,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env file if it exists
    dotenv().ok();

    let args = Args::parse();

    // Use environment variable for recordings directory if not provided via CLI
    let recordings_dir = env::var("RECORDINGS_DIR")
        .unwrap_or_else(|_| args.recordings_dir.clone());

    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing_subscriber::filter::LevelFilter::INFO)
        .init();

    info!("Starting GStreamer Streaming Service");
    info!("RTSP Server: {}", args.rtsp_server_addr);
    info!("Stream URL: {}", args.stream_url);
    info!("Recordings Directory: {}", recordings_dir);
    info!("API Server: {}", args.api_addr);
    if args.enable_mediamtx {
        info!("MediaMTX Config: {}", args.mediamtx_config);
    }
    if args.enable_forwarding {
        info!("Stream Forwarding: Enabled");
        info!("Forward Target: {}", args.forward_target_url);
        if !args.forward_target_auth.is_empty() {
            info!("Forward Auth: Enabled");
        } else {
            info!("Forward Auth: Disabled");
        }
    } else {
        info!("Stream Forwarding: Disabled");
    }

    // H.265 encoding configuration
    if args.enable_h265 {
        info!("H.265 Encoding: Enabled");
        info!("H.265 Preset: {}", args.h265_preset);
        info!("H.265 CRF: {}", args.h265_crf);
        if let Some(ref tune) = args.h265_tune {
            info!("H.265 Tune: {}", tune);
        } else {
            info!("H.265 Tune: None");
        }
    } else {
        info!("H.265 Encoding: Disabled");
    }

    // Frame rate configuration
    if let Some(fps) = args.frame_rate {
        info!("Frame Rate Limiting: Enabled");
        info!("Target Frame Rate: {} FPS", fps);
        if args.force_frame_rate {
            info!("Frame Rate Mode: Forced");
        } else {
            info!("Frame Rate Mode: Limited");
        }
    } else {
        info!("Frame Rate Limiting: Disabled");
    }

    // Frame overlay configuration
    if args.enable_frame_overlay {
        info!("Frame Overlay: Enabled");
        info!("Overlay Size: {}x{}", args.overlay_width, args.overlay_height);
        if args.overlay_x.is_some() || args.overlay_y.is_some() {
            info!("Overlay Position: {}x{}", args.overlay_x.unwrap_or(0), args.overlay_y.unwrap_or(0));
        } else {
            info!("Overlay Position: Centered");
        }
        info!("Overlay Color: {}", args.overlay_color);
        info!("Overlay Line Width: {}", args.overlay_line_width);
    } else {
        info!("Frame Overlay: Disabled");
    }

    // Create shared state
    let shared_state = Arc::new(Mutex::new(models::SharedState::new()));

    // Create PathBuf instances for constructor calls
    let recordings_dir_path: PathBuf = recordings_dir.clone().into();
    let _mediamtx_config_path: PathBuf = args.mediamtx_config.clone().into();

    // Scan existing recordings on startup
    info!("Scanning existing recordings on startup...");
    let clip_extractor = ClipExtractor::new(recordings_dir_path.clone());
    if let Err(e) = clip_extractor.scan_and_update_recordings_async(&shared_state).await {
        error!("Failed to scan existing recordings: {}", e);
    } else {
        info!("✅ Completed startup recording scan");
    }

    // Initialize stream forwarder with H.265 support
    let mut h265_config = None;
    if args.enable_h265 {
        info!("Creating H.265 encoder configuration");
        h265_config = Some(h265_encoder::EncoderConfig::new(
            args.stream_url.clone(),
            args.forward_target_url.clone(),
        )
        .with_preset(args.h265_preset.clone())
        .with_crf(args.h265_crf)
        .with_tune(args.h265_tune.clone())
        .with_rtsp_transport("tcp".to_string()));

        // Add frame overlay configuration if enabled
        if args.enable_frame_overlay {
            info!("Adding frame overlay to H.265 encoder");
            let x = args.overlay_x.unwrap_or(0);
            let y = args.overlay_y.unwrap_or(0);
            h265_config = Some(h265_config.unwrap().with_overlay_config(
                crate::frame_overlay::create_custom_overlay(
                    x,
                    y,
                    args.overlay_width,
                    args.overlay_height,
                    &args.overlay_color,
                    args.overlay_line_width,
                )
            ));
        }

        // Add frame rate configuration if specified
        if let Some(fps) = args.frame_rate {
            if args.force_frame_rate {
                h265_config = Some(h265_config.unwrap().with_fps(fps, true));
            } else {
                h265_config = Some(h265_config.unwrap().with_frame_rate(fps));
            }
        }
    }

    let mut forwarder = StreamForwarder::with_h265_support(args.enable_h265, h265_config.clone());

    if args.enable_forwarding {
        if !args.forward_target_url.is_empty() {
            forwarder.set_target(args.forward_target_url.clone(), args.forward_target_auth.clone());

            if args.enable_h265 {
                info!("Stream forwarder initialized with H.265 encoding to: {}", args.forward_target_url);
                if let Some(ref config) = h265_config {
                    info!("H.265 encoder preset: {}, CRF: {}", config.preset, config.crf);
                    if config.frame_rate.is_some() {
                        info!("Frame rate: {} FPS (mode: {})",
                            config.frame_rate.unwrap(),
                            if config.force_fps { "forced" } else { "limited" }
                        );
                    }
                }
            } else {
                info!("Stream forwarder initialized with native encoding to: {}", args.forward_target_url);
            }
        } else {
            error!("Stream forwarding enabled but no target URL provided");
        }
    }

    // Start file recorder
    let mut recorder = file_recorder::FileRecorder::new(recordings_dir_path.clone());
    let recorder_handle = recorder.start();

    // MediaMTX initialization (disabled for now due to library issues)
    if args.enable_mediamtx {
        info!("MediaMTX startup disabled - GStreamer library issues on macOS");
        // TODO: Fix GStreamer library issues first
    }

    // Start stream receiver
    let receiver = stream_receiver::StreamReceiver::new(
        args.stream_url.clone(),
        recordings_dir_path.clone(),
        shared_state.clone(),
        recorder_handle,
    );
    let receiver_handle = tokio::spawn(async move {
        if let Err(e) = receiver.run().await {
            error!("Stream receiver error: {}", e);
        }
    });

    // Auto-start stream forwarding if enabled
    if args.auto_start_forwarding && args.enable_forwarding {
        info!("Auto-starting stream forwarding...");
        // Use existing forwarder with H.265 and frame rate config
        forwarder.set_target(args.forward_target_url.clone(), args.forward_target_auth.clone());
        if let Err(e) = forwarder.start_forwarding(args.stream_url.clone()).await {
            error!("Failed to auto-start stream forwarding: {}", e);
        } else {
            info!("✅ Stream forwarding auto-started successfully");
        }
    }

    // Initialize stream detector
    let stream_detector = StreamDetector::new();

    // Start API server
    let recordings_dir_copy = recordings_dir_path.clone();
    let clip_extractor = ClipExtractor::new(recordings_dir_path.clone());
    let api_server = api::ApiServer::new(
        shared_state.clone(),
        recordings_dir_copy,
        args.api_addr.clone(),
        clip_extractor,
        forwarder,
        stream_detector,
    );
    let api_handle = tokio::spawn(async move {
        if let Err(e) = api_server.start().await {
            error!("API server error: {}", e);
        }
    });

    // Wait for all tasks to complete
    tokio::select! {
        result = receiver_handle => {
            if let Err(e) = result {
                error!("Receiver task error: {}", e);
            }
        }
        result = api_handle => {
            if let Err(e) = result {
                error!("API task error: {}", e);
            }
        }
    }

    Ok(())
}
