use chrono::{DateTime, Utc};
use std::path::Path;

fn parse_recording_filename(filename: &str) -> Option<DateTime<Utc>> {
    if filename.starts_with("stream_") && filename.ends_with(".mkv") {
        let name_part = &filename[7..filename.len() - 4]; // Remove "stream_" and ".mkv"
        if let Ok(timestamp) = chrono::NaiveDateTime::parse_from_str(name_part, "%Y%m%d_%H%M%S") {
            return Some(DateTime::from_naive_utc_and_offset(timestamp, Utc));
        }
    }
    None
}

fn main() {
    println!("Testing recording filename parsing:");

    // Test with actual filenames from the recordings directory
    let test_filenames = vec![
        "stream_20251117_185932.mkv",
        "stream_20251117_191257.mkv",
        "stream_20251117_192333.mkv",
        "stream_20251117_193845.mkv",
        "stream_20251117_220109.mkv",
        "stream_20251118_154310.mkv",
        "stream_20251118_155014.mkv",
        "stream_20251118_155915.mkv",
        "stream_20251118_160351.mkv",
    ];

    let mut valid_recordings = 0;

    for filename in test_filenames {
        if let Some(timestamp) = parse_recording_filename(filename) {
            valid_recordings += 1;
            println!("✅ {}: {}", filename, timestamp);
        } else {
            println!("❌ {}: Failed to parse", filename);
        }
    }

    println!("\nSummary: {} out of {} recordings can be parsed", valid_recordings, test_filenames.len());

    // Test time range filtering
    if let Some(start_time) = parse_recording_filename("stream_20251117_185932.mkv") {
        if let Some(end_time) = parse_recording_filename("stream_20251118_155915.mkv") {
            println!("\nTesting time range: {} to {}", start_time, end_time);
            println!("Duration: {} hours", (end_time - start_time).num_hours());
        }
    }
}