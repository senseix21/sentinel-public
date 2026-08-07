use crate::stream_processor::StreamProcessor;
use crate::stream_detector::StreamDetector;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    println!("🎥 Intelligent Stream Forwarding Demo");
    println!("=====================================");

    let source_stream = "rtsp://139.162.13.249:8554/mystream";
    let target_url = "rtsp://127.0.0.1:8554/output";

    // Create components
    let detector = StreamDetector::new();
    let mut processor = StreamProcessor::with_h265_support(true);

    println!("\n🔍 Step 1: Detecting stream encoding...");

    // Step 1: Detect stream encoding
    let strategy = processor.detect_and_configure_forwarding(
        source_stream.to_string(),
        target_url.to_string(),
        &detector,
    ).await?;

    println!("\n📊 Detection Results:");
    println!("   ├── Original Codec: {}", strategy.original_codec);
    println!("   ├── Strategy: {:?}", strategy.strategy);

    match strategy.strategy {
        crate::stream_processor::StrategyType::H265Reencoding => {
            println!("   ├── Action: Re-encoding H.264 → H.265");
            if let Some(ref config) = strategy.h265_config {
                println!("   ├── H.265 Preset: {}", config.preset);
                println!("   ├── H.265 CRF: {}", config.crf);
                if config.frame_rate.is_some() {
                    println!("   ├── Frame Rate: {} FPS", config.frame_rate.unwrap());
                }
            }
        }
        crate::stream_processor::StrategyType::NativeForwarding => {
            println!("   ├── Action: Native forwarding (no re-encoding)");
        }
    }

    println!("\n🚀 Step 2: Starting forwarder...");

    // Step 2: Start forwarding based on detected strategy
    match processor.start_forwarding(source_stream.to_string(), target_url.to_string()).await {
        Ok(_) => {
            println!("✅ Forwarding started successfully");

            // Wait a bit to demonstrate
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;

            // Step 3: Show current strategy
            println!("\n📋 Step 3: Current Forwarding Status:");
            if let Some(current_strategy) = processor.get_current_strategy() {
                println!("   ├── Active: {}", if processor.is_forwarding() { "Yes" } else { "No" });
                println!("   ├── Source: {}", current_strategy.source_stream);
                println!("   ├── Target: {}", current_strategy.target_url);
                println!("   ├── Strategy: {:?}", current_strategy.strategy);
                println!("   ├── Original Codec: {}", current_strategy.original_codec);

                match current_strategy.strategy {
                    crate::stream_processor::StrategyType::H265Reencoding => {
                        println!("   🔄 H.265 re-encoding active");
                    }
                    crate::stream_processor::StrategyType::NativeForwarding => {
                        println!("   ➡️ Native forwarding active");
                    }
                }
            }

            // Step 4: Stop forwarding
            println!("\n🛑 Step 4: Stopping forwarder...");
            processor.stop_forwarding().await?;
            println!("✅ Forwarding stopped");
        }
        Err(e) => {
            println!("❌ Failed to start forwarding: {}", e);
            println!("💡 This might be expected if no RTSP server is listening on the target");
        }
    }

    println!("\n🎯 Example Scenarios:");

    println!("\n1. H.264 Source Stream:");
    println!("   📡 Input:  rtsp://camera:8554/stream (H.264)");
    println!("   🎯 Output: rtsp://server:8554/stream");
    println!("   🔄 Action: Detect H.264 → Re-encode to H.265");
    println!("   📊 Result: Bandwidth reduced, quality maintained");

    println!("\n2. H.265 Source Stream:");
    println!("   📡 Input:  rtsp://camera:8554/stream (H.265)");
    println!("   🎯 Output: rtsp://server:8554/stream");
    println!("   🔄 Action: Detect H.265 → Forward natively");
    println!("   📊 Result: No re-encoding, minimal CPU usage");

    println!("\n3. Unknown Source Stream:");
    println!("   📡 Input:  rtsp://camera:8554/stream (VP8)");
    println!("   🎯 Output: rtsp://server:8554/stream");
    println!("   🔄 Action: Detect VP8 → Forward natively");
    println!("   📊 Result: Native forwarding with warning");

    println!("\n💡 Benefits of Intelligent Forwarding:");
    println!("   ✅ Automatic codec detection");
    println!("   ✅ Conditional re-encoding only when beneficial");
    println!("   ✅ Optimized bandwidth usage for H.264 sources");
    println!("   ✅ Native forwarding for H.265 sources (saves CPU)");
    println!("   ✅ Graceful handling of unknown codecs");
    println!("   ✅ Configurable H.265 settings via CLI");

    Ok(())
}