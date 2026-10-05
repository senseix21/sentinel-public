//! `IoU`-based tracker that assigns stable [`TrackId`]s to detections across frames.
//!
//! One instance per detection session (one camera's `DetectionOwner`). The
//! tracker sits between inference and event publication, enriching each
//! detection with a [`TrackId`] that persists while the person remains in
//! view. A track that is unmatched for `max_frames_missing` frames is retired,
//! and its id is never reused in the same session.
//!
//! The matcher is greedy: at each step the pair with the highest `IoU` above
//! `min_iou` is matched first. `n <= 64` tracks per side keeps the quadratic
//! cost immaterial, and greedy matches the industry baseline for this problem
//! size — Hungarian would be a one-point improvement at a cost of a crate.

use std::num::NonZeroU32;
use surveillance_events::event::{
    Confidence, DetectionSet, NormalizedBoundingBox, PersonDetection, TrackId,
    MAX_DETECTIONS_PER_EVENT,
};

/// Standard `IoU` on [`NormalizedBoundingBox`].
///
/// Returns zero when the boxes do not overlap or when the union is zero.
#[must_use]
pub fn iou(a: NormalizedBoundingBox, b: NormalizedBoundingBox) -> f32 {
    let x1 = a.left().max(b.left());
    let y1 = a.top().max(b.top());
    let x2 = (a.left() + a.width()).min(b.left() + b.width());
    let y2 = (a.top() + a.height()).min(b.top() + b.height());
    let inter_w = (x2 - x1).max(0.0);
    let inter_h = (y2 - y1).max(0.0);
    let inter = inter_w * inter_h;
    let union = a.width().mul_add(a.height(), b.width() * b.height()) - inter;
    if union > 0.0 {
        inter / union
    } else {
        0.0
    }
}

/// Tracker tuning.
#[derive(Debug, Clone, Copy)]
pub struct TrackerConfig {
    /// Minimum `IoU` for a detection to match an existing track. Below this
    /// threshold the detection becomes a new track rather than updating one.
    pub min_iou: f32,
    /// Frames a track may be unmatched before retirement. A track with
    /// `frames_missing > max_frames_missing` is removed.
    pub max_frames_missing: u32,
}

impl Default for TrackerConfig {
    fn default() -> Self {
        Self {
            min_iou: 0.3,
            max_frames_missing: 5,
        }
    }
}

struct TrackState {
    id: TrackId,
    last_box: NormalizedBoundingBox,
    last_confidence: Confidence,
    frames_missing: u32,
}

/// `IoU` tracker. One instance per detection session.
pub struct PersonTracker {
    tracks: Vec<TrackState>,
    /// Next id to issue. Starts at 1 (0 is reserved for `NonZeroU32`), wraps
    /// past `u32::MAX` back to 1. Within a session, retired ids are not
    /// reissued until a wrap, which is not reachable in practice.
    next_id: u32,
    config: TrackerConfig,
}

impl PersonTracker {
    #[must_use]
    pub const fn new(config: TrackerConfig) -> Self {
        Self {
            tracks: Vec::new(),
            next_id: 1,
            config,
        }
    }

    /// Runs one tracking frame.
    ///
    /// Every detection in the returned [`DetectionSet`] has
    /// `track_id: Some(...)`. Coasting tracks (`frames_missing > 0`) appear at
    /// their last known position; when matched + coasting exceeds
    /// [`MAX_DETECTIONS_PER_EVENT`], coasting tracks with the highest
    /// `frames_missing` are dropped first.
    pub fn update_detections(&mut self, detections: &DetectionSet) -> DetectionSet {
        let tracked = self.update(detections);
        // SAFETY: `update` respects `MAX_DETECTIONS_PER_EVENT`; every box in
        // `tracked` was already a validated `NormalizedBoundingBox`, so the
        // only validation `DetectionSet::new` performs — the cardinality
        // check — cannot fail here.
        DetectionSet::new(tracked)
            .unwrap_or_else(|_| unreachable!("tracker respects DetectionSet capacity"))
    }

