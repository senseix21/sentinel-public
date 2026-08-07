use cam_rtsp::frame_overlay::{create_centered_red_rectangle, create_custom_overlay, FrameOverlayConfig};

fn main() {
    println!("Testing frame overlay functionality...");

    // Test 1: Centered red rectangle
    let overlay1 = create_centered_red_rectangle();
    println!("Centered red rectangle: {:?}", overlay1);

    let filter1 = overlay1.build_video_filter(Some(1920), Some(1080));
    println!("Filter with video dimensions: {:?}", filter1);

    let ffmpeg_args1 = overlay1.build_ffmpeg_args(Some(1920), Some(1080));
    println!("FFmpeg args: {:?}", ffmpeg_args1);

    // Test 2: Custom overlay with position
    let overlay2 = create_custom_overlay(100, 100, Some(50), Some(50), "blue", 5);
    println!("\nCustom overlay: {:?}", overlay2);

    let filter2 = overlay2.build_video_filter(Some(1920), Some(1080));
    println!("Filter with video dimensions: {:?}", filter2);

    // Test 3: Default overlay (disabled)
    let overlay3 = FrameOverlayConfig::new();
    println!("\nDefault overlay: {:?}", overlay3);

    let filter3 = overlay3.build_video_filter(Some(1920), Some(1080));
    println!("Filter with video dimensions: {:?}", filter3);

    // Test 4: Test rectangle positioning calculation
    let overlay4 = create_centered_red_rectangle();
    let (x, y) = overlay4.rectangle.get_calculated_position(Some(1920), Some(1080));
    println!("\nCentered position for 1920x1080: x={}, y={}", x, y);

    let (x2, y2) = overlay4.rectangle.get_calculated_position(Some(800), Some(600));
    println!("Centered position for 800x600: x={}, y={}", x2, y2);

    // Test 5: FFmpeg command construction
    println!("\n=== FFmpeg Command Test ===");
    let overlay = create_centered_red_rectangle();
    let mut expected_filters = Vec::new();

    if let Some(fps) = Some(60.0) {
        expected_filters.push(format!("fps={}", fps));
    }

    if let Some(overlay_filter) = overlay.build_video_filter(Some(1920), Some(1080)) {
        expected_filters.push(overlay_filter);
    }

    if !expected_filters.is_empty() {
        println!("Combined filter: -vf {}", expected_filters.join(","));
    }
}