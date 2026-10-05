//! DI.R10: the adapter against many frames with real ground truth.
//!
//! `DI.R8` proves the adapter works on one frame. One frame cannot distinguish
//! a detector that works from one that happened to work there, and says nothing
//! about the rate at which people are missed — which for a surveillance product
//! is the number that matters, because a missed person is a missed event.
//!
//! This is **not** Phase 6's accuracy gate. That gate waits on a dataset and
//! criteria agreed to represent this product's cameras; COCO is a benchmark of
//! general photographs, not of surveillance footage, and nothing here proposes
//! it as a substitute. What this measures is the adapter's behaviour on a
//! stated public subset, which is worth having because anyone who disputes the
//! number can regenerate it.
//!
//! Build the set with `scripts/detection/build-eval-set.py` and run it with
//! `make container-verify-accuracy`. Both minimum thresholds are required: a
//! measurement that asserts nothing is a report, and reports do not fail.
//!
//! Percentages are integers throughout, as the settings domain's fractions are.

#![cfg(feature = "onnx-inference")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use surveillance_common::types::CameraId;
use surveillance_detector::{
    backend::{InferenceBackendConfig, PersonInferenceBackend},
    inference::PersonInference,
    letterbox::LetterboxAlignment,
};
use surveillance_media::frame::{DecodedFrame, FrameLimits, PixelFormat};
use surveillance_media::packet::MediaTimestamp;

#[derive(Deserialize)]
struct Manifest {
    input_width: u32,
    input_height: u32,
    /// How the generator placed the padding. Read rather than assumed: the
    /// point of this field is to let the two placements be compared, and an
    /// inverse disagreeing with the frames displaces every box.
    alignment: String,
    frames: Vec<ManifestFrame>,
}

#[derive(Deserialize)]
struct ManifestFrame {
    path: String,
    source_width: u32,
    source_height: u32,
    /// Ground truth as COCO states it: source pixels, origin top-left,
    /// `[x, y, width, height]`.
    persons: Vec<[f32; 4]>,
}

/// A box in normalized source coordinates, which is what both sides speak once
/// ground truth is divided by its own frame.
#[derive(Clone, Copy, Debug)]
struct Box {
    left: f32,
    top: f32,
    width: f32,
    height: f32,
}

impl Box {
    fn area(self) -> f32 {
        self.width * self.height
    }

    fn intersection_over_union(self, other: Self) -> f32 {
        let width = ((self.left + self.width).min(other.left + other.width)
            - self.left.max(other.left))
        .max(0.0);
        let height = ((self.top + self.height).min(other.top + other.height)
            - self.top.max(other.top))
        .max(0.0);
        let overlap = width * height;
        let union = self.area() + other.area() - overlap;
        if union <= 0.0 {
            0.0
        } else {
            overlap / union
        }
    }
}

#[derive(Default)]
struct Tally {
    found: usize,
    total: usize,
}

#[derive(Default)]
struct Outcome {
    true_positives: usize,
    false_positives: usize,
    ground_truth: usize,
    buckets: BTreeMap<&'static str, Tally>,
    sessions: usize,
}

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is required for this test"))
}

fn percentage(name: &str) -> usize {
    required(name)
        .parse()
        .unwrap_or_else(|error| panic!("{name} must be a whole percentage: {error}"))
}

/// Tenths of a percent, computed in integers so a rate is exact and no
/// comparison depends on a float.
fn tenths(part: usize, whole: usize) -> usize {
    if whole == 0 {
        0
    } else {
        part * 1000 / whole
    }
}

fn show(part: usize, whole: usize) -> String {
    let value = tenths(part, whole);
    format!("{}.{}%", value / 10, value % 10)
}

/// COCO's size convention, in source pixels of area, so a recall figure can be
/// read against the thing that drives it.
fn bucket(width: f32, height: f32) -> &'static str {
    let area = width * height;
    if area < 32.0 * 32.0 {
        "small"
    } else if area < 96.0 * 96.0 {
        "medium"
    } else {
        "large"
    }
}

fn dimension(value: u32) -> f32 {
    f32::from(u16::try_from(value).unwrap_or(u16::MAX))
}

