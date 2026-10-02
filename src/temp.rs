use anyhow::{Context, Result};
use gstreamer::prelude::*;
use gstreamer::*;
use tracing::{error, info};

fn main() -> Result<()> {
    // Initialize GStreamer
    gstreamer::init().context("Failed to initialize GStreamer")?;

    // RTSP URL
    let rtsp_url = "rtsp://139.162.13.249:8554/mystream";

    // Create GStreamer elements
    let pipeline = Pipeline::new();
    info!("Pipeline created: {:?}", pipeline);

    // Use uridecodebin for RTSP streaming.
    let src = ElementFactory::make("uridecodebin")
        .name("source")
        .property("uri", &rtsp_url)
        .build()
        .context("Failed to create uridecodebin element")?;
    info!("uridecodebin element created: {:?}", src);

    // Create sink (filesink for recording)
    let sink = ElementFactory::make("filesink")
        .name("sink")
        .property("location", "./test_record.mkv")
        .build()
        .context("Failed to create filesink element")?;
    info!("filesink element created: {:?}", sink);

    // Add elements to pipeline
    pipeline.add(&src)?;
    pipeline.add(&sink)?;

    // uridecodebin creates its own pads when it starts decoding
    src.connect_pad_added(move |_element, pad| {
        info!("Pad available from uridecodebin: {:?}", pad.name());
        if let Some(sink_pad) = sink.static_pad("sink") {
            if let Err(e) = pad.link(&sink_pad) {
                error!("Failed to link uridecodebin to filesink: {:?}", e);
            }
        }
    });

    // Set pipeline to playing state
    pipeline.set_state(State::Playing)?;

    info!("Starting RTSP stream from: {}", rtsp_url);

    // Run main loop
    let main_loop = glib::MainLoop::new(None, false);
    main_loop.run();

    // Clean up

    let _ = pipeline.set_state(State::Null);
    Ok(())
}
