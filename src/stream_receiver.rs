use anyhow::{Context, Result};
use gstreamer::prelude::*;
use gstreamer::ElementFactory;
use gstreamer::*;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info};

use crate::file_recorder::FileRecorderHandle;
use crate::models::SharedState;

pub struct StreamReceiver {
    rtsp_url: String,
    recordings_dir: PathBuf,
    shared_state: Arc<Mutex<SharedState>>,
    recorder_handle: Option<FileRecorderHandle>,
}

#[allow(dead_code)]
impl StreamReceiver {
    pub fn new(
        rtsp_url: String,
        recordings_dir: PathBuf,
        shared_state: Arc<Mutex<SharedState>>,
        recorder_handle: FileRecorderHandle,
    ) -> Self {
        Self {
            rtsp_url,
            recordings_dir,
            shared_state,
            recorder_handle: Some(recorder_handle),
        }
    }

    pub async fn run(&self) -> Result<()> {
        // Initialize GStreamer
        gstreamer::init().context("Failed to initialize GStreamer")?;

        // Create pipeline
        let pipeline = self.create_pipeline()?;

        // Start streaming
        info!("Starting RTSP stream from: {}", self.rtsp_url);

        // Set state to playing
        pipeline
            .set_state(State::Playing)
            .context("Failed to set pipeline to playing state")?;

        // Let the stream run indefinitely until stopped
        let bus = pipeline.bus().unwrap();
        let mut eos_received = false;

        loop {
            match bus.timed_pop(Some(gstreamer::ClockTime::from_mseconds(100))) {
                Some(msg) => {
                    match msg.view() {
                        MessageView::Error(err) => {
                            error!("GStreamer error: {:?}", err);
                        }
                        MessageView::Eos(_) => {
                            info!("End of stream received, finalizing files...");
                            eos_received = true;
                            break;
                        }
                        MessageView::StateChanged(_) => {
                            info!("Pipeline state changed");
                        }
                        _ => (),
                    }
                }
                None => {
                    // Check for Ctrl+C
                    if tokio::signal::ctrl_c().await.is_ok() {
                        info!("Received Ctrl+C, sending EOS to pipeline...");
                        // Send EOS to the pipeline to finalize the file
                        let eos_event = gstreamer::event::Eos::new();
                        pipeline.send_event(eos_event);
                        break;
                    }
                }
            }
        }

        // Wait a bit for EOS to be processed
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // We'll update the file metadata in the pad-added callback when it's available
        // This avoids scope issues with the file_path variable

        // Clean shutdown - set pipeline to NULL state to finalize files
        info!("Shutting down pipeline and finalizing files...");
        pipeline
            .set_state(State::Null)
            .context("Failed to set pipeline to null state")?;

        info!("Pipeline shutdown complete");
        Ok(())
    }

    fn create_pipeline(&self) -> Result<Pipeline> {
        // Create GStreamer elements - exact pattern from working CLI command
        let pipeline = Pipeline::new();
        info!("Pipeline created: {:?}", pipeline);

        info!("Creating pipeline with RTSP source: {}", self.rtsp_url);

        // Use the exact working CLI pattern with pad-added callback for dynamic linking
        // Based on working command: gst-launch-1.0 rtspsrc location=rtsp://139.162.13.249:8554/mystream latency=0 ! rtph264depay ! h264parse ! matroskamux ! filesink location=output.mkv

        // Use rtspsrc - exact pattern from working CLI command
        let src = ElementFactory::make("rtspsrc")
            .name("rtspsrc")
            .property("location", &self.rtsp_url)
            .property("latency", &0u32)
            .build()
            .context("Failed to create rtspsrc element")?;
        info!("rtspsrc element created: {:?}", src);

        // Use rtph264depay - exact pattern from working CLI command
        let rtph264depay = ElementFactory::make("rtph264depay")
            .name("rtph264depay")
            .build()
            .context("Failed to create rtph264depay element")?;
        info!("rtph264depay element created: {:?}", rtph264depay);

        // Use h264parse - exact pattern from working CLI command
        let h264parse = ElementFactory::make("h264parse")
            .name("h264parse")
            .build()
            .context("Failed to create h264parse element")?;
        info!("h264parse element created: {:?}", h264parse);

        // Use matroskamux - exact pattern from working CLI command
        let matroskamux = ElementFactory::make("matroskamux")
            .name("matroskamux")
            .build()
            .context("Failed to create matroskamux element")?;
        info!("matroskamux element created: {:?}", matroskamux);

        // Create sink (filesink for recording) - use timestamped filename
        let timestamp_str = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let file_path = format!("./recordings/stream_{}.mkv", timestamp_str);
        let sink = ElementFactory::make("filesink")
            .name("sink")
            .property("location", &file_path)
            .build()
            .context("Failed to create filesink element")?;
        info!("filesink element created: {:?} (location: {})", sink, file_path);

        // Add elements to pipeline
        pipeline.add(&src)?;
        pipeline.add(&rtph264depay)?;
        pipeline.add(&h264parse)?;
        pipeline.add(&matroskamux)?;
        pipeline.add(&sink)?;

        // Set pipeline to ready state
        pipeline.set_state(State::Ready)?;

        // Link known elements statically
        rtph264depay.link(&h264parse)
            .context("Failed to link rtph264depay to h264parse")?;
        info!("✅ Successfully linked rtph264depay to h264parse");

        h264parse.link(&matroskamux)
            .context("Failed to link h264parse to matroskamux")?;
        info!("✅ Successfully linked h264parse to matroskamux");

        matroskamux.link(&sink)
            .context("Failed to link matroskamux to filesink")?;
        info!("✅ Successfully linked matroskamux to filesink");

        // Set up pad-added callback for rtspsrc to connect to rtph264depay
        let rtph264depay_clone = rtph264depay.clone();
        let file_path_clone = PathBuf::from(&file_path);
        let shared_state_clone = self.shared_state.clone();
        src.connect_pad_added(move |_element, pad| {
            info!("✅ PAD AVAILABLE from rtspsrc: {:?} (caps: {:?})", pad.name(), pad.current_caps());

            // Try to link the pad to rtph264depay
            if let Some(rtph264depay_pad) = rtph264depay_clone.static_pad("sink") {
                match pad.link(&rtph264depay_pad) {
                    Ok(_) => {
                        info!("✅ Successfully linked rtspsrc to rtph264depay");

                        // Add recording to shared state - use blocking lock to avoid async issues
                        let start_time = chrono::Utc::now();
                        let recording = crate::models::Recording::new(
                            "default_stream",
                            start_time,
                            start_time + chrono::Duration::seconds(3600), // Default 1 hour duration
                            file_path_clone.clone(),
                            0, // Will be updated when file is finalized
                            Some("h264".to_string()),
                            Some("aac".to_string()),
                        );

                        let mut state = shared_state_clone.blocking_lock();
                        state.add_recording(recording);
                        info!("✅ Added recording to shared state: {}", file_path_clone.display());
                    }
                    Err(e) => {
                        error!("❌ Failed to link rtspsrc to rtph264depay: {:?}", e);
                    }
                }
            }
        });

        // Set pipeline to playing state - exact pattern from working CLI
        pipeline.set_state(State::Playing)?;
        info!("Pipeline set to playing state");

        Ok(pipeline)
    }
}