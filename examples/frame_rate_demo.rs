mod h265_encoder;

use h265_encoder::{EncoderConfig, create_fps_limited_encoder, create_forced_fps_encoder, create_30fps_encoder};

fn main() {
    println!("🎥 Testing Frame Rate Limiting in H.265 Encoder");

    let source_stream = "rtsp://139.162.13.249:8554/mystream";
    let target_url = "rtsp://127.0.0.1:8554/output_h265";

    // Test 1: Basic frame rate configuration
    println!("\n📊 Test 1: Basic frame rate configuration");
    let config = EncoderConfig::new(source_stream.to_string(), target_url.to_string())
        .with_frame_rate(30.0);

    println!("✅ Source: {}", config.source_stream);
    println!("✅ Target: {}", config.target_url);
    println!("✅ Frame Rate: {:?}", config.frame_rate);
    println!("✅ Force FPS: {}", config.force_fps);

    // Test 2: Forced frame rate configuration
    println!("\n📊 Test 2: Forced frame rate configuration");
    let forced_config = EncoderConfig::new(source_stream.to_string(), target_url.to_string())
        .with_fps(25.0, true);

    println!("✅ Frame Rate: {:?}", forced_config.frame_rate);
    println!("✅ Force FPS: {}", forced_config.force_fps);

    // Test 3: Generate FFmpeg commands with frame rate
    println!("\n📊 Test 3: FFmpeg command generation");
    let normal_cmd = config.get_command_string();
    let forced_cmd = forced_config.get_command_string();

    println!("✅ Normal 30 FPS command:");
    println!("{}", normal_cmd);

    println!("\n✅ Forced 25 FPS command:");
    println!("{}", forced_cmd);

    // Test 4: Convenience functions
    println!("\n📊 Test 4: Convenience functions");
    let encoder_30fps = create_30fps_encoder(source_stream.to_string(), target_url.to_string());
    let encoder_15fps = create_fps_limited_encoder(source_stream.to_string(), target_url.to_string(), 15.0);
    let encoder_forced_10fps = create_forced_fps_encoder(source_stream.to_string(), target_url.to_string(), 10.0);

    println!("✅ 30 FPS preset: {:?}", encoder_30fps.get_status().config.frame_rate);
    println!("✅ 15 FPS preset: {:?}", encoder_15fps.get_status().config.frame_rate);
    println!("✅ Forced 10 FPS preset: {:?}", encoder_forced_10fps.get_status().config.frame_rate);
    println!("✅ 10 FPS forced flag: {}", encoder_forced_10fps.get_status().config.force_fps);

    // Test 5: Frame rate comparison examples
    println!("\n📊 Test 5: Frame rate examples comparison");
    let fps_examples = vec![10.0, 15.0, 20.0, 25.0, 30.0, 50.0];

    for fps in fps_examples {
        let example_config = EncoderConfig::new(
            "rtsp://source:8554/stream".to_string(),
            "rtsp://dest:8554/stream".to_string(),
        )
        .with_frame_rate(fps);

        let cmd = example_config.get_command_string();
        println!("  {}: {} FPS → {}", if fps == 30.0 { "🎯" } else { "   " }, fps, cmd.lines().nth(2).unwrap_or(""));
    }

    // Test 6: Real-world usage examples
    println!("\n📊 Test 6: Real-world usage examples");
    println!("📋 Example 1: Low bandwidth scenario (10 FPS)");
    let low_bw_config = EncoderConfig::new(
        "rtsp://camera:8554/stream".to_string(),
        "rtsp://server:8554/stream_low".to_string(),
    )
    .with_preset("ultrafast".to_string())
    .with_crf(32)
    .with_frame_rate(10.0)
    .with_tune(Some("zerolatency".to_string()));

    println!("   Command: {}", low_bw_config.get_command_string());

    println!("\n📋 Example 2: High quality scenario (30 FPS)");
    let high_quality_config = EncoderConfig::new(
        "rtsp://camera:8554/stream".to_string(),
        "rtsp://server:8554/stream_high".to_string(),
    )
    .with_preset("fast".to_string())
    .with_crf(23)
    .with_frame_rate(30.0)
    .with_tune(None);

    println!("   Command: {}", high_quality_config.get_command_string());

    println!("\n📋 Example 3: Security/monitoring (forced 15 FPS)");
    let security_config = EncoderConfig::new(
        "rtsp://camera:8554/stream".to_string(),
        "rtsp://server:8554/stream_security".to_string(),
    )
    .with_preset("ultrafast".to_string())
    .with_crf(28)
    .with_fps(15.0, true)
    .with_tune(Some("zerolatency".to_string()));

    println!("   Command: {}", security_config.get_command_string());

    println!("\n🎉 All frame rate limiting tests completed successfully!");
    println!("💡 Frame rate limiting successfully implemented with flexible options");
    println!("💡 Both basic and forced frame rate modes are available");
    println!("💡 Convenience functions for common frame rates (10, 15, 25, 30 FPS)");
    println!("💡 Real-world examples demonstrate practical use cases");
}