    fn update(&mut self, detections: &DetectionSet) -> Vec<PersonDetection> {
        let dets = detections.as_slice();
        let nt = self.tracks.len();
        let nd = dets.len();

        let mut matched_tracks = vec![false; nt];
        let mut matched_dets = vec![false; nd];

        // Candidate pairs with `IoU` above the threshold, sorted descending.
        let mut pairs: Vec<(f32, usize, usize)> = Vec::with_capacity(nt * nd);
        for (ti, track) in self.tracks.iter().enumerate() {
            for (di, det) in dets.iter().enumerate() {
                let score = iou(track.last_box, det.bounding_box);
                if score >= self.config.min_iou {
                    pairs.push((score, ti, di));
                }
            }
        }
        pairs.sort_unstable_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        for (_, ti, di) in &pairs {
            if !matched_tracks[*ti] && !matched_dets[*di] {
                matched_tracks[*ti] = true;
                matched_dets[*di] = true;
                self.tracks[*ti].last_box = dets[*di].bounding_box;
                self.tracks[*ti].last_confidence = dets[*di].confidence;
                self.tracks[*ti].frames_missing = 0;
            }
        }

        // Coast unmatched tracks; collect indices to retire. Walk in reverse
        // order when removing so swap_remove indices remain valid.
        let mut retire: Vec<usize> = Vec::new();
        for (ti, track) in self.tracks.iter_mut().enumerate() {
            if !matched_tracks[ti] {
                track.frames_missing = track.frames_missing.saturating_add(1);
                if track.frames_missing > self.config.max_frames_missing {
                    retire.push(ti);
                }
            }
        }
        for &ti in retire.iter().rev() {
            self.tracks.swap_remove(ti);
        }

        // Create new tracks for unmatched incoming detections.
        for (di, det) in dets.iter().enumerate() {
            if !matched_dets[di] {
                let raw = self.next_id;
                // SAFETY: `next_id` is 1 at construction and is replaced only
                // via `advance_next_id`, which keeps it nonzero.
                let id = NonZeroU32::new(raw).unwrap_or_else(|| {
                    unreachable!("next_id invariant: never zero after construction")
                });
                self.advance_next_id();
                self.tracks.push(TrackState {
                    id,
                    last_box: det.bounding_box,
                    last_confidence: det.confidence,
                    frames_missing: 0,
                });
            }
        }

        // Build output, capped at MAX_DETECTIONS_PER_EVENT. Matched tracks
        // (frames_missing == 0) sort first; if trimming is needed, the oldest
        // coasters are dropped.
        let mut order: Vec<(u32, usize)> = self
            .tracks
            .iter()
            .enumerate()
            .map(|(i, t)| (t.frames_missing, i))
            .collect();
        order.sort_unstable();

        order
            .into_iter()
            .take(MAX_DETECTIONS_PER_EVENT)
            .map(|(_, ti)| {
                let t = &self.tracks[ti];
                PersonDetection {
                    confidence: t.last_confidence,
                    bounding_box: t.last_box,
                    track_id: Some(t.id),
                }
            })
            .collect()
    }

