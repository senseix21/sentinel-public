use std::process::Command;

fn main() {
    let file_path = "/Volumes/thezone/Documents/Rust/recordings/stream_20251118_171348.mkv";

    // Test alternative ffprobe syntax
    println!("Testing alternative ffprobe syntax...");

    let output = Command::new("ffprobe")
        .args(&["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", file_path])
        .output()
        .expect("Failed to execute ffprobe");

    if output.status.success() {
        let duration_str = String::from_utf8_lossy(&output.stdout);
        let duration = duration_str.trim();
        println!("✅ Duration: {} seconds", duration);
    } else {
        let error_str = String::from_utf8_lossy(&output.stderr);
        println!("❌ Failed: {}", error_str);
    }
}