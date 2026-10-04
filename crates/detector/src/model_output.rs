//! Family-scoped decoding of a model output tensor into normalized detections.
//!
//! The output layout is the only part of inference that changes with the model,
//! so it is the only part behind its own seam. A decoder receives numbers and
//! geometry and nothing else: no frame, no session, no runtime. That is what
//! lets the arithmetic below be tested against a tensor written by hand, with
//! no model file and no ONNX Runtime present.

use surveillance_events::event::{
    Confidence, DetectionSet, NormalizedBoundingBox, PersonDetection, MAX_DETECTIONS_PER_EVENT,
};

use crate::letterbox::{Letterbox, LetterboxAlignment};

/// The COCO class index this product cares about.
const PERSON_CLASS: usize = 0;

/// Columns preceding the class scores: centre x, centre y, width, height, objectness.
const BOX_AND_OBJECTNESS_COLUMNS: usize = 5;

/// Why an output tensor could not be decoded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelDecodeError {
    /// The tensor rank or extents are not what this family produces.
    UnexpectedShape,
    /// The tensor length disagrees with the shape describing it.
    ShapeLengthMismatch,
    /// The row count matches no decomposition of the configured input size.
    UnexpectedRowCount,
    /// Decoded geometry violated the event contract.
    InvalidGeometry,
    /// A score or overlap bound was outside `0..=1`.
    InvalidThreshold,
}

impl core::fmt::Display for ModelDecodeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let reason = match self {
            Self::UnexpectedShape => "output tensor shape is not this model family's",
            Self::ShapeLengthMismatch => "output tensor length disagrees with its shape",
            Self::UnexpectedRowCount => "output row count matches no stride decomposition",
            Self::InvalidGeometry => "decoded geometry violated the event contract",
            Self::InvalidThreshold => "score and overlap bounds must be within 0..=1",
        };
        formatter.write_str(reason)
    }
}

impl core::error::Error for ModelDecodeError {}

/// Score and overlap bounds, supplied per call rather than held.
///
/// These are passed in rather than stored so the configuration layer can
/// classify them as taking effect immediately. A decoder that captured them at
/// construction would make that classification false, and the settings domain
/// treats reporting `Immediate` for a value that only changes on restart as the
/// failure to avoid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DecodeThresholds {
    score: f32,
    nms_iou: f32,
}

impl DecodeThresholds {
    /// # Errors
    /// Returns an error unless both bounds are finite and within `0..=1`.
    pub fn new(score: f32, nms_iou: f32) -> Result<Self, ModelDecodeError> {
        let sane = |value: f32| value.is_finite() && (0.0..=1.0).contains(&value);
        if !sane(score) || !sane(nms_iou) {
            return Err(ModelDecodeError::InvalidThreshold);
        }
        Ok(Self { score, nms_iou })
    }

    /// Builds from the integer percentages the settings domain stores.
    ///
    /// The settings value model has no float, and the house precedent for a
    /// fraction is an integer percentage, as the storage watermarks use. That
    /// also keeps the value exact through TOML, the database and JSON, and
    /// makes a NaN threshold unrepresentable rather than merely rejected.
    ///
    /// # Errors
    /// Returns an error when either percentage exceeds 100.
    pub fn from_percentages(score: u8, nms_iou: u8) -> Result<Self, ModelDecodeError> {
        if score > 100 || nms_iou > 100 {
            return Err(ModelDecodeError::InvalidThreshold);
        }
        Self::new(f32::from(score) / 100.0, f32::from(nms_iou) / 100.0)
    }
}

/// Converts one model family's output tensor into normalized detections.
pub trait ModelOutputDecoder: Send + 'static {
    /// # Errors
    /// Returns a classified decode failure. Never panics on tensor content.
    fn decode(
        &self,
        values: &[f32],
        shape: &[usize],
        letterbox: Letterbox,
        thresholds: DecodeThresholds,
    ) -> Result<DetectionSet, ModelDecodeError>;
}

