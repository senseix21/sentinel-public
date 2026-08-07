use std::path::PathBuf;
use chrono::{DateTime, Utc};

fn main() {
    // Test the exact algorithm we want to implement
    let request_start_time = "2025-11-18T17:13:48Z";
    let request_end_time = "2025-11-18T17:15:48Z";

    // Parse the start time
    let start_time = request_start_time.parse::<chrono::DateTime<chrono::Utc>>()
        .expect("Failed to parse start time");

    println!("Request start time: {}", start_time);

    // Convert to filename format
    let filename = format!("stream_{}.mkv", start_time.format("%Y%m%d_%H%M%S"));
    println!("Looking for file: {}", filename);

    // Check if file exists
    let recordings_dir = PathBuf::from("/Volumes/thezone/Documents/Rust/recordings");
    let file_path = recordings_dir.join(&filename);

    println!("Full path: {}", file_path.display());

    if file_path.exists() {
        println!("✅ File found!");

        // Try to get file duration using ffprobe
        let output = std::process::Command::new("ffprobe")
            .args(&["-v", "error", "-show_format", "-show_streams", &file_path.display().to_string()])
            .output()
            .expect("Failed to execute ffprobe");

        if output.status.success() {
            let output_str = String::from_utf8_lossy(&output.stdout);
            println!("ffprobe output:");
            for line in output_str.lines() {
                if line.starts_with("duration=") {
                    let duration = &line[9..]; // Remove "duration="
                    println!("Duration: {} seconds", duration);
                    let duration_f64 = duration.parse::<f64>().unwrap();
                    println!("Duration in seconds: {:.3}", duration_f64);
                }
            }
        } else {
            println!("ffprobe failed: {}", String::from_utf8_lossy(&output.stderr));
        }
    } else {
        println!("❌ File not found");
    }
}