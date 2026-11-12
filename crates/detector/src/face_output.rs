//! `YuNet` 2023mar tensor decoding, independent of the runtime.
//! Contract: `OpenCV` `FaceDetectorYN`'s cls/obj/bbox stride 8/16/32 outputs.
use crate::{
    inference::{InferenceError, InferenceFailureKind},
    letterbox::Letterbox,
};
use surveillance_events::event::{
    Confidence, FaceDetection, FaceDetectionSet, NormalizedBoundingBox, MAX_DETECTIONS_PER_EVENT,
};

pub const YUNET_INPUT_SIZE: u32 = 640;
/// Fixed artifact: 80² + 40² + 20² locations, no unbounded candidate queue.
pub const YUNET_MAX_CANDIDATES: usize = 8400;
pub struct FaceLevel<'a> {
    pub stride: u16,
    pub classes: &'a [f32],
    pub objects: &'a [f32],
    pub boxes: &'a [f32],
}
fn malformed() -> InferenceError {
    InferenceError::new(InferenceFailureKind::MalformedOutput)
}
/// # Errors
/// Refuses mismatched or non-finite model tensors and invalid thresholds.
pub fn decode_yunet(
    levels: &[FaceLevel<'_>],
    geometry: Letterbox,
    score_percent: u8,
    nms_percent: u8,
) -> Result<FaceDetectionSet, InferenceError> {
    decode_yunet_observed(levels, geometry, score_percent, nms_percent).map(|(set, _)| set)
}
/// # Errors
/// Refuses malformed tensors; reports capacity drops alongside bounded results.
pub fn decode_yunet_observed(
    levels: &[FaceLevel<'_>],
    geometry: Letterbox,
    score_percent: u8,
    nms_percent: u8,
) -> Result<(FaceDetectionSet, u64), InferenceError> {
    if score_percent > 100
        || nms_percent > 100
        || geometry.target() != (640, 640)
        || levels.len() != 3
    {
        return Err(malformed());
    }
    let mut candidates = Vec::with_capacity(YUNET_MAX_CANDIDATES);
    for (level, stride) in levels.iter().zip([8_u16, 16, 32]) {
        let columns = usize::from(640 / stride);
        let count = columns * columns;
        if level.stride != stride
            || level.classes.len() != count
            || level.objects.len() != count
            || level.boxes.len() != count * 4
        {
            return Err(malformed());
        }
        if level
            .classes
            .iter()
            .chain(level.objects)
            .chain(level.boxes)
            .any(|v| !v.is_finite())
        {
            return Err(malformed());
        }
        for index in 0..count {
            let score = (level.classes[index].clamp(0.0, 1.0)
                * level.objects[index].clamp(0.0, 1.0))
            .sqrt();
            if score < f32::from(score_percent) / 100.0 {
                continue;
            }
            let prediction = &level.boxes[index * 4..index * 4 + 4];
            let column = f32::from(u16::try_from(index % columns).map_err(|_| malformed())?);
            let row = f32::from(u16::try_from(index / columns).map_err(|_| malformed())?);
            let stride = f32::from(stride);
            let width = prediction[2].exp() * stride;
            let height = prediction[3].exp() * stride;
            if !width.is_finite() || !height.is_finite() {
                return Err(malformed());
            }
            if let Some(r) = geometry.to_normalized_source(
                (column + prediction[0]) * stride - width / 2.0,
                (row + prediction[1]) * stride - height / 2.0,
                width,
                height,
            ) {
                candidates.push(FaceDetection {
                    confidence: Confidence::new(score).map_err(|_| malformed())?,
                    bounding_box: NormalizedBoundingBox::new(r.left, r.top, r.width, r.height)
                        .map_err(|_| malformed())?,
                });
            }
        }
    }
    candidates.sort_by(|a, b| b.confidence.get().total_cmp(&a.confidence.get()));
    let mut kept: Vec<FaceDetection> = Vec::with_capacity(MAX_DETECTIONS_PER_EVENT);
    let mut capacity_dropped = 0_u64;
    for candidate in candidates {
        if kept
            .iter()
            .any(|k| iou(k.bounding_box, candidate.bounding_box) > f32::from(nms_percent) / 100.0)
        {
            continue;
        }
        if kept.len() == MAX_DETECTIONS_PER_EVENT {
            capacity_dropped += 1;
        } else {
            kept.push(candidate);
        }
    }
    FaceDetectionSet::new(kept)
        .map(|set| (set, capacity_dropped))
        .map_err(|_| malformed())
}
fn iou(a: NormalizedBoundingBox, b: NormalizedBoundingBox) -> f32 {
    let w = ((a.left() + a.width()).min(b.left() + b.width()) - a.left().max(b.left())).max(0.0);
    let h = ((a.top() + a.height()).min(b.top() + b.height()) - a.top().max(b.top())).max(0.0);
    let intersection = w * h;
    intersection / (a.width() * a.height() + b.width() * b.height() - intersection)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::letterbox::LetterboxAlignment;
    fn run(score: f32, malformed_box: bool) -> Result<FaceDetectionSet, InferenceError> {
        let mut classes = [vec![0.0; 6400], vec![0.0; 1600], vec![0.0; 400]];
        let objects = [vec![1.0; 6400], vec![1.0; 1600], vec![1.0; 400]];
        let mut boxes = [vec![0.0; 25600], vec![0.0; 6400], vec![0.0; 1600]];
        // Face centered at 80,80, dimensions 40x48; not a full-body guess.
        classes[0][810] = score * score;
        boxes[0][3240..3244].copy_from_slice(&[0.0, 0.0, 5.0_f32.ln(), 6.0_f32.ln()]);
        if malformed_box {
            boxes[0][3240] = f32::NAN;
        }
        let levels: Vec<_> = [8, 16, 32]
            .into_iter()
            .enumerate()
            .map(|(i, stride)| FaceLevel {
                stride,
                classes: &classes[i],
                objects: &objects[i],
                boxes: &boxes[i],
            })
            .collect();
        decode_yunet(
            &levels,
            Letterbox::new(640, 640, 640, 640, LetterboxAlignment::TopLeft).unwrap(),
            70,
            30,
        )
    }
    #[test]
    fn crowded_output_is_bounded_and_capacity_loss_is_counted() {
        let classes = [vec![0.9; 6400], vec![0.0; 1600], vec![0.0; 400]];
        let objects = [vec![1.0; 6400], vec![1.0; 1600], vec![1.0; 400]];
        let boxes = [vec![0.0; 25600], vec![0.0; 6400], vec![0.0; 1600]];
        let levels: Vec<_> = [8, 16, 32]
            .into_iter()
            .enumerate()
            .map(|(i, stride)| FaceLevel {
                stride,
                classes: &classes[i],
                objects: &objects[i],
                boxes: &boxes[i],
            })
            .collect();
        let (set, dropped) = decode_yunet_observed(
            &levels,
            Letterbox::new(640, 640, 640, 640, LetterboxAlignment::TopLeft).unwrap(),
            70,
            45,
        )
        .unwrap();
        assert_eq!(set.as_slice().len(), 64);
        assert!(dropped > 0);
    }

    #[test]
    fn seventy_percent_is_inclusive() {
        assert!(run(0.699, false).unwrap().as_slice().is_empty());
        assert_eq!(run(0.7, false).unwrap().as_slice().len(), 1);
        assert_eq!(run(0.701, false).unwrap().as_slice().len(), 1);
    }
    #[test]
    fn malformed_output_fails_closed() {
        assert!(run(0.9, true).is_err());
    }
    #[test]
    fn boxes_are_face_geometry() {
        let set = run(0.9, false).unwrap();
        let b = set.as_slice()[0].bounding_box;
        assert!((b.width() - 0.0625).abs() < 1e-5);
        assert!((b.height() - 0.075).abs() < 1e-5);
    }
}
