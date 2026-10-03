//! DI.R8: the whole adapter against a real model and a real frame.
//!
//! Ignored by default and gated on the ONNX backend, matching how every other
//! test needing an external artifact is handled here. It proves the adapter
//! works end to end. It is **not** evidence of detection accuracy: one frame is
//! an anecdote, and Phase 6's accuracy item needs a labeled dataset and agreed
//! criteria that do not exist.
//!
//! Supply the inputs through the environment, as the other external-artifact
//! harnesses do:
//!
//! ```text
//! DETECTION_MODEL_PATH=/path/to/yolox_tiny.onnx \
//! DETECTION_FRAME_PATH=/path/to/frame.rgb \
//! DETECTION_FRAME_WIDTH=416 DETECTION_FRAME_HEIGHT=416 \
//! DETECTION_SOURCE_WIDTH=1280 DETECTION_SOURCE_HEIGHT=720 \
//! cargo test -p surveillance-detector --features onnx-inference \
//!   --test onnx_real_model -- --ignored
//! ```
//!
//! `DETECTION_MINIMUM_DETECTIONS` is how many the frame is known to contain,
//! and defaults to one. Without it this test passes on a model that detects
//! nothing at all: the per-box assertions below hold vacuously over an empty
//! set, so silence would read as success.
//!
//! The frame is raw packed RGB24 at the model's input geometry, which is what
//! the detector's decoder now produces. Create one with:
//!
//! ```text
//! ffmpeg -i frame.png -vf \
//!   "scale=416:416:force_original_aspect_ratio=decrease,\
//!    pad=416:416:(ow-iw)/2:(oh-ih)/2:color=0x727272" \
//!   -pix_fmt rgb24 -f rawvideo frame.rgb
//! ```

#![cfg(feature = "onnx-inference")]

use std::time::Instant;

use surveillance_common::types::CameraId;
use surveillance_detector::{
    backend::{InferenceBackendConfig, PersonInferenceBackend},
    inference::PersonInference,
};
use surveillance_media::frame::{DecodedFrame, FrameLimits, PixelFormat};
use surveillance_media::packet::MediaTimestamp;

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is required for this test"))
}

fn dimension(name: &str) -> u32 {
    required(name)
        .parse()
        .unwrap_or_else(|error| panic!("{name} must be a number: {error}"))
}

#[test]
#[ignore = "requires a model artifact and a prepared frame"]
fn a_real_model_produces_detections_from_a_real_frame() {
    let input_width = dimension("DETECTION_FRAME_WIDTH");
    let input_height = dimension("DETECTION_FRAME_HEIGHT");
    let config = InferenceBackendConfig {
        model_path: Some(required("DETECTION_MODEL_PATH").into()),
        // An empty value is no pin, not a pin that nothing can match: the
        // Makefile target passes the variable through whether or not it is set.
        model_sha256: std::env::var("DETECTION_MODEL_SHA256")
            .ok()
            .filter(|value| !value.is_empty()),
        source_width: dimension("DETECTION_SOURCE_WIDTH"),
        source_height: dimension("DETECTION_SOURCE_HEIGHT"),
        input_width,
        input_height,
        execution_providers: vec!["cpu".to_owned()],
        intra_op_threads: 1,
        inter_op_threads: 1,
        score_threshold_percent: 30,
        nms_iou_percent: 45,
    };

    let loaded = Instant::now();
    let mut backend = PersonInferenceBackend::select(&config)
        .unwrap_or_else(|error| panic!("the backend must load: {error}"));
    let load_duration = loaded.elapsed();
    assert!(
        backend.is_production_model(),
        "a loaded model must report itself as production, or the API lies about it"
    );

    let pixels = std::fs::read(required("DETECTION_FRAME_PATH"))
        .unwrap_or_else(|error| panic!("read the prepared frame: {error}"));
    let stride = input_width as usize * 3;
    assert_eq!(
        pixels.len(),
        stride * input_height as usize,
        "the frame must be raw RGB24 at the model's input geometry"
    );
    let frame = DecodedFrame::new(
        CameraId::new("camera-01".to_owned()).unwrap_or_else(|error| panic!("camera: {error}")),
        1,
        MediaTimestamp::new(0, 90_000).unwrap_or_else(|error| panic!("timestamp: {error}")),
        std::time::SystemTime::UNIX_EPOCH,
        input_width,
        input_height,
        stride,
        PixelFormat::Rgb24,
        pixels.into(),
        FrameLimits::new(input_width, input_height, stride * input_height as usize)
            .unwrap_or_else(|error| panic!("limits: {error}")),
    )
    .unwrap_or_else(|error| panic!("frame: {error}"));

    let started = Instant::now();
    let detections = backend
        .infer(&frame)
        .unwrap_or_else(|error| panic!("inference must succeed: {error}"));
    let inference_duration = started.elapsed();

    // The frame is supplied because it holds people. A model that loads, runs,
    // and finds none of them has failed in the way that matters, and every
    // assertion that follows is satisfied by an empty set.
    let minimum: usize = std::env::var("DETECTION_MINIMUM_DETECTIONS")
        .map_or(Ok(1), |value| value.parse())
        .unwrap_or_else(|error| panic!("DETECTION_MINIMUM_DETECTIONS must be a number: {error}"));
    assert!(
        detections.as_slice().len() >= minimum,
        "expected at least {minimum} detection(s), got {}: the adapter ran and found nothing",
        detections.as_slice().len()
    );

    // Every box must be inside the frame. A wrong inverse letterbox produces
    // boxes that look plausible and are consistently displaced, so this is the
    // assertion that catches it on real output.
    for detection in detections.as_slice() {
        let geometry = detection.bounding_box;
        assert!(geometry.left() >= 0.0 && geometry.top() >= 0.0);
        assert!(geometry.left() + geometry.width() <= 1.0 + 1e-6);
        assert!(geometry.top() + geometry.height() <= 1.0 + 1e-6);
    }

    // Evidence, not an assertion about accuracy.
    println!(
        "load {:?}, inference {:?}, {} detection(s)",
        load_duration,
        inference_duration,
        detections.as_slice().len()
    );
    for detection in detections.as_slice() {
        let geometry = detection.bounding_box;
        println!(
            "  confidence {:.3} at ({:.3},{:.3}) {:.3}x{:.3} of the source frame",
            f32::from(detection.confidence),
            geometry.left(),
            geometry.top(),
            geometry.width(),
            geometry.height()
        );
    }
}