/// Decoder for the YOLOX family's raw export.
///
/// The published `yolox_nano.onnx` is exported without `decode_in_inference`,
/// established by running it: its first four columns ranged -2.07 to 3.48,
/// which is neither pixel nor normalized space. Grid and stride reconstruction
/// therefore belongs here. Objectness and class columns arrive already
/// sigmoid-activated and must not be activated again.
#[derive(Clone, Copy, Debug, Default)]
pub struct YoloxDecoder;

/// The feature-map strides YOLOX concatenates, in output order.
const YOLOX_STRIDES: [u32; 3] = [8, 16, 32];

/// Where YOLOX preprocessing puts the scaled image on the padded canvas.
///
/// Its own `preproc` writes the resized image at `[:h, :w]` of a 114-grey
/// canvas, so the padding falls on the right and bottom. Exported here because
/// the pipeline that produces frames and the inverse that reads boxes back must
/// use the same value, and a family's preprocessing is the only thing that
/// decides which value is right.
pub const YOLOX_LETTERBOX_ALIGNMENT: LetterboxAlignment = LetterboxAlignment::TopLeft;

impl YoloxDecoder {
    /// Rebuilds the anchor grid the raw export omits.
    ///
    /// Each stride level contributes `(width / stride) * (height / stride)`
    /// rows in stride order. The caller compares the total against the tensor:
    /// a mismatch means this is not the family it claims to be.
    fn grid(letterbox: Letterbox) -> Vec<(f32, f32, f32)> {
        let (target_width, target_height) = letterbox.target();
        let mut cells = Vec::new();
        for stride in YOLOX_STRIDES {
            let columns = target_width / stride;
            let rows = target_height / stride;
            for row in 0..rows {
                for column in 0..columns {
                    cells.push((
                        f32::from(u16::try_from(column).unwrap_or(u16::MAX)),
                        f32::from(u16::try_from(row).unwrap_or(u16::MAX)),
                        f32::from(u16::try_from(stride).unwrap_or(u16::MAX)),
                    ));
                }
            }
        }
        cells
    }
}

/// One candidate in model input pixel space, before suppression.
#[derive(Clone, Copy, Debug)]
struct Candidate {
    score: f32,
    left: f32,
    top: f32,
    width: f32,
    height: f32,
}

impl ModelOutputDecoder for YoloxDecoder {
    fn decode(
        &self,
        values: &[f32],
        shape: &[usize],
        letterbox: Letterbox,
        thresholds: DecodeThresholds,
    ) -> Result<DetectionSet, ModelDecodeError> {
        let (rows, columns) = match shape {
            [1, rows, columns] if *columns > BOX_AND_OBJECTNESS_COLUMNS => (*rows, *columns),
            _ => return Err(ModelDecodeError::UnexpectedShape),
        };
        if values.len() != rows.saturating_mul(columns) {
            return Err(ModelDecodeError::ShapeLengthMismatch);
        }
        let grid = YoloxDecoder::grid(letterbox);
        if grid.len() != rows {
            return Err(ModelDecodeError::UnexpectedRowCount);
        }

        let mut candidates = Vec::new();
        for (row, cell) in grid.iter().enumerate() {
            let base = row * columns;
            let Some(window) = values.get(base..base + columns) else {
                return Err(ModelDecodeError::ShapeLengthMismatch);
            };
            let score = window[4] * window[BOX_AND_OBJECTNESS_COLUMNS + PERSON_CLASS];
            if !score.is_finite() || score < thresholds.score {
                continue;
            }
            let (column_index, row_index, stride) = *cell;
            let centre_x = (window[0] + column_index) * stride;
            let centre_y = (window[1] + row_index) * stride;
            let width = window[2].exp() * stride;
            let height = window[3].exp() * stride;
            if ![centre_x, centre_y, width, height]
                .iter()
                .all(|v| v.is_finite())
            {
                continue;
            }
            candidates.push(Candidate {
                score,
                left: centre_x - width / 2.0,
                top: centre_y - height / 2.0,
                width,
                height,
            });
        }

        let kept = suppress(&mut candidates, thresholds.nms_iou);
        let mut detections = Vec::new();
        for candidate in kept {
            if detections.len() == MAX_DETECTIONS_PER_EVENT {
                break;
            }
            let Some(rect) = letterbox.to_normalized_source(
                candidate.left,
                candidate.top,
                candidate.width,
                candidate.height,
            ) else {
                continue;
            };
            let confidence = Confidence::new(candidate.score.clamp(0.0, 1.0))
                .map_err(|_| ModelDecodeError::InvalidGeometry)?;
            let bounding_box =
                NormalizedBoundingBox::new(rect.left, rect.top, rect.width, rect.height)
                    .map_err(|_| ModelDecodeError::InvalidGeometry)?;
            detections.push(PersonDetection {
                confidence,
                bounding_box,
            });
        }
        DetectionSet::new(detections).map_err(|_| ModelDecodeError::InvalidGeometry)
    }
}

