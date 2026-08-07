use anyhow::{Context, Result};
use gstreamer::{prelude::*};
use tracing::{debug, info, error};

fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Initialize GStreamer
    gstreamer::init().context("Failed to initialize GStreamer")?;

    let target_url = "rtsp://192.168.1.157:8554/forwarded_stream";
    let source_stream = "rtsp://139.162.13.249:8554/mystream";

    info!("Testing stream forwarding to: {}", target_url);

    // Create pipeline: rtspsrc -> decodebin -> rtspclientsink
    let pipeline = gstreamer::Pipeline::new();

    // Use uridecodebin which works better for RTSP streams
    let src = gstreamer::ElementFactory::make("uridecodebin")
        .name("source")
        .property("uri", &source_stream)
        .build()?;

    // Create RTSP sink for publishing
    let rtsp_sink = gstreamer::ElementFactory::make("rtspclientsink")
        .name("rtspclientsink")
        .property("location", target_url)
        .build()?;

    // Add elements to pipeline
    pipeline.add_many(&[&src, &rtsp_sink])?;

    // Connect pad-added signal from uridecodebin
    src.connect_pad_added(move |_element, pad| {
        let pad_name = pad.name();
        debug!("Pad available from uridecodebin: {}", pad_name);

        if pad_name.starts_with("src_") {
            debug!("Found source pad from uridecodebin, linking to rtsp sink directly");

            // Try to link directly to rtspclientsink
            if let Some(sink_pad) = rtsp_sink.static_pad("sink") {
                if let Err(e) = pad.link(&sink_pad) {
                    error!("Failed to link uridecodebin to rtspclientsink: {}", e);
                } else {
                    info!("Successfully linked uridecodebin to rtspclientsink");
                }
            } else {
                error!("Could not get sink pad from rtspclientsink");
            }
        }
    });

    // Set pipeline state to playing
    info!("Setting pipeline to playing state...");
    pipeline.set_state(gstreamer::State::Playing)?;

    info!("✅ Test pipeline started successfully. Check if forwarding works at: {}", target_url);

    // Let it run for a bit
    std::thread::sleep(std::time::Duration::from_secs(10));

    // Cleanup
    info!("Stopping pipeline...");
    pipeline.set_state(gstreamer::State::Null)?;

    info!("✅ Test completed");
    Ok(())
}