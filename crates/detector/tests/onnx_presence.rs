//! Real dual-model canonical frame qualification, independent of camera hardware.
//! Back-facing fixture uses the left 160x375 crop of COCO 000000000077:
//! its other subjects include a visible profile face and are outside this test.
#![cfg(feature = "onnx-inference")]
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime},
};
use surveillance_common::types::CameraId;
use surveillance_detector::{
    backend::{InferenceBackendConfig, ModeInferenceBackend},
    combined::resize,
    inference::{DetectionInference, DetectionMode, InferenceResults},
    letterbox::LetterboxAlignment,
    presence::{PresencePolicy, PresenceTracker},
};
use surveillance_media::{
    frame::{DecodedFrame, FrameLimits, PixelFormat},
    packet::MediaTimestamp,
};
#[test]
#[ignore = "requires pinned YuNet/YOLOX artifacts and canonical RGB fixture"]
#[allow(
    clippy::too_many_lines,
    reason = "external qualification keeps model setup and evidence assertions together"
)]
fn real_person_evidence_survives_unobservable_face_and_stationary_samples() {
    let env = |name| std::env::var(name).unwrap_or_else(|_| panic!("{name} required"));
    let mut cfg = InferenceBackendConfig {
        model_path: Some(PathBuf::from(env("PERSON_MODEL_PATH"))),
        model_sha256: Some(env("PERSON_MODEL_SHA256")),
        source_width: 810,
        source_height: 1080,
        input_width: 416,
        input_height: 416,
        letterbox_alignment: LetterboxAlignment::TopLeft,
        execution_providers: vec!["cpu".to_owned()],
        intra_op_threads: 1,
        inter_op_threads: 1,
        score_threshold_percent: 70,
        nms_iou_percent: 45,
    };
    let mut person = ModeInferenceBackend::select(&cfg, DetectionMode::Person).unwrap();
    cfg.model_path = Some(PathBuf::from(env("FACE_MODEL_PATH")));
    cfg.model_sha256 = Some(env("FACE_MODEL_SHA256"));
    cfg.input_width = 640;
    cfg.input_height = 640;
    let mut face = ModeInferenceBackend::select(&cfg, DetectionMode::Face).unwrap();
    let pixels = std::fs::read(env("PRESENCE_FRAME_PATH")).unwrap();
    let frame = |pixels: Vec<u8>| {
        DecodedFrame::new(
            CameraId::new("public-fixture").unwrap(),
            1,
            MediaTimestamp::new(0, 90000).unwrap(),
            SystemTime::UNIX_EPOCH,
            640,
            640,
            1920,
            PixelFormat::Rgb24,
            Arc::from(pixels),
            FrameLimits::new(640, 640, 640 * 640 * 3).unwrap(),
        )
        .unwrap()
    };
    let input = frame(pixels.clone());
    let InferenceResults::Persons(initial) = person
        .infer_detections(&resize(&input, 416, 416).unwrap())
        .unwrap()
    else {
        panic!("person contract")
    };
    assert!(
        initial.as_slice().len() >= 2,
        "full-body fixture needs at least two person detections at 70%"
    );
    let InferenceResults::Faces(initial_faces) = face.infer_detections(&input).unwrap() else {
        panic!("face contract")
    };
    assert!(!initial_faces.as_slice().is_empty());
    // Remove visible face pixels while preserving bodies. This is an occlusion
    // contract check, not a real back-facing dataset or a pose-accuracy claim.
    let mut masked = pixels;
    for y in 210..295 {
        for x in 25..205 {
            masked[(y * 640 + x) * 3..(y * 640 + x) * 3 + 3].fill(0);
        }
    }
    let obscured = frame(masked);
    let InferenceResults::Persons(after) = person
        .infer_detections(&resize(&obscured, 416, 416).unwrap())
        .unwrap()
    else {
        panic!("person contract")
    };
    let InferenceResults::Faces(after_faces) = face.infer_detections(&obscured).unwrap() else {
        panic!("face contract")
    };
    println!(
        "real fixture: before people={} faces={}, occluded people={} faces={}",
        initial.as_slice().len(),
        initial_faces.as_slice().len(),
        after.as_slice().len(),
        after_faces.as_slice().len()
    );
    assert!(after.as_slice().len() >= 2);
    assert!(after_faces.as_slice().is_empty());
    let policy = PresencePolicy::new(
        Duration::from_secs(1),
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .unwrap();
    let mut tracker = PresenceTracker::new(policy);
    tracker.observe(&initial, Duration::ZERO, true).unwrap();
    for i in 1..6 {
        tracker
            .observe(&after, Duration::from_millis(i * 200), true)
            .unwrap();
        assert!(
            tracker
                .snapshot(Duration::from_millis(i * 200), true)
                .counts()
                .0
                >= 2
        );
    }
    let blank = frame(vec![0; 640 * 640 * 3]);
    let InferenceResults::Persons(empty) = person
        .infer_detections(&resize(&blank, 416, 416).unwrap())
        .unwrap()
    else {
        panic!("person contract")
    };
    assert!(empty.as_slice().is_empty());
}

#[test]
#[ignore = "requires pinned models and COCO 000000000077 canonical RGB"]
fn real_back_facing_people_are_detected_without_prior_face_evidence() {
    let env = |name| std::env::var(name).unwrap_or_else(|_| panic!("{name} required"));
    let mut cfg = InferenceBackendConfig {
        model_path: Some(PathBuf::from(env("PERSON_MODEL_PATH"))),
        model_sha256: Some(env("PERSON_MODEL_SHA256")),
        source_width: 160,
        source_height: 375,
        input_width: 416,
        input_height: 416,
        letterbox_alignment: LetterboxAlignment::TopLeft,
        execution_providers: vec!["cpu".to_owned()],
        intra_op_threads: 1,
        inter_op_threads: 1,
        score_threshold_percent: 70,
        nms_iou_percent: 45,
    };
    let mut person = ModeInferenceBackend::select(&cfg, DetectionMode::Person).unwrap();
    cfg.model_path = Some(PathBuf::from(env("FACE_MODEL_PATH")));
    cfg.model_sha256 = Some(env("FACE_MODEL_SHA256"));
    cfg.input_width = 640;
    cfg.input_height = 640;
    let mut face = ModeInferenceBackend::select(&cfg, DetectionMode::Face).unwrap();
    let frame = DecodedFrame::new(
        CameraId::new("back-fixture").unwrap(),
        1,
        MediaTimestamp::new(0, 90000).unwrap(),
        SystemTime::UNIX_EPOCH,
        640,
        640,
        1920,
        PixelFormat::Rgb24,
        Arc::from(std::fs::read(env("BACK_FRAME_PATH")).unwrap()),
        FrameLimits::new(640, 640, 640 * 640 * 3).unwrap(),
    )
    .unwrap();
    let InferenceResults::Persons(people) = person
        .infer_detections(&resize(&frame, 416, 416).unwrap())
        .unwrap()
    else {
        panic!("person contract")
    };
    let InferenceResults::Faces(faces) = face.infer_detections(&frame).unwrap() else {
        panic!("face contract")
    };
    println!(
        "actual back-facing fixture: people={}, faces={}",
        people.as_slice().len(),
        faces.as_slice().len()
    );
    for detection in faces.as_slice() {
        println!("back-fixture face {detection:?}");
    }
    assert!(
        !people.as_slice().is_empty(),
        "back-facing arrival needs person evidence"
    );
    assert!(faces.as_slice().is_empty(), "no visible face expected");
    let mut tracker = PresenceTracker::new(
        PresencePolicy::new(
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .unwrap(),
    );
    for i in 0..10 {
        tracker
            .observe(&people, Duration::from_millis(i * 200), true)
            .unwrap();
        if i > 0 {
            assert!(
                tracker
                    .snapshot(Duration::from_millis(i * 200), true)
                    .counts()
                    .0
                    > 0
            );
        }
    }
}