/// Greedy non-maximum suppression over candidates, highest score first.
fn suppress(candidates: &mut [Candidate], iou_threshold: f32) -> Vec<Candidate> {
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(core::cmp::Ordering::Equal)
    });
    let mut kept: Vec<Candidate> = Vec::new();
    for candidate in candidates.iter() {
        if kept
            .iter()
            .any(|k| intersection_over_union(*k, *candidate) > iou_threshold)
        {
            continue;
        }
        kept.push(*candidate);
    }
    kept
}

fn intersection_over_union(a: Candidate, b: Candidate) -> f32 {
    let left = a.left.max(b.left);
    let top = a.top.max(b.top);
    let right = (a.left + a.width).min(b.left + b.width);
    let bottom = (a.top + a.height).min(b.top + b.height);
    let overlap = (right - left).max(0.0) * (bottom - top).max(0.0);
    let union = a.width * a.height + b.width * b.height - overlap;
    if union <= 0.0 {
        return 0.0;
    }
    overlap / union
}

#[cfg(test)]
mod tests {
    use super::{
        DecodeThresholds, ModelDecodeError, ModelOutputDecoder, YoloxDecoder,
        BOX_AND_OBJECTNESS_COLUMNS,
    };
    use crate::letterbox::{Letterbox, LetterboxAlignment};
    use surveillance_events::event::MAX_DETECTIONS_PER_EVENT;

    /// A 32x32 input decomposes to 4x4 + 2x2 + 1x1 = 21 rows.
    const SMALL_ROWS: usize = 21;
    const COLUMNS: usize = BOX_AND_OBJECTNESS_COLUMNS + 2;

    fn small_letterbox() -> Letterbox {
        Letterbox::new(64, 64, 32, 32, LetterboxAlignment::Centred)
            .unwrap_or_else(|error| panic!("valid letterbox: {error}"))
    }

    fn decoder() -> YoloxDecoder {
        YoloxDecoder
    }

    fn thresholds() -> DecodeThresholds {
        DecodeThresholds::new(0.3, 0.45).unwrap_or_else(|error| panic!("valid bounds: {error}"))
    }

    fn blank(rows: usize) -> Vec<f32> {
        vec![0.0; rows * COLUMNS]
    }

    /// Writes one candidate into a row, in the raw layout the export produces.
    fn put(values: &mut [f32], row: usize, raw: [f32; 4], objectness: f32, person: f32) {
        let base = row * COLUMNS;
        values[base..base + 4].copy_from_slice(&raw);
        values[base + 4] = objectness;
        values[base + BOX_AND_OBJECTNESS_COLUMNS] = person;
    }

    #[test]
    fn the_reconstructed_grid_matches_the_measured_row_count() {
        // The real artifact: 416x416 produced exactly 3549 rows.
        let letterbox = Letterbox::new(1280, 720, 416, 416, LetterboxAlignment::Centred)
            .unwrap_or_else(|error| panic!("valid letterbox: {error}"));
        assert_eq!(YoloxDecoder::grid(letterbox).len(), 3549);
        assert_eq!(YoloxDecoder::grid(small_letterbox()).len(), SMALL_ROWS);
    }

