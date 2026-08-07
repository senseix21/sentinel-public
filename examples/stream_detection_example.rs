mod stream_detector;

use stream_detector::StreamDetector;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let detector = StreamDetector::new();
    let rtsp_url = "rtsp://139.162.13.249:8554/mystream";

    println!("🔍 Detecting stream encoding for: {}", rtsp_url);

    match detector.auto_detect(rtsp_url).await {
        Ok(stream_info) => {
            println!("✅ Stream detected successfully!");
            println!("📊 Stream Information:");
            println!("   ├── Codec: {}", stream_info.codec_name);
            println!("   ├── Type: {}", stream_info.codec_type);
            println!("   ├── Resolution: {}x{}",
                stream_info.width.unwrap_or(0),
                stream_info.height.unwrap_or(0)
            );

            if let Some(frame_rate) = &stream_info.frame_rate {
                println!("   ├── Frame Rate: {}", frame_rate);
            }

            if let Some(bit_rate) = &stream_info.bit_rate {
                println!("   ├── Bit Rate: {} bps", bit_rate);
            }

            println!("   ├── Detection Method: {}", stream_info.method_used);

            // Suggest encoding based on codec
            match stream_info.codec_name.as_str() {
                "h264" => println!("💡 Recommendation: H.264 - Good compatibility, moderate compression"),
                "hevc" | "h265" => println!("💡 Recommendation: H.265 - Excellent compression, lower bandwidth"),
                "vp8" => println!("💡 Recommendation: VP8 - Good compression, open source"),
                "vp9" => println!("💡 Recommendation: VP9 - Better compression than VP8"),
                "av1" => println!("💡 Recommendation: AV1 - Best compression, higher CPU usage"),
                _ => println!("💡 Recommendation: Unknown codec, check compatibility"),
            }
        }
        Err(e) => {
            println!("❌ Failed to detect stream encoding: {}", e);
            println!("💡 Troubleshooting tips:");
            println!("   ├── Check if the RTSP stream is accessible");
            println!("   ├── Verify ffprobe/ffmpeg is installed");
            println!("   ├── Ensure the RTSP URL is correct");
            println!("   ├── Check network connectivity");
        }
    }

    Ok(())
}