fn alignment_of(manifest: &Manifest) -> LetterboxAlignment {
    match manifest.alignment.as_str() {
        "centre" => LetterboxAlignment::Centred,
        "topleft" => LetterboxAlignment::TopLeft,
        other => panic!("unknown manifest alignment {other}"),
    }
}

fn load_frame(path: &Path, manifest: &Manifest) -> DecodedFrame {
    let pixels =
        std::fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let stride = manifest.input_width as usize * 3;
    assert_eq!(
        pixels.len(),
        stride * manifest.input_height as usize,
        "{} is not raw RGB24 at {}x{}",
        path.display(),
        manifest.input_width,
        manifest.input_height
    );
    DecodedFrame::new(
        CameraId::new("camera-01".to_owned()).unwrap_or_else(|error| panic!("camera: {error}")),
        1,
        MediaTimestamp::new(0, 90_000).unwrap_or_else(|error| panic!("timestamp: {error}")),
        std::time::SystemTime::UNIX_EPOCH,
        manifest.input_width,
        manifest.input_height,
        stride,
        PixelFormat::Rgb24,
        pixels.into(),
        FrameLimits::new(
            manifest.input_width,
            manifest.input_height,
            stride * manifest.input_height as usize,
        )
        .unwrap_or_else(|error| panic!("limits: {error}")),
    )
    .unwrap_or_else(|error| panic!("frame: {error}"))
}

fn backend_for(model: &Path, manifest: &Manifest, frame: &ManifestFrame) -> PersonInferenceBackend {
    let config = InferenceBackendConfig {
        model_path: Some(model.to_path_buf()),
        model_sha256: None,
        source_width: frame.source_width,
        source_height: frame.source_height,
        input_width: manifest.input_width,
        input_height: manifest.input_height,
        letterbox_alignment: alignment_of(manifest),
        execution_providers: vec!["cpu".to_owned()],
        intra_op_threads: 1,
        inter_op_threads: 1,
        score_threshold_percent: u8::try_from(std::env::var("DETECTION_SCORE_PERCENT").map_or(
            30,
            |value| {
                value
                    .parse()
                    .unwrap_or_else(|error| panic!("DETECTION_SCORE_PERCENT: {error}"))
            },
        ))
        .unwrap_or(u8::MAX),
        nms_iou_percent: 45,
    };
    PersonInferenceBackend::select(&config)
        .unwrap_or_else(|error| panic!("the backend must load: {error}"))
}

fn truth_of(frame: &ManifestFrame) -> Vec<Box> {
    let width = dimension(frame.source_width);
    let height = dimension(frame.source_height);
    frame
        .persons
        .iter()
        .map(|[x, y, box_width, box_height]| Box {
            left: x / width,
            top: y / height,
            width: box_width / width,
            height: box_height / height,
        })
        .collect()
}