    #[test]
    fn a_confident_person_decodes_to_the_expected_box() {
        let mut values = blank(SMALL_ROWS);
        // Row 0 is grid cell (0,0) at stride 8, so centre is (0+0)*8 = 0 after
        // adding the raw offset below, and the box is exp(0)*8 = 8 wide.
        put(&mut values, 0, [2.0, 2.0, 0.0, 0.0], 0.9, 0.9);
        let set = decoder()
            .decode(
                &values,
                &[1, SMALL_ROWS, COLUMNS],
                small_letterbox(),
                thresholds(),
            )
            .unwrap_or_else(|error| panic!("decode: {error}"));
        assert_eq!(set.as_slice().len(), 1);
        let detection = &set.as_slice()[0];
        assert!((f32::from(detection.confidence) - 0.81).abs() < 1e-5);
        // centre (16,16) in a 32x32 input over a 64x64 source: centre of frame.
        let geometry = detection.bounding_box;
        assert!((geometry.left() + geometry.width() / 2.0 - 0.5).abs() < 1e-3);
        assert!((geometry.top() + geometry.height() / 2.0 - 0.5).abs() < 1e-3);
    }

    #[test]
    fn a_sub_threshold_candidate_is_dropped() {
        let mut values = blank(SMALL_ROWS);
        put(&mut values, 0, [2.0, 2.0, 0.0, 0.0], 0.5, 0.5); // 0.25 < 0.3
        let set = decoder()
            .decode(
                &values,
                &[1, SMALL_ROWS, COLUMNS],
                small_letterbox(),
                thresholds(),
            )
            .unwrap_or_else(|error| panic!("decode: {error}"));
        assert!(set.as_slice().is_empty());
    }

    #[test]
    fn a_non_person_class_is_filtered_out() {
        let mut values = blank(SMALL_ROWS);
        let base = 0;
        put(&mut values, base, [2.0, 2.0, 0.0, 0.0], 0.95, 0.01);
        // High score on the second class must not rescue it.
        values[base * COLUMNS + BOX_AND_OBJECTNESS_COLUMNS + 1] = 0.99;
        let set = decoder()
            .decode(
                &values,
                &[1, SMALL_ROWS, COLUMNS],
                small_letterbox(),
                thresholds(),
            )
            .unwrap_or_else(|error| panic!("decode: {error}"));
        assert!(set.as_slice().is_empty());
    }

    #[test]
    fn overlapping_candidates_collapse_to_one() {
        let mut values = blank(SMALL_ROWS);
        // Rows 0 and 1 are adjacent stride-8 cells; identical raw offsets put
        // their boxes almost on top of each other.
        put(&mut values, 0, [2.0, 2.0, 1.0, 1.0], 0.9, 0.9);
        put(&mut values, 1, [1.0, 2.0, 1.0, 1.0], 0.8, 0.9);
        let set = decoder()
            .decode(
                &values,
                &[1, SMALL_ROWS, COLUMNS],
                small_letterbox(),
                thresholds(),
            )
            .unwrap_or_else(|error| panic!("decode: {error}"));
        assert_eq!(
            set.as_slice().len(),
            1,
            "suppression should keep the stronger box"
        );
        assert!(f32::from(set.as_slice()[0].confidence) > 0.8);
    }

    #[test]
    fn results_are_truncated_to_the_event_bound() {
        let letterbox = Letterbox::new(1280, 720, 416, 416, LetterboxAlignment::Centred)
            .unwrap_or_else(|error| panic!("valid letterbox: {error}"));
        let rows = YoloxDecoder::grid(letterbox).len();
        let mut values = vec![0.0; rows * COLUMNS];
        // Grid row 20 of the stride-8 level puts these at y=160 in model
        // space, inside the image band. Row 0 would sit in the 91-pixel top
        // padding, where every box is correctly discarded as describing
        // nothing the camera saw.
        for offset in 0..100 {
            put(
                &mut values,
                20 * 52 + offset,
                [0.0, 0.0, 1.0, 1.0],
                0.9,
                0.9,
            );
        }
        // An IoU bound of 1.0 suppresses nothing, so truncation is what limits.
        let permissive =
            DecodeThresholds::new(0.3, 1.0).unwrap_or_else(|error| panic!("valid bounds: {error}"));
        let set = decoder()
            .decode(&values, &[1, rows, COLUMNS], letterbox, permissive)
            .unwrap_or_else(|error| panic!("decode: {error}"));
        assert_eq!(set.as_slice().len(), MAX_DETECTIONS_PER_EVENT);
    }

