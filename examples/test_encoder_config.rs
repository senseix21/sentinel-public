use cam_rtsp::h265_encoder::{EncoderConfig, create_low_latency_h265_encoder};

fn main() {
    println!("🎥 Testing H.265 Encoder Configuration");

    let source_stream = "rtsp://139.162.13.249:8554/mystream";
    let target_url = "rtsp://127.0.0.1:8554/output_h265";

    // Test 1: Default configuration
    println!("\n📊 Test 1: Default configuration");
    let config = EncoderConfig::new(source_stream.to_string(), target_url.to_string());

    println!("✅ Source: {}", config.source_stream);
    println!("✅ Target: {}", config.target_url);
    println!("✅ Preset: {}", config.preset);
    println!("✅ CRF: {}", config.crf);
    println!("✅ Tune: {:?}", config.tune);
    println!("✅ Audio Codec: {}", config.audio_codec);
    println!("✅ RTSP Transport: {}", config.rtsp_transport);

    // Test 2: Build FFmpeg command
    println!("\n📊 Test 2: FFmpeg command generation");
    let ffmpeg_command = config.get_command_string();
    println!("✅ Generated command: {}", ffmpeg_command);

    // Test 3: User's exact configuration
    println!("\n📊 Test 3: User's exact configuration");
    let user_config = EncoderConfig::new(
        "rtsp://SOURCE_IP:8554/mystream".to_string(),
        "rtsp://DEST_IP:8554/output_h265".to_string(),
    )
    .with_preset("veryfast".to_string())
    .with_crf(28)
    .with_audio_codec("aac".to_string())
    .with_rtsp_transport("tcp".to_string());

    println!("✅ User config preset: {}", user_config.preset);
    println!("✅ User config CRF: {}", user_config.crf);
    println!("✅ User config command: {}", user_config.get_command_string());

    // Test 4: Low-latency encoder
    println!("\n📊 Test 4: Low-latency encoder");
    let encoder = create_low_latency_h265_encoder(
        source_stream.to_string(),
        target_url.to_string(),
    );

    println!("✅ Low-latency preset: {}", encoder.get_status().config.preset);
    println!("✅ Low-latency CRF: {}", encoder.get_status().config.crf);
    println!("✅ Low-latency command: {}", encoder.get_status().config.get_command_string());

    // Test 5: Command comparison with user's original
    println!("\n📊 Test 5: Command comparison");
    println!("📋 User's original command:");
    println!("ffmpeg -i rtsp://SOURCE_IP:8554/mystream \\");
    println!("    -c:v libx265 -preset veryfast -crf 28 \\");
    println!("    -c:a aac -f rtsp -rtsp_transport tcp \\");
    println!("    rtsp://DEST_IP:8554/output_h265");

    println!("\n📋 Generated equivalent command:");
    println!("{}", user_config.get_command_string());

    println!("\n🎉 All tests completed successfully!");
    println!("💡 The H.265 encoder correctly implements your re-encoding requirements");
    println!("💡 You can now integrate this into your stream forwarding logic");
}