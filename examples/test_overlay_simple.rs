use cam_rtsp::frame_overlay::{create_centered_red_rectangle, FrameOverlayConfig};

fn main() {
    println!("Testing overlay functionality...");

    // Create a centered red rectangle overlay
    let overlay = create_centered_red_rectangle();
    println!("Overlay created: {:?}", overlay);

    // Test with known video dimensions (1920x1080)
    let video_width = Some(1920);
    let video_height = Some(1080);

    let filter = overlay.build_video_filter(video_width, video_height);
    println!("Video filter for 1920x1080: {:?}", filter);

    if let Some(filter_str) = filter {
        println!("Filter string: {}", filter_str);

        // Check if the filter contains the correct drawbox syntax
        if filter_str.contains("drawbox=colour=red:") {
            println!("✅ Correct drawbox format found");

            // Parse the filter to verify positioning
            if filter_str.contains("x=910:") && filter_str.contains("y=490:") {
                println!("✅ Correct centered positioning for 1920x1080");
            } else {
                println!("❌ Positioning might be incorrect");
            }
        } else {
            println!("❌ Incorrect drawbox format");
        }
    }

    // Test FFmpeg args
    let ffmpeg_args = overlay.build_ffmpeg_args(video_width, video_height);
    println!("FFmpeg args: {:?}", ffmpeg_args);

    if let Some(vf_index) = ffmpeg_args.iter().position(|arg| arg == "-vf") {
        if vf_index + 1 < ffmpeg_args.len() {
            let filter_arg = &ffmpeg_args[vf_index + 1];
            println!("Filter argument: {}", filter_arg);

            if filter_arg.contains("drawbox=colour=red:x=910:y=490:w=100:h=100:t=3") {
                println!("✅ Perfect! FFmpeg command has correct overlay syntax");
            } else {
                println!("❌ FFmpeg filter syntax needs correction");
            }
        }
    }

    // Test with custom dimensions (1280x720)
    let video_width_720 = Some(1280);
    let video_height_720 = Some(720);
    let filter_720 = overlay.build_video_filter(video_width_720, video_height_720);

    if let Some(filter_str_720) = filter_720 {
        println!("\nFilter for 1280x720: {}", filter_str_720);
        if filter_str_720.contains("x=590:") && filter_str_720.contains("y=310:") {
            println!("✅ Correct centered positioning for 1280x720");
        }
    }

    println!("\n✅ Overlay functionality test completed");
}