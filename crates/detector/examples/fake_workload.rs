use std::{
    sync::Arc,
    time::{Instant, SystemTime},
};
use surveillance_common::types::CameraId;
use surveillance_detector::inference::{DeterministicFakeInference, PersonInference};
use surveillance_media::{
    frame::{DecodedFrame, FrameLimits, PixelFormat},
    packet::MediaTimestamp,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    const ITERATIONS: usize = 10_000;
    let limits = FrameLimits::new(640, 360, 640 * 360 * 3)?;
    let frame = DecodedFrame::new(
        CameraId::new("benchmark-camera")?,
        1,
        MediaTimestamp::new(90_000, 90_000)?,
        SystemTime::now(),
        640,
        360,
        640 * 3,
        PixelFormat::Rgb24,
        Arc::from(vec![1_u8; 640 * 360 * 3]),
        limits,
    )?;
    let mut inference = DeterministicFakeInference::new()?;
    let mut samples = Vec::with_capacity(ITERATIONS);
    let workload_started = Instant::now();
    for _ in 0..ITERATIONS {
        let started = Instant::now();
        let result = inference.infer(&frame)?;
        if result.as_slice().len() != 1 {
            return Err("unexpected deterministic result".into());
        }
        samples.push(u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX));
    }
    samples.sort_unstable();
    let percentile = |percent: usize| samples[(ITERATIONS * percent / 100).min(ITERATIONS - 1)];
    println!("{{\"workload\":\"deterministic-fake-person-1.0.0-rgb640x360\",\"iterations\":{ITERATIONS},\"elapsed_micros\":{},\"p50_nanos\":{},\"p90_nanos\":{},\"p99_nanos\":{}}}", workload_started.elapsed().as_micros(), percentile(50), percentile(90), percentile(99));
    Ok(())
}