    /// Advances `next_id`, wrapping back to 1 past `u32::MAX`. The wrap-around
    /// is defence in depth: a session that issues 4 billion ids has worse
    /// problems than ID reuse.
    fn advance_next_id(&mut self) {
        self.next_id = match self.next_id.checked_add(1) {
            Some(0) | None => 1,
            Some(n) => n,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bbox(left: f32, top: f32, width: f32, height: f32) -> NormalizedBoundingBox {
        NormalizedBoundingBox::new(left, top, width, height)
            .unwrap_or_else(|error| panic!("bbox: {error}"))
    }

    fn det(left: f32, top: f32, width: f32, height: f32) -> PersonDetection {
        PersonDetection {
            confidence: Confidence::new(0.9).unwrap_or_else(|error| panic!("conf: {error}")),
            bounding_box: bbox(left, top, width, height),
            track_id: None,
        }
    }

    fn detection_set(dets: Vec<PersonDetection>) -> DetectionSet {
        DetectionSet::new(dets).unwrap_or_else(|error| panic!("set: {error}"))
    }

    fn tracker() -> PersonTracker {
        PersonTracker::new(TrackerConfig::default())
    }

    #[test]
    fn iou_identical_boxes_is_one() {
        let b = bbox(0.1, 0.1, 0.5, 0.5);
        assert!((iou(b, b) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn iou_non_overlapping_boxes_is_zero() {
        let a = bbox(0.0, 0.0, 0.4, 0.4);
        let b = bbox(0.6, 0.6, 0.3, 0.3);
        assert!(iou(a, b).abs() < 1e-6);
    }

    #[test]
    fn iou_half_overlap() {
        // a = [0.0..0.4, 0.0..0.4]  area = 0.16
        // b = [0.2..0.6, 0.0..0.4]  area = 0.16
        // intersection = [0.2..0.4, 0.0..0.4] = 0.08
        // union = 0.16 + 0.16 - 0.08 = 0.24
        let a = bbox(0.0, 0.0, 0.4, 0.4);
        let b = bbox(0.2, 0.0, 0.4, 0.4);
        assert!((iou(a, b) - (0.08_f32 / 0.24_f32)).abs() < 1e-5);
    }

    #[test]
    fn single_person_gets_stable_id_across_frames() {
        let mut t = tracker();
        let r1 = t.update_detections(&detection_set(vec![det(0.1, 0.1, 0.3, 0.5)]));
        let id1 = r1.as_slice()[0]
            .track_id
            .unwrap_or_else(|| panic!("first frame must assign an id"));

        let r2 = t.update_detections(&detection_set(vec![det(0.12, 0.11, 0.3, 0.5)]));
        let id2 = r2.as_slice()[0]
            .track_id
            .unwrap_or_else(|| panic!("second frame must have an id"));
        assert_eq!(
            id2, id1,
            "the same person must keep the same id across frames"
        );
    }

    #[test]
    fn two_persons_ids_do_not_swap() {
        let mut t = tracker();
        let first = detection_set(vec![det(0.0, 0.1, 0.2, 0.5), det(0.7, 0.1, 0.2, 0.5)]);
        let r1 = t.update_detections(&first);
        let (id_left, id_right) = {
            let s = r1.as_slice();
            let (a_id, b_id) = (
                s[0].track_id.expect("id after frame 1"),
                s[1].track_id.expect("id after frame 1"),
            );
            if s[0].bounding_box.left() < s[1].bounding_box.left() {
                (a_id, b_id)
            } else {
                (b_id, a_id)
            }
        };

        // Both move slightly but stay on their sides.
        let second = detection_set(vec![det(0.02, 0.11, 0.2, 0.5), det(0.68, 0.11, 0.2, 0.5)]);
        let r2 = t.update_detections(&second);
        let s = r2.as_slice();
        let (new_left, new_right) = if s[0].bounding_box.left() < s[1].bounding_box.left() {
            (s[0].track_id.unwrap(), s[1].track_id.unwrap())
        } else {
            (s[1].track_id.unwrap(), s[0].track_id.unwrap())
        };
        assert_eq!(new_left, id_left, "left person must keep its id");
        assert_eq!(new_right, id_right, "right person must keep its id");
    }

    #[test]
    fn track_coasts_through_missed_frames_then_retires() {
        let config = TrackerConfig {
            min_iou: 0.3,
            max_frames_missing: 2,
        };
        let mut t = PersonTracker::new(config);
        let r1 = t.update_detections(&detection_set(vec![det(0.1, 0.1, 0.3, 0.5)]));
        let id = r1.as_slice()[0].track_id.unwrap();

        let r2 = t.update_detections(&detection_set(vec![]));
        assert_eq!(r2.as_slice().len(), 1, "coasting after one miss");
        assert_eq!(r2.as_slice()[0].track_id.unwrap(), id);

        let r3 = t.update_detections(&detection_set(vec![]));
        assert_eq!(r3.as_slice().len(), 1, "still coasting at the limit");

        let r4 = t.update_detections(&detection_set(vec![]));
        assert!(r4.as_slice().is_empty(), "retired past the limit");
    }

    #[test]
    fn retired_id_is_never_reused_in_this_session() {
        let config = TrackerConfig {
            min_iou: 0.3,
            max_frames_missing: 0,
        };
        let mut t = PersonTracker::new(config);
        let r1 = t.update_detections(&detection_set(vec![det(0.1, 0.1, 0.3, 0.3)]));
        let first_id = r1.as_slice()[0].track_id.unwrap();

        // Miss once with `max_frames_missing = 0` retires immediately.
        t.update_detections(&detection_set(vec![]));

        let r3 = t.update_detections(&detection_set(vec![det(0.1, 0.1, 0.3, 0.3)]));
        let new_id = r3.as_slice()[0].track_id.unwrap();
        assert!(
            new_id > first_id,
            "re-entering person must get a higher id than {first_id}, got {new_id}"
        );
    }

    #[test]
    fn output_capped_at_max_detections() {
        let config = TrackerConfig {
            min_iou: 0.3,
            max_frames_missing: 10,
        };
        let mut t = PersonTracker::new(config);

        // Establish 60 coasting tracks.
        let first_wave: Vec<_> = (0_u16..60)
            .map(|i| det(f32::from(i).mul_add(0.015, 0.0), 0.0, 0.01, 0.01))
            .collect();
        t.update_detections(&detection_set(first_wave));

        // 10 new detections, non-overlapping, so none match coasters.
        let second_wave: Vec<_> = (0_u16..10)
            .map(|i| det(f32::from(i).mul_add(0.015, 0.0), 0.5, 0.01, 0.01))
            .collect();
        let result = t.update_detections(&detection_set(second_wave));
        assert!(
            result.as_slice().len() <= MAX_DETECTIONS_PER_EVENT,
            "output must not exceed {MAX_DETECTIONS_PER_EVENT}, got {}",
            result.as_slice().len()
        );
    }

    #[test]
    fn every_output_detection_has_a_track_id() {
        let mut t = tracker();
        let dets = detection_set(vec![det(0.1, 0.1, 0.2, 0.3), det(0.5, 0.1, 0.2, 0.3)]);
        let result = t.update_detections(&dets);
        for d in result.as_slice() {
            assert!(
                d.track_id.is_some(),
                "every output detection must have an id"
            );
        }
    }
}