    #[test]
    fn padding_candidates_do_not_consume_the_detection_limit() {
        let letterbox = Letterbox::new(1280, 720, 416, 416, LetterboxAlignment::Centred)
            .unwrap_or_else(|error| panic!("valid letterbox: {error}"));
        let rows = YoloxDecoder::grid(letterbox).len();
        let mut values = vec![0.0; rows * COLUMNS];
        for row in 0..MAX_DETECTIONS_PER_EVENT {
            // These boxes are entirely in the top letterbox padding.
            put(&mut values, row, [0.0, 0.0, 0.0, 0.0], 0.99, 0.99);
        }
        // A lower-scoring valid candidate follows all the padding-only boxes.
        put(&mut values, 20 * 52, [0.0, 0.0, 0.0, 0.0], 0.8, 0.8);
        let permissive =
            DecodeThresholds::new(0.3, 1.0).unwrap_or_else(|error| panic!("valid bounds: {error}"));

        let set = decoder()
            .decode(&values, &[1, rows, COLUMNS], letterbox, permissive)
            .unwrap_or_else(|error| panic!("decode: {error}"));

        assert_eq!(set.as_slice().len(), 1);
    }

    #[test]
    fn an_unexpected_shape_is_classified_not_panicked() {
        let values = blank(SMALL_ROWS);
        for shape in [
            vec![SMALL_ROWS, COLUMNS],
            vec![2, SMALL_ROWS, COLUMNS],
            vec![1, SMALL_ROWS, BOX_AND_OBJECTNESS_COLUMNS],
        ] {
            assert_eq!(
                decoder().decode(&values, &shape, small_letterbox(), thresholds()),
                Err(ModelDecodeError::UnexpectedShape),
                "shape {shape:?} must be refused"
            );
        }
    }

    #[test]
    fn a_row_count_from_another_model_is_refused() {
        let values = blank(SMALL_ROWS + 1);
        assert_eq!(
            decoder().decode(
                &values,
                &[1, SMALL_ROWS + 1, COLUMNS],
                small_letterbox(),
                thresholds()
            ),
            Err(ModelDecodeError::UnexpectedRowCount)
        );
    }

    #[test]
    fn a_length_disagreeing_with_its_shape_is_refused() {
        let values = blank(SMALL_ROWS);
        assert_eq!(
            decoder().decode(
                &values[..values.len() - 1],
                &[1, SMALL_ROWS, COLUMNS],
                small_letterbox(),
                thresholds()
            ),
            Err(ModelDecodeError::ShapeLengthMismatch)
        );
    }

    #[test]
    fn a_non_finite_row_is_skipped_rather_than_emitted() {
        let mut values = blank(SMALL_ROWS);
        put(&mut values, 0, [f32::NAN, 0.0, 0.0, 0.0], 0.9, 0.9);
        put(&mut values, 1, [0.0, 0.0, 500.0, 0.0], 0.9, 0.9); // exp overflows
        let set = decoder()
            .decode(
                &values,
                &[1, SMALL_ROWS, COLUMNS],
                small_letterbox(),
                thresholds(),
            )
            .unwrap_or_else(|error| panic!("decode: {error}"));
        assert!(set.as_slice().is_empty());
    }

    #[test]
    fn thresholds_outside_the_unit_range_are_refused() {
        assert!(DecodeThresholds::new(1.5, 0.45).is_err());
        assert!(DecodeThresholds::new(0.3, f32::NAN).is_err());
        assert!(DecodeThresholds::from_percentages(101, 45).is_err());
    }

    #[test]
    fn percentages_convert_to_the_fractions_they_name() {
        let bounds = DecodeThresholds::from_percentages(30, 45)
            .unwrap_or_else(|error| panic!("valid bounds: {error}"));
        assert_eq!(bounds, thresholds());
    }
}
