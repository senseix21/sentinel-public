fn main() {
    println!("🎥 CLI Usage Examples for H.265 Encoding and Frame Rate Control");
    println!("================================================================");

    let source_stream = "rtsp://139.162.13.249:8554/mystream";
    let target_url = "rtsp://127.0.0.1:8554/output";

    println!("\n📡 Basic Usage (Native Encoding):");
    println!("cargo run --bin cam_rtsp");
    println!("   - Uses original codec without re-encoding");

    println!("\n🎯 H.265 Encoding Examples:");
    println!("\n1. Basic H.265 encoding:");
    println!("cargo run --bin cam_rtsp \\");
    println!("    --enable-forwarding \\");
    println!("    --forward-target-url {} \\", target_url);
    println!("    --enable-h265");

    println!("\n2. H.265 with custom settings:");
    println!("cargo run --bin cam_rtsp \\");
    println!("    --enable-forwarding \\");
    println!("    --forward-target-url {} \\", target_url);
    println!("    --enable-h265 \\");
    println!("    --h265-preset fast \\");
    println!("    --h265-crf 24 \\");
    println!("    --h265-tune zerolatency");

    println!("\n3. H.265 with frame rate limiting:");
    println!("cargo run --bin cam_rtsp \\");
    println!("    --enable-forwarding \\");
    println!("    --forward-target-url {} \\", target_url);
    println!("    --enable-h265 \\");
    println!("    --frame-rate 30.0");

    println!("\n4. H.265 with forced frame rate:");
    println!("cargo run --bin cam_rtsp \\");
    println!("    --enable-forwarding \\");
    println!("    --forward-target-url {} \\", target_url);
    println!("    --enable-h265 \\");
    println!("    --frame-rate 25.0 \\");
    println!("    --force-frame-rate");

    println!("\n5. Full featured H.265 with frame rate:");
    println!("cargo run --bin cam_rtsp \\");
    println!("    --enable-forwarding \\");
    println!("    --forward-target-url {} \\", target_url);
    println!("    --enable-h265 \\");
    println!("    --h265-preset ultrafast \\");
    println!("    --h265-crf 28 \\");
    println!("    --h265-tune zerolatency \\");
    println!("    --frame-rate 15.0 \\");
    println!("    --force-frame-rate");

    println!("\n🎚️ Frame Rate Control Scenarios:");
    println!("\n✅ Frame Rate Limiting (drops frames if needed):");
    println!("   --frame-rate 30.0   # Limits to 30 FPS maximum");
    println!("   --frame-rate 15.0   # Limits to 15 FPS maximum");
    println!("   --frame-rate 10.0   # Limits to 10 FPS (bandwidth saving)");

    println!("\n🔄 Forced Frame Rate (re-timestamps frames):");
    println!("   --frame-rate 25.0 --force-frame-rate   # Forces exact 25 FPS");
    println!("   --frame-rate 30.0 --force-frame-rate   # Forces exact 30 FPS");
    println!("   --frame-rate 10.0 --force-frame-rate   # Forces exact 10 FPS");

    println!("\n💡 Use Cases:");
    println!("\n📡 Low Bandwidth (Remote Monitoring):");
    println!("cargo run --bin cam_rtsp \\");
    println!("    --enable-forwarding \\");
    println!("    --forward-target-url rtsp://server:8554/remote \\");
    println!("    --enable-h265 \\");
    println!("    --h265-crf 32 \\");
    println!("    --frame-rate 10.0 \\");
    println!("    --h265-preset ultrafast");
    println!("   Result: Low bandwidth usage, lower quality but stable stream");

    println!("\n🎮 High Quality (Local Recording):");
    println!("cargo run --bin cam_rtsp \\");
    println!("    --enable-forwarding \\");
    println!("    --forward-target-url rtsp://server:8554/local \\");
    println!("    --enable-h265 \\");
    println!("    --h265-crf 20 \\");
    println!("    --frame-rate 30.0 \\");
    println!("    --h265-preset fast");
    println!("   Result: High quality, 30 FPS, optimal compression");

    println!("\n🔒 Security (Consistent 15 FPS):");
    println!("cargo run --bin cam_rtsp \\");
    println!("    --enable-forwarding \\");
    println!("    --forward-target-url rtsp://server:8554/security \\");
    println!("    --enable-h265 \\");
    println!("    --h265-crf 26 \\");
    println!("    --frame-rate 15.0 \\");
    println!("    --force-frame-rate");
    println!("   Result: Consistent 15 FPS, reliable for motion detection");

    println!("\n📊 Bandwidth Comparison:");
    println!("\nNative H.264 (Original):");
    println!("   - Bandwidth: ~2-4 Mbps");
    println!("   - Quality: Original");
    println!("   - CPU: Low");
    println!("   Command: (no --enable-h265 flag)");

    println!("\nH.265 (ultrafast, 30 FPS):");
    println!("   - Bandwidth: ~1-2 Mbps (50% reduction)");
    println!("   - Quality: Slightly compressed");
    println!("   - CPU: Medium");
    println!("   Command: --enable-h265 --frame-rate 30.0");

    println!("\nH.265 (ultrafast, 10 FPS):");
    println!("   - Bandwidth: ~0.5-1 Mbps (75% reduction)");
    println!("   - Quality: Lower but usable");
    println!("   - CPU: Low (due to lower frame rate)");
    println!("   Command: --enable-h265 --frame-rate 10.0");

    println!("\n⚡ Performance Presets:");
    println!("\n🚀 Ultrafast (Recommended for real-time):");
    println!("   --h265-preset ultrafast");
    println!("   - Best latency");
    println!("   - Lower compression efficiency");
    println!("   - Recommended for streaming");

    println!("\n⚡ Fast (Balanced performance):");
    println!("   --h265-preset fast");
    println!("   - Good balance of speed/quality");
    println!("   - Recommended for recordings");

    println!("\n🎚️ CRF (Compression Quality):");
    println!("\nLow numbers = Higher quality = More bandwidth");
    println!("   --h265-crf 18  # Visually lossless (high bandwidth)");
    println!("   --h265-crf 24  # Good quality (default)");
    println!("   --h265-crf 28  # Lower quality (less bandwidth)");
    println!("   --h265-crf 32  # Very low quality (minimal bandwidth)");

    println!("\n📝 Complete Example Commands:");
    println!("\n🔹 Low Bandwidth Setup:");
    println!("cargo run --bin cam_rtsp \\");
    println!("    --rtsp-server-addr 139.162.13.249:8554 \\");
    println!("    --stream-url rtsp://139.162.13.249:8554/mystream \\");
    println!("    --enable-forwarding \\");
    println!("    --forward-target-url rtsp://server.example.com:8554/lowbw \\");
    println!("    --enable-h265 \\");
    println!("    --h265-preset ultrafast \\");
    println!("    --h265-crf 30 \\");
    println!("    --frame-rate 10.0 \\");
    println!("    --force-frame-rate");

    println!("\n🔹 High Quality Setup:");
    println!("cargo run --bin cam_rtsp \\");
    println!("    --rtsp-server-addr 139.162.13.249:8554 \\");
    println!("    --stream-url rtsp://139.162.13.249:8554/mystream \\");
    println!("    --enable-forwarding \\");
    println!("    --forward-target-url rtsp://server.example.com:8554/highq \\");
    println!("    --enable-h265 \\");
    println!("    --h265-preset fast \\");
    println!("    --h265-crf 22 \\");
    println!("    --frame-rate 30.0");

    println!("\n🎉 CLI features successfully implemented!");
    println!("💡 You can now control H.265 encoding and frame rate from command line");
    println!("💡 All configurations are properly logged at startup");
    println!("💡 Native encoding is used when H.265 is disabled (default)");
}