/// Greedy one-to-one assignment in confidence order, which is what a consumer
/// of these events would do: the most confident box claims the person it
/// overlaps, and a second box over the same person is a false positive rather
/// than a second hit.
fn assign(predictions: &[(f32, Box)], truth: &[Box], minimum_iou: f32) -> (Vec<bool>, usize) {
    let mut claimed = vec![false; truth.len()];
    let mut unmatched = 0;
    for (_, predicted) in predictions {
        let best = truth
            .iter()
            .enumerate()
            .filter(|(index, _)| !claimed[*index])
            .map(|(index, actual)| (index, predicted.intersection_over_union(*actual)))
            .max_by(|left, right| {
                left.1
                    .partial_cmp(&right.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        match best {
            Some((index, overlap)) if overlap >= minimum_iou => claimed[index] = true,
            _ => unmatched += 1,
        }
    }
    (claimed, unmatched)
}

fn predictions_of(detections: &surveillance_events::event::DetectionSet) -> Vec<(f32, Box)> {
    let mut predictions: Vec<(f32, Box)> = detections
        .as_slice()
        .iter()
        .map(|detection| {
            let geometry = detection.bounding_box;
            (
                f32::from(detection.confidence),
                Box {
                    left: geometry.left(),
                    top: geometry.top(),
                    width: geometry.width(),
                    height: geometry.height(),
                },
            )
        })
        .collect();
    predictions.sort_by(|left, right| {
        right
            .0
            .partial_cmp(&left.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    predictions
}

fn evaluate(model: &Path, root: &Path, manifest: &Manifest, minimum_iou: f32) -> Outcome {
    // One session per distinct source geometry: the letterbox is built at load
    // from the camera's dimensions, and photographs are not all one size.
    let mut sessions: BTreeMap<(u32, u32), PersonInferenceBackend> = BTreeMap::new();
    let mut outcome = Outcome::default();
    for frame in &manifest.frames {
        let decoded = load_frame(&root.join(&frame.path), manifest);
        let backend = sessions
            .entry((frame.source_width, frame.source_height))
            .or_insert_with(|| backend_for(model, manifest, frame));
        let detections = backend
            .infer(&decoded)
            .unwrap_or_else(|error| panic!("inference must succeed: {error}"));

        let truth = truth_of(frame);
        let predictions = predictions_of(&detections);
        let (claimed, unmatched) = assign(&predictions, &truth, minimum_iou);
        outcome.ground_truth += truth.len();
        outcome.false_positives += unmatched;
        outcome.true_positives += claimed.iter().filter(|claim| **claim).count();
        for (index, person) in frame.persons.iter().enumerate() {
            let tally = outcome
                .buckets
                .entry(bucket(person[2], person[3]))
                .or_default();
            tally.total += 1;
            if claimed.get(index).copied().unwrap_or(false) {
                tally.found += 1;
            }
        }
    }
    outcome.sessions = sessions.len();
    outcome
}

#[test]
#[ignore = "requires a model artifact and a generated evaluation set"]
fn a_real_model_finds_people_across_a_set_with_ground_truth() {
    let model = PathBuf::from(required("DETECTION_MODEL_PATH"));
    let manifest_path = PathBuf::from(required("DETECTION_EVAL_MANIFEST"));
    let root = manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let manifest: Manifest = serde_json::from_slice(
        &std::fs::read(&manifest_path)
            .unwrap_or_else(|error| panic!("read {}: {error}", manifest_path.display())),
    )
    .unwrap_or_else(|error| panic!("parse {}: {error}", manifest_path.display()));
    assert!(
        !manifest.frames.is_empty(),
        "an evaluation set with no frames measures nothing"
    );
    let minimum_iou = percentage("DETECTION_EVAL_IOU");
    let minimum_recall = percentage("DETECTION_MIN_RECALL_PERCENT");
    let minimum_precision = percentage("DETECTION_MIN_PRECISION_PERCENT");

    let started = std::time::Instant::now();
    // The one place a percentage has to become a float, because overlap is
    // computed from box geometry and box geometry is f32.
    let overlap_threshold = f32::from(u8::try_from(minimum_iou).unwrap_or(100)) / 100.0;
    let outcome = evaluate(&model, &root, &manifest, overlap_threshold);
    let predicted = outcome.true_positives + outcome.false_positives;

    println!(
        "{} frames, {} people, {} sessions, {} padding, {:?}",
        manifest.frames.len(),
        outcome.ground_truth,
        outcome.sessions,
        manifest.alignment,
        started.elapsed()
    );
    println!(
        "recall {} ({}/{}), precision {} ({}/{predicted}), IoU >= {minimum_iou}%",
        show(outcome.true_positives, outcome.ground_truth),
        outcome.true_positives,
        outcome.ground_truth,
        show(outcome.true_positives, predicted),
        outcome.true_positives,
    );
    for (name, tally) in &outcome.buckets {
        println!(
            "  {name}: {} ({}/{})",
            show(tally.found, tally.total),
            tally.found,
            tally.total
        );
    }

    let recall = tenths(outcome.true_positives, outcome.ground_truth);
    let precision = tenths(outcome.true_positives, predicted);
    assert!(
        recall >= minimum_recall * 10,
        "recall {} is below the required {minimum_recall}%",
        show(outcome.true_positives, outcome.ground_truth)
    );
    assert!(
        precision >= minimum_precision * 10,
        "precision {} is below the required {minimum_precision}%",
        show(outcome.true_positives, predicted)
    );
}
