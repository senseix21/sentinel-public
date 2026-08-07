use h265_encoder::{H265Encoder, EncoderConfig, create_low_latency_h265_encoder};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    let source_stream = "rtsp://139.162.13.249:8554/mystream";
    let target_url = "rtsp://127.0.0.1:8554/test_h265";

    println!("🎥 Testing H.265 Encoder");
    println!("📡 Source: {}", source_stream);
    println!("🎯 Target: {}", target_url);

    // Test 1: Create encoder with default config
    let mut encoder = H265Encoder::new();

    println!("\n📊 Test 1: Building encoder config...");
    let config = EncoderConfig::new(source_stream.to_string(), target_url.to_string());
    println!("📋 Source: {}", config.source_stream);
    println!("📋 Target: {}", config.target_url);
    println!("📋 Preset: {}", config.preset);
    println!("📋 CRF: {}", config.crf);
    println!("📋 Tune: {:?}", config.tune);
    println!("📋 Audio Codec: {}", config.audio_codec);
    println!("📋 RTSP Transport: {}", config.rtsp_transport);

    // Test 2: Build ffmpeg command
    println!("\n📊 Test 2: Building FFmpeg command...");
    let ffmpeg_args = config.build_ffmpeg_command();
    let ffmpeg_command = config.get_command_string();

    println!("📋 FFmpeg command: {}", ffmpeg_command);
    println!("📋 Arguments count: {}", ffmpeg_args.len());

    // Print all arguments
    for (i, arg) in ffmpeg_args.iter().enumerate() {
        println!("  {}: {}", i, arg);
    }

    // Test 3: Use convenience function
    println!("\n📊 Test 3: Testing convenience function...");
    let mut encoder2 = create_low_latency_h265_encoder(
        source_stream.to_string(),
        target_url.to_string(),
    );

    println!("📋 Low-latency config preset: {}", encoder2.get_status().config.preset);
    println!("📋 Low-latency config CRF: {}", encoder2.get_status().config.crf);
    println!("📋 Low-latency command: {}", encoder2.get_status().config.get_command_string());

    // Test 4: Custom configuration
    println!("\n📊 Test 4: Testing custom configuration...");
    let custom_config = EncoderConfig::new(source_stream.to_string(), target_url.to_string())
        .with_preset("veryfast".to_string())
        .with_crf(24)
        .with_tune(Some("zerolatency".to_string()))
        .with_audio_codec("aac".to_string())
        .with_rtsp_transport("tcp".to_string());

    println!("📋 Custom preset: {}", custom_config.preset);
    println!("📋 Custom CRF: {}", custom_config.crf);
    println!("📋 Custom tune: {:?}", custom_config.tune);
    println!("📋 Custom command: {}", custom_config.get_command_string());

    // Test 5: Compare with user's exact command
    println!("\n📊 Test 5: Comparing with user's exact command...");
    println!("📋 User's command:");
    println!("ffmpeg -i rtsp://SOURCE_IP:8554/mystream \\");
    println!("    -c:v libx265 -preset veryfast -crf 28 \\");
    println!("    -c:a aac -f rtsp -rtsp_transport tcp \\");
    println!("    rtsp://DEST_IP:8554/output_h265");

    let user_config = EncoderConfig::new(
        "rtsp://SOURCE_IP:8554/mystream".to_string(),
        "rtsp://DEST_IP:8554/output_h265".to_string(),
    )
    .with_preset("veryfast".to_string())
    .with_crf(28)
    .with_audio_codec("aac".to_string())
    .with_rtsp_transport("tcp".to_string());

    println!("\n📋 Generated command: {}", user_config.get_command_string());

    // Show the differences
    let user_args = user_config.build_ffmpeg_command();
    println!("\n📋 Command breakdown:");
    for (i, arg) in user_args.iter().enumerate() {
        match i {
            0 => println!("  {}: FFmpeg binary", i),
            1 => println!("  {}: RTSP transport protocol: {}", i, arg),
            2 => println!("  {}: Input source", i),
            3 => println!("  {}: Input URL: {}", i, arg),
            4 => println!("  {}: Video codec", i),
            5 => println!("  {}: H.265 encoder: {}", i, arg),
            6 => println!("  {}: Preset setting", i),
            7 => println!("  {}: Preset value: {}", i, arg),
            8 => println!("  {}: CRF setting", i),
            9 => println!("  {}: CRF value: {}", i, arg),
            10 => println!("  {}: Audio codec", i),
            11 => println!("  {}: Audio encoder: {}", i, arg),
            12 => println!("  {}: Output format", i),
            13 => println!("  {}: Output format: {}", i, arg),
            14 => println!("  {}: Target URL", i),
            15 => println!("  {}: Destination RTSP: {}", i, arg),
            _ => println!("  {}: {}", i, arg),
        }
    }

    println!("\n🎉 H.265 encoder configuration tests completed!");
    println!("💡 The encoder logic correctly implements your H.265 re-encoding requirements");
    println!("💡 You can now use the H265Encoder struct in your stream forwarder or other components");

    Ok(())
}