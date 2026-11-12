//! External pinned-model qualification, not a production accuracy claim.
#![cfg(feature = "onnx-inference")]
use std::{
    path::PathBuf,
    time::{Instant, SystemTime},
};
use surveillance_common::types::CameraId;
use surveillance_detector::{
    backend::InferenceBackendConfig,
    face::OnnxFaceInference,
    inference::{DetectionInference, InferenceResults},
    letterbox::LetterboxAlignment,
};
use surveillance_media::{
    frame::{DecodedFrame, FrameLimits, PixelFormat},
    packet::MediaTimestamp,
};
#[test]
#[ignore = "requires a pinned YuNet artifact and raw RGB fixture"]
fn real_face_model_localizes_faces_and_clears_on_blank() {
    let env = |name| std::env::var(name).unwrap_or_else(|_| panic!("{name} required"));
    let config = InferenceBackendConfig {
        model_path: Some(PathBuf::from(env("FACE_MODEL_PATH"))),
        model_sha256: Some(env("FACE_MODEL_SHA256")),
        source_width: env("FACE_SOURCE_WIDTH").parse().unwrap(),
        source_height: env("FACE_SOURCE_HEIGHT").parse().unwrap(),
        input_width: 640,
        input_height: 640,
        letterbox_alignment: LetterboxAlignment::TopLeft,
        execution_providers: vec!["cpu".to_owned()],
        intra_op_threads: 1,
        inter_op_threads: 1,
        score_threshold_percent: 70,
        nms_iou_percent: 45,
    };
    let mut model = OnnxFaceInference::load(&config).unwrap();
    let frame = |pixels: Vec<u8>| {
        DecodedFrame::new(
            CameraId::new("face-test".to_owned()).unwrap(),
            1,
            MediaTimestamp::new(0, 90000).unwrap(),
            SystemTime::UNIX_EPOCH,
            640,
            640,
            1920,
            PixelFormat::Rgb24,
            pixels.into(),
            FrameLimits::new(640, 640, 640 * 640 * 3).unwrap(),
        )
        .unwrap()
    };
    let input = frame(std::fs::read(env("FACE_FRAME_PATH")).unwrap());
    let mut durations = Vec::new();
    for _ in 0..30 {
        let started = Instant::now();
        let result = model.infer_detections(&input).unwrap();
        durations.push(started.elapsed().as_micros());
        let InferenceResults::Faces(set) = result else {
            panic!("face model must not emit people")
        };
        let minimum = std::env::var("FACE_MINIMUM_DETECTIONS")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(1);
        assert!(
            set.as_slice().len() >= minimum,
            "positive fixture must detect its visible faces"
        );
        if let Ok(rect) = std::env::var("FACE_EXPECTED_BOX") {
            let expected: Vec<f32> = rect.split(',').map(|v| v.parse().unwrap()).collect();
            assert_eq!(expected.len(), 4);
            let best = set
                .as_slice()
                .iter()
                .map(|face| {
                    let b = face.bounding_box;
                    let w = ((b.left() + b.width()).min(expected[0] + expected[2])
                        - b.left().max(expected[0]))
                    .max(0.0);
                    let h = ((b.top() + b.height()).min(expected[1] + expected[3])
                        - b.top().max(expected[1]))
                    .max(0.0);
                    let intersection = w * h;
                    intersection
                        / (b.width() * b.height() + expected[2] * expected[3] - intersection)
                })
                .fold(0.0_f32, f32::max);
            assert!(
                best >= 0.5,
                "face bounds must overlap manual ground truth, IoU={best}"
            );
        }
        if durations.len() == 1 {
            for d in set.as_slice() {
                println!(
                    "face confidence={} box={:?}",
                    d.confidence.get(),
                    d.bounding_box
                );
            }
        }
    }
    durations.sort_unstable();
    println!(
        "CPU inference microseconds p50={} p90={} p99={}",
        durations[15], durations[27], durations[29]
    );
    let InferenceResults::Faces(blank) = model
        .infer_detections(&frame(vec![0; 640 * 640 * 3]))
        .unwrap()
    else {
        panic!("wrong kind")
    };
    assert!(blank.as_slice().is_empty(), "blank input must clear boxes");
    let wrong = InferenceBackendConfig {
        input_width: 416,
        ..config
    };
    assert!(OnnxFaceInference::load(&wrong).is_err());
}
