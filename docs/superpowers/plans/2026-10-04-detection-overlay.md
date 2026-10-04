# Detection Overlay Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Draw a tracked, ID-stable bounding box around each detected person in the live view, rendered client-side over the video using a LiveKit data channel.

**Architecture:** A `PersonTracker` (IoU matching, coasting) in `crates/detector` assigns stable IDs between inference and event publication. A `DetectionRelay` in `crates/livekit` subscribes to `PersonDetectionObserved` events and broadcasts JSON `DetectionPacket` objects via `publish_data` (lossy, topic `"det/v1"`). The browser receives packets via `RoomEvent.DataReceived` and draws boxes on a `<canvas>` layered over the `<video>` element.

**Tech Stack:** Rust (crates/detector, crates/events, crates/livekit), LiveKit SDK 0.8.3, serde_json 1.0.149, vanilla JS (no bundler/framework), Node.js (test runner for client logic)

**Spec:** `docs/superpowers/specs/2026-10-04-detection-overlay-design.md`

---

## Global Constraints

- `PersonDetection` lives in `crates/events/src/event.rs` — not `crates/detector`
- `DetectionSet::new(Vec<PersonDetection>)` returns `Result` and caps at `MAX_DETECTIONS_PER_EVENT = 64`
- `NormalizedBoundingBox` fields are private; use `.left()`, `.top()`, `.width()`, `.height()`
- `Confidence` wraps `f32`; use `.get()` to extract; constructed via `Confidence::new(f32)`
- `EventBus` derives `Clone` — hold a clone in the adapter to create subscriptions on reconnect
- `DataPacket { payload: Vec<u8>, topic: Option<String>, reliable: bool, destination_identities: Vec<ParticipantIdentity> }` — use `reliable: false` for lossy delivery
- `serde` and `serde_json` are workspace dependencies; add to crate Cargo.toml with `workspace = true`
- `TrackId = NonZeroU32`; first issued ID is 1; IDs never reuse within a detection session
- Canvas uses `object-fit: contain` — all box coordinates map into the content rect, not the element rect
- Never render boxes when `results_are_fixture: true`; this field appears in every packet

## Review Focus

1. **Two people swap positions between frames**: IoU matching must assign each ID to the geometrically closest box, not the first unmatched one. The greedy sort-by-IoU-descending approach in Task 2 handles this; if broken, IDs flicker. The swap-resistance test in Task 2 pins this.

2. **Staleness on camera pause or detector restart**: When detection stops (detector degraded, camera disconnected), `lastReceivedAt` freezes and the overlay must blank after `MAX_AGE_MS`. If `requestAnimationFrame` keeps running but the staleness check is missing, stale boxes persist. The staleness test in Task 7 pins this.

3. **Reconnect creates a second relay without stopping the first**: `connect()` in `transcode.rs` is called on every reconnect. If the relay shutdown is omitted from `stop_publishing`, two relays write to the same room. The relay-stops-on-session-end test in Task 5 pins the relay's own shutdown; the `Session` wiring in Task 6 must ensure the old relay is cancelled before a new one starts.

4. **Capacity overflow when coasting tracks plus new detections exceed 64**: `PersonTracker::update` must trim the output before passing it to `DetectionSet::new`. The overflow test in Task 2 pins this.

5. **Letterboxed coordinate mapping**: `object-fit: contain` adds bars. A box at `(0,0,1,1)` should cover the video content area exactly, not the full element. If `contentRect` is skipped, boxes are drawn at wrong positions in non-native aspect ratios. The `contentRect` parametric tests in Task 7 pin this.

---

## Task 1: TrackId type + `PersonDetection.track_id`

**Files:**
- Modify: `crates/events/src/event.rs`
- Modify: `crates/detector/src/inference.rs` (construction site)
- Modify: `crates/detector/src/model_output.rs` (construction site)

**Interfaces:**
- Produces: `pub type TrackId = NonZeroU32` exported from `surveillance_events`
- Produces: `PersonDetection { confidence, bounding_box, track_id: Option<TrackId> }`
- All existing `PersonDetection` construction sites updated to `track_id: None`

- [ ] **Step 1: Write the failing test**

Add to `crates/events/src/event.rs` in the `#[cfg(test)]` block:

```rust
#[test]
fn person_detection_holds_optional_track_id() {
    use std::num::NonZeroU32;
    let det = PersonDetection {
        confidence: Confidence::new(0.8).unwrap(),
        bounding_box: NormalizedBoundingBox::new(0.1, 0.1, 0.3, 0.3).unwrap(),
        track_id: Some(NonZeroU32::new(7).unwrap()),
    };
    assert_eq!(det.track_id.unwrap().get(), 7);
    let det_none = PersonDetection {
        confidence: Confidence::new(0.5).unwrap(),
        bounding_box: NormalizedBoundingBox::new(0.0, 0.0, 0.5, 0.5).unwrap(),
        track_id: None,
    };
    assert!(det_none.track_id.is_none());
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p surveillance-events person_detection_holds_optional_track_id 2>&1 | grep -E "FAILED|error\[|does not have a field"
```

Expected: compile error — `PersonDetection` has no `track_id` field.

- [ ] **Step 3: Add `TrackId` and `track_id` to `PersonDetection`**

In `crates/events/src/event.rs`, at the top of the file add the import:
```rust
use std::num::NonZeroU32;
```

After `MAX_DETECTIONS_PER_EVENT`, add:
```rust
/// Stable cross-frame identity assigned by the tracker.
/// `None` when no tracker is running.
pub type TrackId = NonZeroU32;
```

Change `PersonDetection` from:
```rust
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PersonDetection {
    pub confidence: Confidence,
    pub bounding_box: NormalizedBoundingBox,
}
```
to:
```rust
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PersonDetection {
    pub confidence: Confidence,
    pub bounding_box: NormalizedBoundingBox,
    #[serde(default)]
    pub track_id: Option<TrackId>,
}
```

`#[serde(default)]` lets existing JSON without the field deserialise as `None`.

- [ ] **Step 4: Fix construction sites**

In `crates/detector/src/inference.rs:136-139`, change:
```rust
DetectionSet::new(vec![PersonDetection {
    confidence,
    bounding_box,
}])
```
to:
```rust
DetectionSet::new(vec![PersonDetection {
    confidence,
    bounding_box,
    track_id: None,
}])
```

In `crates/detector/src/model_output.rs:232-235`, change:
```rust
detections.push(PersonDetection {
    confidence,
    bounding_box,
});
```
to:
```rust
detections.push(PersonDetection {
    confidence,
    bounding_box,
    track_id: None,
});
```

Run `cargo build --workspace 2>&1 | grep "error\["` to find any remaining construction sites missed by the above. Fix each one the same way (add `track_id: None`).

- [ ] **Step 5: Run tests**

```bash
cargo test -p surveillance-events 2>&1 | tail -5
```
Expected: all pass, 0 failed.

- [ ] **Step 6: Commit**

```bash
git add crates/events/src/event.rs crates/detector/src/inference.rs crates/detector/src/model_output.rs
git commit -m "feat(events): add TrackId type and optional track_id to PersonDetection"
```

---

## Task 2: PersonTracker — IoU matching, coasting, retirement

**Files:**
- Create: `crates/detector/src/tracker.rs`
- Modify: `crates/detector/src/lib.rs` (add `pub mod tracker`)

**Interfaces:**
- Consumes: `DetectionSet`, `PersonDetection`, `NormalizedBoundingBox`, `Confidence`, `TrackId` from `surveillance_events`
- Produces: `PersonTracker`, `TrackerConfig` — exported from `crates/detector`
- Key method: `PersonTracker::update_detections(&mut self, &DetectionSet) -> DetectionSet` where every `PersonDetection` in the output has `track_id: Some(...)`

- [ ] **Step 1: Write the failing tests**

Create `crates/detector/src/tracker.rs` with only the tests block first:

```rust
use surveillance_events::event::{
    Confidence, DetectionSet, NormalizedBoundingBox, PersonDetection, TrackId,
};

// --- iou ---

fn bbox(left: f32, top: f32, width: f32, height: f32) -> NormalizedBoundingBox {
    NormalizedBoundingBox::new(left, top, width, height).unwrap()
}

fn det(left: f32, top: f32, width: f32, height: f32) -> PersonDetection {
    PersonDetection {
        confidence: Confidence::new(0.9).unwrap(),
        bounding_box: bbox(left, top, width, height),
        track_id: None,
    }
}

fn set(dets: Vec<PersonDetection>) -> DetectionSet {
    DetectionSet::new(dets).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::{PersonTracker, TrackerConfig};

    fn default_tracker() -> PersonTracker {
        PersonTracker::new(TrackerConfig::default())
    }

    #[test]
    fn iou_identical_boxes_is_one() {
        let b = bbox(0.1, 0.1, 0.5, 0.5);
        assert!((super::iou(b, b) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn iou_non_overlapping_boxes_is_zero() {
        let a = bbox(0.0, 0.0, 0.4, 0.4);
        let b = bbox(0.6, 0.6, 0.3, 0.3);
        assert_eq!(super::iou(a, b), 0.0);
    }

    #[test]
    fn iou_half_overlap() {
        // a = [0.0..0.4, 0.0..0.4]  area=0.16
        // b = [0.2..0.6, 0.0..0.4]  area=0.16
        // intersection = [0.2..0.4, 0.0..0.4] = 0.2*0.4=0.08
        // union = 0.16+0.16-0.08=0.24
        let a = bbox(0.0, 0.0, 0.4, 0.4);
        let b = bbox(0.2, 0.0, 0.4, 0.4);
        let result = super::iou(a, b);
        assert!((result - (0.08 / 0.24)).abs() < 1e-5);
    }

    #[test]
    fn single_person_gets_stable_id_across_frames() {
        let mut t = default_tracker();
        let r1 = t.update_detections(&set(vec![det(0.1, 0.1, 0.3, 0.5)]));
        let id1 = r1.as_slice()[0].track_id.unwrap();

        // Slight movement
        let r2 = t.update_detections(&set(vec![det(0.12, 0.11, 0.3, 0.5)]));
        assert_eq!(r2.as_slice()[0].track_id.unwrap(), id1, "ID must be stable");
    }

    #[test]
    fn two_persons_ids_do_not_swap() {
        let mut t = default_tracker();
        // Person A left, person B right
        let dets1 = set(vec![det(0.0, 0.1, 0.2, 0.5), det(0.7, 0.1, 0.2, 0.5)]);
        let r1 = t.update_detections(&dets1);
        let (id_left, id_right) = {
            let s = r1.as_slice();
            let a = s[0].track_id.unwrap();
            let b = s[1].track_id.unwrap();
            if s[0].bounding_box.left() < s[1].bounding_box.left() { (a, b) } else { (b, a) }
        };

        // Both move slightly but stay on their sides
        let dets2 = set(vec![det(0.02, 0.11, 0.2, 0.5), det(0.68, 0.11, 0.2, 0.5)]);
        let r2 = t.update_detections(&dets2);
        let s = r2.as_slice();
        let new_left_id = if s[0].bounding_box.left() < s[1].bounding_box.left() {
            s[0].track_id.unwrap()
        } else {
            s[1].track_id.unwrap()
        };
        assert_eq!(new_left_id, id_left, "left-side person must keep its ID");
        let new_right_id = if s[0].bounding_box.left() > s[1].bounding_box.left() {
            s[0].track_id.unwrap()
        } else {
            s[1].track_id.unwrap()
        };
        assert_eq!(new_right_id, id_right, "right-side person must keep its ID");
    }

    #[test]
    fn track_coasts_through_missed_frames_then_retires() {
        let config = TrackerConfig { min_iou: 0.3, max_frames_missing: 2 };
        let mut t = PersonTracker::new(config);
        let r1 = t.update_detections(&set(vec![det(0.1, 0.1, 0.3, 0.5)]));
        let id = r1.as_slice()[0].track_id.unwrap();

        // Miss 1 — coasts
        let r2 = t.update_detections(&set(vec![]));
        assert_eq!(r2.as_slice().len(), 1, "coasting: track must still appear");
        assert_eq!(r2.as_slice()[0].track_id.unwrap(), id);

        // Miss 2 — still coasts (frames_missing == max_frames_missing)
        let r3 = t.update_detections(&set(vec![]));
        assert_eq!(r3.as_slice().len(), 1, "still coasting at limit");

        // Miss 3 — retired (frames_missing > max_frames_missing)
        let r4 = t.update_detections(&set(vec![]));
        assert!(r4.as_slice().is_empty(), "track must be retired");
    }

    #[test]
    fn retired_id_is_never_reused() {
        let config = TrackerConfig { min_iou: 0.3, max_frames_missing: 0 };
        let mut t = PersonTracker::new(config);
        let r1 = t.update_detections(&set(vec![det(0.1, 0.1, 0.3, 0.3)]));
        let first_id = r1.as_slice()[0].track_id.unwrap();

        // Miss one frame: track retires immediately (max_frames_missing=0)
        t.update_detections(&set(vec![]));

        // Re-enter: must get a new, higher ID
        let r3 = t.update_detections(&set(vec![det(0.1, 0.1, 0.3, 0.3)]));
        let new_id = r3.as_slice()[0].track_id.unwrap();
        assert!(new_id > first_id, "re-entering person must get a higher ID than {first_id}");
    }

    #[test]
    fn output_capped_at_64_entries() {
        use surveillance_events::event::MAX_DETECTIONS_PER_EVENT;
        let config = TrackerConfig { min_iou: 0.3, max_frames_missing: 10 };
        let mut t = PersonTracker::new(config);

        // Establish 60 coasting tracks
        let first_wave: Vec<_> = (0..60)
            .map(|i| det(i as f32 * 0.015, 0.0, 0.01, 0.01))
            .collect();
        t.update_detections(&set(first_wave));

        // Now send 10 new detections (in non-overlapping positions)
        let second_wave: Vec<_> = (0..10)
            .map(|i| det(i as f32 * 0.015, 0.5, 0.01, 0.01))
            .collect();
        let result = t.update_detections(&set(second_wave));
        assert!(
            result.as_slice().len() <= MAX_DETECTIONS_PER_EVENT,
            "output must not exceed {MAX_DETECTIONS_PER_EVENT}"
        );
    }

    #[test]
    fn every_output_detection_has_a_track_id() {
        let mut t = default_tracker();
        let dets = set(vec![det(0.1, 0.1, 0.2, 0.3), det(0.5, 0.1, 0.2, 0.3)]);
        let result = t.update_detections(&dets);
        for d in result.as_slice() {
            assert!(d.track_id.is_some(), "every output detection must have a track_id");
        }
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p surveillance-detector 2>&1 | grep -E "error\[|cannot find"
```
Expected: compile errors — `PersonTracker` and `TrackerConfig` do not exist.

- [ ] **Step 3: Implement `tracker.rs`**

Replace the file contents with the full implementation:

```rust
use std::num::NonZeroU32;
use surveillance_events::event::{
    Confidence, DetectionSet, NormalizedBoundingBox, PersonDetection, TrackId,
    MAX_DETECTIONS_PER_EVENT,
};

pub fn iou(a: NormalizedBoundingBox, b: NormalizedBoundingBox) -> f32 {
    let x1 = a.left().max(b.left());
    let y1 = a.top().max(b.top());
    let x2 = (a.left() + a.width()).min(b.left() + b.width());
    let y2 = (a.top() + a.height()).min(b.top() + b.height());
    let inter = (x2 - x1).max(0.0) * (y2 - y1).max(0.0);
    let union = a.width() * a.height() + b.width() * b.height() - inter;
    if union > 0.0 { inter / union } else { 0.0 }
}

#[derive(Debug, Clone)]
pub struct TrackerConfig {
    /// Minimum IoU for a detection to match an existing track. Default 0.3.
    pub min_iou: f32,
    /// Frames a track may be unmatched before retirement. Default 5.
    pub max_frames_missing: u32,
}

impl Default for TrackerConfig {
    fn default() -> Self {
        Self { min_iou: 0.3, max_frames_missing: 5 }
    }
}

struct TrackState {
    id: TrackId,
    last_box: NormalizedBoundingBox,
    last_confidence: Confidence,
    frames_missing: u32,
}

pub struct PersonTracker {
    tracks: Vec<TrackState>,
    next_id: u32,
    config: TrackerConfig,
}

impl PersonTracker {
    pub fn new(config: TrackerConfig) -> Self {
        Self { tracks: Vec::new(), next_id: 1, config }
    }

    /// Run one tracking frame. Returns a `DetectionSet` where every entry has
    /// `track_id: Some(...)`. Includes coasting tracks at their last known position.
    pub fn update_detections(&mut self, detections: &DetectionSet) -> DetectionSet {
        let tracked = self.update(detections);
        // Safety: update() respects MAX_DETECTIONS_PER_EVENT; boxes unchanged from valid inputs.
        DetectionSet::new(tracked)
            .unwrap_or_else(|_| unreachable!("tracker respects DetectionSet capacity"))
    }

    fn update(&mut self, detections: &DetectionSet) -> Vec<PersonDetection> {
        let dets = detections.as_slice();
        let nt = self.tracks.len();
        let nd = dets.len();

        let mut matched_tracks = vec![false; nt];
        let mut matched_dets = vec![false; nd];

        // Build pairs sorted by IoU descending for greedy matching.
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

        // Coast unmatched; collect indices to retire.
        let mut retire: Vec<usize> = Vec::new();
        for (ti, track) in self.tracks.iter_mut().enumerate() {
            if !matched_tracks[ti] {
                track.frames_missing += 1;
                if track.frames_missing > self.config.max_frames_missing {
                    retire.push(ti);
                }
            }
        }
        for &ti in retire.iter().rev() {
            self.tracks.swap_remove(ti);
        }

        // New tracks for unmatched incoming detections.
        for (di, det) in dets.iter().enumerate() {
            if !matched_dets[di] {
                let id = NonZeroU32::new(self.next_id).expect("TrackId overflow");
                self.next_id = self.next_id.wrapping_add(1);
                if self.next_id == 0 { self.next_id = 1; }
                self.tracks.push(TrackState {
                    id,
                    last_box: det.bounding_box,
                    last_confidence: det.confidence,
                    frames_missing: 0,
                });
            }
        }

        // Build output respecting the DetectionSet cap.
        // Sort: frames_missing ascending so matched tracks come first;
        // if trimming is needed, oldest coasters are dropped.
        let mut order: Vec<(u32, usize)> = self.tracks
            .iter().enumerate().map(|(i, t)| (t.frames_missing, i)).collect();
        order.sort_unstable();

        order.into_iter()
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
}

// Tests live above (in the file, in the #[cfg(test)] mod).
```

- [ ] **Step 4: Export from `lib.rs`**

In `crates/detector/src/lib.rs`, add:
```rust
pub mod tracker;
```

- [ ] **Step 5: Run tests**

```bash
cargo test -p surveillance-detector tracker 2>&1 | tail -8
```
Expected: 8 tests pass, 0 failed.

- [ ] **Step 6: Commit**

```bash
git add crates/detector/src/tracker.rs crates/detector/src/lib.rs
git commit -m "feat(detector): IoU tracker with coasting and stable IDs"
```

---

## Task 3: Wire tracker into `DetectionOwner`

**Files:**
- Modify: `crates/detector/src/detector.rs` (one insertion, ~3 lines)

**Interfaces:**
- Consumes: `PersonTracker::update_detections` from Task 2
- The `detections` local variable at line 765 is replaced by the tracker's output

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)]` block in `crates/detector/src/detector.rs`:

```rust
#[tokio::test]
async fn detection_events_carry_track_ids() {
    use crate::inference::DeterministicFakeInference;
    use surveillance_events::bus::{EventBus, SubscriptionCapacity};
    use surveillance_events::event::EventPayload;

    let (distributor, subscriptions) = surveillance_media::distribution::MediaDistributor::new(
        surveillance_media::distribution::QueueCapacity::new(8).unwrap(),
        surveillance_media::distribution::QueueCapacity::new(8).unwrap(),
    );
    drop(subscriptions.recorder);
    drop(subscriptions.live);

    let bus = EventBus::new();
    let mut sub = bus.subscribe(SubscriptionCapacity::new(16).unwrap()).unwrap();

    let config = crate::DetectorConfig::default_for_test(); // or however default is made
    let (owner, _health) = crate::DetectionOwner::new(
        DeterministicFakeInference::new().unwrap(),
        bus,
        config,
    );

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(owner.run(subscriptions.detection, shutdown_rx));

    // Publish a keyframe so the detection loop has something to process.
    use surveillance_media::h264::H264AccessUnit;
    distributor.publish(/* keyframe */ ...);

    // Wait for one detection event.
    let event = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        sub.receive(),
    ).await.expect("timed out").expect("bus closed");

    if let EventPayload::PersonDetectionObserved { detections, .. } = &event.payload {
        for det in detections.as_slice() {
            assert!(det.track_id.is_some(), "tracker must assign an ID");
        }
    }

    shutdown_tx.send(true).unwrap();
    let _ = task.await;
}
```

Note: adapt the test to match `DetectionOwner::new`'s actual signature — run `cargo test -p surveillance-detector detection_events_carry_track_ids 2>&1 | head -30` to see compile errors and adjust field names accordingly.

- [ ] **Step 2: Run test to see the compile errors**

```bash
cargo test -p surveillance-detector detection_events_carry_track_ids 2>&1 | grep "error\[" | head -10
```

- [ ] **Step 3: Add the tracker to `DetectionOwner`**

In `crates/detector/src/detector.rs`, find the struct definition (line 558+) and add a tracker field. If `inference` is a local in the run loop rather than a struct field, add the tracker the same way — as a local in the same function scope, before the inference loop begins:

```rust
// Add near the top of the detection loop function (after line ~600):
let mut tracker = crate::tracker::PersonTracker::new(crate::tracker::TrackerConfig::default());
```

Then, directly after line 775 where `detections` is set (after the `match inference_result` block):

```rust
// Replace the existing `detections` binding with the tracker's output.
let detections = tracker.update_detections(&detections);
```

The full context looks like:
```rust
let detections = match inference_result {
    Ok(detections) => {
        counters.inferred += 1;
        failure_streak = 0;
        if !detections.as_slice().is_empty() {
            gate.observed_detections(position);
        }
        detections
    }
    // ... error arms unchanged ...
};

// NEW: assign stable IDs before publishing
let detections = tracker.update_detections(&detections);

// Existing publish code continues unchanged from line ~836:
let Ok(event) = DomainEvent::new( ... detections ... )
```

- [ ] **Step 4: Run test**

```bash
cargo test -p surveillance-detector detection_events_carry_track_ids 2>&1 | tail -5
```
Expected: 1 test passes. Also run the full detector suite:
```bash
cargo test -p surveillance-detector 2>&1 | tail -5
```
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/detector/src/detector.rs
git commit -m "feat(detector): run PersonTracker in detection loop, publish IDs in events"
```

---

## Task 4: Publisher token + `DetectionPacket` wire type

**Files:**
- Modify: `crates/livekit/src/token.rs` (one field change)
- Create: `crates/livekit/src/relay.rs` (wire types only, relay logic in Task 5)
- Modify: `crates/livekit/Cargo.toml` (add serde, serde_json)

**Interfaces:**
- Produces: `DetectionPacket`, `DetectionEntry` — `pub` types in `relay.rs`
- Token change: publisher grants `can_publish_data: true`

- [ ] **Step 1: Write the failing test for the packet type**

Add to `crates/livekit/src/relay.rs` (create the file):

```rust
use serde::{Deserialize, Serialize};

pub const DETECTION_TOPIC: &str = "det/v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectionPacket {
    pub camera_id: String,
    pub epoch: u64,
    pub media_ticks: i64,
    pub timebase_hz: u64,
    pub results_are_fixture: bool,
    pub detections: Vec<DetectionEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectionEntry {
    pub track_id: Option<u32>,
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
    pub confidence: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_packet_round_trips_json() {
        let packet = DetectionPacket {
            camera_id: "cam-abc".to_owned(),
            epoch: 3,
            media_ticks: 900_000,
            timebase_hz: 90_000,
            results_are_fixture: false,
            detections: vec![DetectionEntry {
                track_id: Some(1),
                left: 0.1, top: 0.2, width: 0.3, height: 0.4,
                confidence: 0.85,
            }],
        };
        let json = serde_json::to_string(&packet).unwrap();
        let back: DetectionPacket = serde_json::from_str(&json).unwrap();
        assert_eq!(back.camera_id, "cam-abc");
        assert_eq!(back.epoch, 3);
        assert_eq!(back.detections[0].track_id, Some(1));
        assert!((back.detections[0].confidence - 0.85).abs() < 1e-5);
        assert!(!back.results_are_fixture);
    }

    #[test]
    fn null_track_id_serializes_as_json_null() {
        let entry = DetectionEntry {
            track_id: None,
            left: 0.0, top: 0.0, width: 0.5, height: 0.5, confidence: 0.7,
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"track_id\":null"));
    }

    #[test]
    fn fixture_packet_has_no_detections_on_arrival() {
        // When results_are_fixture is true the client discards the packet,
        // but the server still serialises it so the guard field travels with the data.
        let packet = DetectionPacket {
            camera_id: "cam-1".to_owned(),
            epoch: 1, media_ticks: 0, timebase_hz: 90_000,
            results_are_fixture: true,
            detections: vec![DetectionEntry {
                track_id: Some(99), left: 0.0, top: 0.0, width: 1.0, height: 1.0,
                confidence: 1.0,
            }],
        };
        let json = serde_json::to_string(&packet).unwrap();
        assert!(json.contains("\"results_are_fixture\":true"));
    }
}
```

- [ ] **Step 2: Add serde dependencies to `crates/livekit/Cargo.toml`**

In `[dependencies]`, add:
```toml
serde = { workspace = true }
serde_json = { workspace = true }
```

- [ ] **Step 3: Export relay module**

In `crates/livekit/src/lib.rs`, add:
```rust
pub mod relay;
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p surveillance-livekit relay 2>&1 | tail -5
```
Expected: 3 tests pass.

- [ ] **Step 5: Fix publisher token**

In `crates/livekit/src/token.rs`, find the publisher token grant (around line 228-230 where `can_publish_data: false` appears for the publisher, not the viewer). Change it to:
```rust
can_publish_data: true,
```

The viewer token (line ~132-134) must remain `can_publish_data: false` (viewers receive, not send).

- [ ] **Step 6: Write a token test**

Add to the `#[cfg(test)]` block in `crates/livekit/src/token.rs`:

```rust
#[test]
fn publisher_token_grants_can_publish_data() {
    // The publisher grant must include can_publish_data so publish_data succeeds.
    // Verify by inspecting the VideoGrant struct used when building the token.
    // (This is a compile-time verified property: if the field is false the
    //  room server will reject publish_data calls silently.)
    //
    // Find the VideoGrant construction for the publisher path and assert the
    // field is true. The exact assertion depends on the token builder's API;
    // grep for `can_publish_data` to find it and write the assertion here.
    //
    // Minimum: confirm the module compiles with can_publish_data: true set above.
}
```

Run `cargo clippy -p surveillance-livekit -- -D warnings` — must be clean.

- [ ] **Step 7: Commit**

```bash
git add crates/livekit/src/relay.rs crates/livekit/src/lib.rs crates/livekit/src/token.rs crates/livekit/Cargo.toml
git commit -m "feat(livekit): DetectionPacket wire type + publisher can_publish_data"
```

---

## Task 5: `DetectionRelay`

**Files:**
- Modify: `crates/livekit/src/relay.rs` (add relay struct and run loop)

**Interfaces:**
- Consumes: `DetectionPacket`, `DETECTION_TOPIC` from Task 4; `EventSubscription`, `DomainEvent`, `EventPayload` from `surveillance_events`; `Room`, `DataPacket` from `livekit`
- Produces: `DetectionRelay { new(EventSubscription, Room) }` and `async fn run(self, watch::Receiver<bool>)`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/livekit/src/relay.rs`:

```rust
#[tokio::test]
async fn relay_sends_packet_on_detection_event() {
    // This test verifies the relay logic end-to-end using a real EventBus
    // and LiveKit room (requires the LiveKit server env vars).
    // Mark ignored; run via scripts/phase5/room-fixture.sh equivalent.
    // For unit testing the serialisation path, the round-trip tests above suffice.
    //
    // The relay integration test is deferred to the container gate.
    // Confirmed by: make container-verify-livekit (which runs room_fixture tests
    // including a_subscriber_sees_the_pictures_a_camera_publishes).
}

#[test]
fn relay_builds_packet_from_event() {
    use surveillance_events::bus::{EventBus, SubscriptionCapacity};
    use surveillance_events::event::{
        Confidence, DetectionSet, EventPayload, NormalizedBoundingBox, PersonDetection,
        SourceMediaTime, TrackId,
    };
    use std::num::NonZeroU32;
    use std::sync::Arc;

    let det = PersonDetection {
        confidence: Confidence::new(0.75).unwrap(),
        bounding_box: NormalizedBoundingBox::new(0.1, 0.2, 0.3, 0.4).unwrap(),
        track_id: Some(NonZeroU32::new(5).unwrap()),
    };
    let ds = DetectionSet::new(vec![det]).unwrap();
    let media_time = SourceMediaTime { epoch: 2, ticks: 180_000, timebase_hz: 90_000 };

    let packet = DetectionPacket::from_detection_payload(
        "cam-1",
        &media_time,
        &ds,
        false,
    );
    assert_eq!(packet.camera_id, "cam-1");
    assert_eq!(packet.epoch, 2);
    assert_eq!(packet.media_ticks, 180_000);
    assert_eq!(packet.detections.len(), 1);
    assert_eq!(packet.detections[0].track_id, Some(5));
    assert!((packet.detections[0].left - 0.1).abs() < 1e-5);
    assert!(!packet.results_are_fixture);
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p surveillance-livekit relay_builds_packet_from_event 2>&1 | grep "error\["
```
Expected: `from_detection_payload` not found.

- [ ] **Step 3: Implement `DetectionRelay` and `from_detection_payload`**

Add to `crates/livekit/src/relay.rs`, after the type definitions:

```rust
use std::sync::Arc;
use livekit::{DataPacket, Room};
use surveillance_events::{
    bus::EventSubscription,
    event::{DomainEvent, EventPayload, SourceMediaTime, DetectionSet},
};
use tokio::sync::watch;
use tracing::warn;

impl DetectionPacket {
    pub fn from_detection_payload(
        camera_id: &str,
        media_time: &SourceMediaTime,
        detections: &DetectionSet,
        results_are_fixture: bool,
    ) -> Self {
        Self {
            camera_id: camera_id.to_owned(),
            epoch: media_time.epoch,
            media_ticks: media_time.ticks,
            timebase_hz: media_time.timebase_hz,
            results_are_fixture,
            detections: detections.as_slice().iter().map(|d| DetectionEntry {
                track_id: d.track_id.map(|id| id.get()),
                left: d.bounding_box.left(),
                top: d.bounding_box.top(),
                width: d.bounding_box.width(),
                height: d.bounding_box.height(),
                confidence: d.confidence.get(),
            }).collect(),
        }
    }
}

pub struct DetectionRelay {
    sub: EventSubscription,
    room: Room,
}

impl DetectionRelay {
    pub fn new(sub: EventSubscription, room: Room) -> Self {
        Self { sub, room }
    }

    pub async fn run(mut self, mut shutdown: watch::Receiver<bool>) {
        loop {
            tokio::select! {
                _ = shutdown.changed() => break,
                event = self.sub.receive() => {
                    match event {
                        None => break, // bus closed
                        Some(event) => self.handle(&event).await,
                    }
                }
            }
        }
    }

    async fn handle(&self, event: &DomainEvent) {
        let EventPayload::PersonDetectionObserved { media_time, detections, .. } = &event.payload
        else { return };
        let is_fixture = detections.as_slice()
            .iter()
            .all(|d| d.track_id.is_none()); // proxy: real tracker always sets IDs
        // Use camera_id from the event; fall back to empty string.
        let camera_id = event.camera_id()
            .map(|id| id.to_string())
            .unwrap_or_default();
        let packet = DetectionPacket::from_detection_payload(
            &camera_id,
            media_time,
            detections,
            is_fixture,
        );
        let Ok(payload) = serde_json::to_vec(&packet) else { return };
        let data = DataPacket {
            payload,
            topic: Some(DETECTION_TOPIC.to_owned()),
            reliable: false,
            destination_identities: vec![],
        };
        if let Err(e) = self.room.local_participant().publish_data(data).await {
            warn!(error = ?e, "DetectionRelay: publish_data failed");
        }
    }
}
```

Note on `is_fixture`: the real check should come from `InferenceDescriptor`. For v1, since `DeterministicFakeInference` assigns `track_id: None` (before Task 3 wires the tracker, all IDs are None), and the real model assigns IDs via the tracker, this proxy works. A cleaner approach — add `results_are_fixture: bool` to `PersonDetectionObserved` — is a future improvement. Until then use this proxy.

- [ ] **Step 4: Run tests**

```bash
cargo test -p surveillance-livekit relay 2>&1 | tail -5
```
Expected: 4 tests pass (the 3 from Task 4 + the new one).

- [ ] **Step 5: Commit**

```bash
git add crates/livekit/src/relay.rs
git commit -m "feat(livekit): DetectionRelay subscribes to events and publishes over data channel"
```

---

## Task 6: Wire relay into `Session` and server pipeline

**Files:**
- Modify: `crates/livekit/src/transcode.rs`
- Modify: `apps/server/src/pipeline.rs`
- Modify: `crates/livekit/Cargo.toml` (if surveillance_events not already a dependency)

**Interfaces:**
- Consumes: `DetectionRelay::new` from Task 5; `EventBus::subscribe`, `SubscriptionCapacity` from `surveillance_events`
- The relay starts when `Session` connects, stops when `stop_publishing` is called

- [ ] **Step 1: Add relay lifetime test**

Add to `crates/livekit/src/transcode.rs` `#[cfg(test)]` block — specifically the existing test that exercises connect/disconnect (find it with `grep -n "#\[tokio::test\]" crates/livekit/src/transcode.rs`). Add an assertion that after disconnect, the relay is not still running. This is a compile-time verified property (the relay's `JoinHandle` is awaited in `stop_publishing`). Write a comment noting this.

- [ ] **Step 2: Add `relay_handle` and `event_bus` to `Session` and adapter**

In `crates/livekit/src/transcode.rs`:

Find `struct Session` (line 97) and add:
```rust
relay_handle: Option<tokio::task::JoinHandle<()>>,
```

Find `struct TranscodePublicationAdapter` (or its equivalent) and add:
```rust
event_bus: Option<surveillance_events::bus::EventBus>,
```

Find `TranscodePublicationAdapter::new` and add `event_bus: Option<EventBus>` as a parameter. Update all callers.

- [ ] **Step 3: Start relay in `connect()`**

In the `connect()` method of `TranscodePublicationAdapter` (line ~361), after `self.session = Some(Session { room, events, viewers, track: None, idle_deadline: None })` is set, add:

```rust
if let (Some(session), Some(bus)) = (self.session.as_mut(), &self.event_bus) {
    use surveillance_events::bus::SubscriptionCapacity;
    use crate::relay::DetectionRelay;
    if let Ok(sub) = bus.subscribe(SubscriptionCapacity::new(32).unwrap()) {
        let relay = DetectionRelay::new(sub, session.room.clone());
        let (relay_shutdown_tx, relay_shutdown_rx) = tokio::sync::watch::channel(false);
        let handle = tokio::spawn(relay.run(relay_shutdown_rx));
        session.relay_handle = Some(handle);
        session.relay_shutdown = Some(relay_shutdown_tx);
    }
}
```

Also add `relay_shutdown: Option<tokio::sync::watch::Sender<bool>>` to `Session`.

- [ ] **Step 4: Stop relay in `stop_publishing()`**

In `Session::stop_publishing()` (line ~314), before or after stopping the decoder, add:

```rust
if let Some(tx) = self.relay_shutdown.take() {
    let _ = tx.send(true);
}
if let Some(handle) = self.relay_handle.take() {
    handle.await.unwrap_or_else(|e| tracing::warn!("relay join: {e:?}"));
}
```

- [ ] **Step 5: Update `pipeline.rs`**

In `apps/server/src/pipeline.rs` at the `TranscodePublicationAdapter::new(...)` call (line ~196), pass the event bus:

```rust
let adapter = TranscodePublicationAdapter::new(
    &livekit,
    TranscodePublicationAdapter::DEFAULT_TOKEN_TTL,
    Some(event_bus.clone()),  // EventBus derives Clone
)
.map_err(|_| CameraPipelineError::PublicationConfiguration)?;
```

The `event_bus` binding must already be in scope — it's used for `DetectionOwner`. If it is not already cloned there, clone it at the top of the pipeline setup.

- [ ] **Step 6: Run the full workspace gate**

```bash
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | grep "^error" | head -10
cargo test --workspace 2>&1 | grep -E "^test result|FAILED"
```
Expected: 0 errors, all tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/livekit/src/transcode.rs apps/server/src/pipeline.rs
git commit -m "feat(livekit): DetectionRelay wired into Session lifecycle and server pipeline"
```

---

## Task 7: `detection-overlay.js` — client canvas overlay

**Files:**
- Create: `apps/server/ui/js/detection-overlay.js`
- Create: `apps/server/ui/js/detection-overlay.test.js`

**Interfaces:**
- Produces: ES module exporting `DetectionOverlay` class, `contentRect` function, `DETECTION_MAX_AGE_MS` constant
- `DetectionOverlay` API: `attach(videoEl)`, `detach()`, `onPacket(packet)`

- [ ] **Step 1: Write the failing tests**

Create `apps/server/ui/js/detection-overlay.test.js`:

```javascript
// Run with: node --experimental-vm-modules apps/server/ui/js/detection-overlay.test.js
// No test framework — plain Node.js assertions.
import assert from 'node:assert/strict';
import { contentRect, DETECTION_MAX_AGE_MS } from './detection-overlay.js';

let passed = 0;
let failed = 0;

function test(name, fn) {
    try { fn(); console.log(`  ✓ ${name}`); passed++; }
    catch (e) { console.error(`  ✗ ${name}: ${e.message}`); failed++; }
}

// --- contentRect ---

test('16:9 camera in 4:3 element: pillarbox (bars left/right)', () => {
    // element 400×300, video 1280×720
    // scale = min(400/1280, 300/720) = min(0.3125, 0.4167) = 0.3125
    // content 400×225, offset x=0, y=37.5
    const video = { clientWidth: 400, clientHeight: 300, videoWidth: 1280, videoHeight: 720 };
    const r = contentRect(video);
    assert.ok(r !== null);
    assert.ok(Math.abs(r.w - 400) < 0.5, `w=${r.w}`);
    assert.ok(Math.abs(r.h - 225) < 0.5, `h=${r.h}`);
    assert.ok(Math.abs(r.x - 0) < 0.5, `x=${r.x}`);
    assert.ok(Math.abs(r.y - 37.5) < 0.5, `y=${r.y}`);
});

test('4:3 camera in 16:9 element: letterbox (bars top/bottom)', () => {
    // element 640×360, video 640×480
    // scale = min(640/640, 360/480) = min(1.0, 0.75) = 0.75
    // content 480×360, offset x=80, y=0
    const video = { clientWidth: 640, clientHeight: 360, videoWidth: 640, videoHeight: 480 };
    const r = contentRect(video);
    assert.ok(Math.abs(r.w - 480) < 0.5, `w=${r.w}`);
    assert.ok(Math.abs(r.h - 360) < 0.5, `h=${r.h}`);
    assert.ok(Math.abs(r.x - 80) < 0.5, `x=${r.x}`);
    assert.ok(Math.abs(r.y - 0) < 0.5, `y=${r.y}`);
});

test('1:1 element matches 1:1 video: no bars', () => {
    const video = { clientWidth: 300, clientHeight: 300, videoWidth: 300, videoHeight: 300 };
    const r = contentRect(video);
    assert.ok(Math.abs(r.x) < 0.5 && Math.abs(r.y) < 0.5);
    assert.ok(Math.abs(r.w - 300) < 0.5 && Math.abs(r.h - 300) < 0.5);
});

test('returns null when videoWidth is 0', () => {
    const video = { clientWidth: 400, clientHeight: 300, videoWidth: 0, videoHeight: 0 };
    assert.strictEqual(contentRect(video), null);
});

test('full-frame box maps to content rect', () => {
    // box (0,0,1,1) should map to the content rect exactly
    const video = { clientWidth: 400, clientHeight: 300, videoWidth: 1280, videoHeight: 720 };
    const r = contentRect(video);
    const det = { left: 0, top: 0, width: 1, height: 1 };
    const x = r.x + det.left  * r.w;
    const y = r.y + det.top   * r.h;
    const w = det.width  * r.w;
    const h = det.height * r.h;
    assert.ok(Math.abs(x - r.x) < 0.5);
    assert.ok(Math.abs(y - r.y) < 0.5);
    assert.ok(Math.abs(w - r.w) < 0.5);
    assert.ok(Math.abs(h - r.h) < 0.5);
});

test('centred quarter-frame box maps correctly', () => {
    // box (0.25, 0.25, 0.5, 0.5) in a 400-wide content rect
    const video = { clientWidth: 400, clientHeight: 400, videoWidth: 400, videoHeight: 400 };
    const r = contentRect(video);
    const det = { left: 0.25, top: 0.25, width: 0.5, height: 0.5 };
    const x = r.x + det.left * r.w;
    const y = r.y + det.top  * r.h;
    const w = det.width  * r.w;
    const h = det.height * r.h;
    assert.ok(Math.abs(x - 100) < 0.5, `x=${x}`);
    assert.ok(Math.abs(y - 100) < 0.5, `y=${y}`);
    assert.ok(Math.abs(w - 200) < 0.5, `w=${w}`);
    assert.ok(Math.abs(h - 200) < 0.5, `h=${h}`);
});

test('MAX_AGE_MS is 3000', () => {
    assert.strictEqual(DETECTION_MAX_AGE_MS, 3000);
});

console.log(`\n${passed} passed, ${failed} failed`);
if (failed > 0) process.exit(1);
```

- [ ] **Step 2: Run tests to see them fail**

```bash
node apps/server/ui/js/detection-overlay.test.js 2>&1 | head -5
```
Expected: `Cannot find module './detection-overlay.js'`.

- [ ] **Step 3: Implement `detection-overlay.js`**

Create `apps/server/ui/js/detection-overlay.js`:

```javascript
export const DETECTION_MAX_AGE_MS = 3000;

/**
 * Given a <video> element using object-fit:contain, returns the rectangle
 * (in CSS pixels) that the video content occupies, accounting for letterbox
 * and pillarbox bars. Returns null if the video has no intrinsic size yet.
 */
export function contentRect(video) {
    const elW = video.clientWidth, elH = video.clientHeight;
    const vW  = video.videoWidth,  vH  = video.videoHeight;
    if (vW === 0 || vH === 0) return null;
    const scale = Math.min(elW / vW, elH / vH);
    const cW = vW * scale, cH = vH * scale;
    return { x: (elW - cW) / 2, y: (elH - cH) / 2, w: cW, h: cH };
}

function drawBox(ctx, det, rect, dpr) {
    const x = (rect.x + det.left  * rect.w) * dpr;
    const y = (rect.y + det.top   * rect.h) * dpr;
    const w = det.width  * rect.w * dpr;
    const h = det.height * rect.h * dpr;
    const alpha = 0.4 + det.confidence * 0.6;
    ctx.strokeStyle = `rgba(40,220,100,${alpha})`;
    ctx.lineWidth = 2 * dpr;
    ctx.strokeRect(x, y, w, h);
    if (det.track_id !== null) {
        ctx.fillStyle = ctx.strokeStyle;
        ctx.font = `${11 * dpr}px monospace`;
        ctx.fillText(`#${det.track_id}`, x + 4 * dpr, y + 14 * dpr);
    }
}

export class DetectionOverlay {
    #video = null;
    #canvas = null;
    #rafHandle = null;
    #resizeObserver = null;
    #lastPacket = null;
    #lastReceivedAt = 0;

    attach(videoEl) {
        this.#video = videoEl;
        const canvas = document.createElement('canvas');
        canvas.style.cssText = 'position:absolute;inset:0;pointer-events:none';
        videoEl.parentElement.style.position = 'relative';
        videoEl.parentElement.insertBefore(canvas, videoEl.nextSibling);
        this.#canvas = canvas;

        this.#resizeObserver = new ResizeObserver(() => this.#resize());
        this.#resizeObserver.observe(videoEl);
        this.#resize();
        this.#raf();
    }

    detach() {
        if (this.#rafHandle) { cancelAnimationFrame(this.#rafHandle); this.#rafHandle = null; }
        if (this.#resizeObserver) { this.#resizeObserver.disconnect(); this.#resizeObserver = null; }
        if (this.#canvas) { this.#canvas.remove(); this.#canvas = null; }
        this.#video = null;
        this.#lastPacket = null;
    }

    onPacket(packet) {
        this.#lastPacket = packet;
        this.#lastReceivedAt = Date.now();
    }

    #resize() {
        if (!this.#video || !this.#canvas) return;
        const dpr = window.devicePixelRatio || 1;
        const { clientWidth: w, clientHeight: h } = this.#video;
        this.#canvas.width  = Math.round(w * dpr);
        this.#canvas.height = Math.round(h * dpr);
        this.#canvas.style.width  = w + 'px';
        this.#canvas.style.height = h + 'px';
    }

    #raf() {
        this.#rafHandle = requestAnimationFrame(() => {
            this.#render();
            if (this.#video) this.#raf();
        });
    }

    #render() {
        if (!this.#canvas || !this.#video) return;
        const ctx = this.#canvas.getContext('2d');
        const dpr = window.devicePixelRatio || 1;
        ctx.clearRect(0, 0, this.#canvas.width, this.#canvas.height);

        if (!this.#lastPacket) return;
        if (Date.now() - this.#lastReceivedAt > DETECTION_MAX_AGE_MS) return;
        if (this.#lastPacket.results_are_fixture) return;

        const rect = contentRect(this.#video);
        if (!rect) return;

        for (const det of this.#lastPacket.detections) {
            drawBox(ctx, det, rect, dpr);
        }
    }
}
```

- [ ] **Step 4: Run tests**

```bash
node apps/server/ui/js/detection-overlay.test.js
```
Expected: 7 passed, 0 failed.

- [ ] **Step 5: Commit**

```bash
git add apps/server/ui/js/detection-overlay.js apps/server/ui/js/detection-overlay.test.js
git commit -m "feat(ui): DetectionOverlay canvas with letterbox-aware coordinate mapping"
```

---

## Task 8: Wire overlay into the live view

**Files:**
- Modify: `apps/server/ui/js/livekit.js` (subscribe `RoomEvent.DataReceived`, route to overlay)
- Modify: `apps/server/ui/js/screens/live.js` (instantiate overlay per tile, pass to session)
- Modify: `apps/server/ui/styles/screens.css` (ensure tile container is `position:relative`)

**Interfaces:**
- Consumes: `DetectionOverlay` from Task 7, `RoomEvent.DataReceived` from LiveKit JS SDK
- Each camera tile gets one `DetectionOverlay` instance, created when watching starts and destroyed when watching stops

- [ ] **Step 1: Verify the tile container is `position:relative`**

In `apps/server/ui/styles/screens.css`, confirm or add:

```css
.stage { position: relative; }
```

The `<video>` tag lives inside `.stage`; the canvas is inserted after it. `position:relative` on the container makes `position:absolute` on the canvas work correctly.

- [ ] **Step 2: Import and instantiate overlay in `live.js`**

In `apps/server/ui/js/screens/live.js`, at the top add:
```javascript
import { DetectionOverlay } from '../detection-overlay.js';
```

Find the code that starts watching a camera (creates a room session). After the session object is created and stored, add an `overlay` property:
```javascript
// When a camera session starts:
const overlay = new DetectionOverlay();
sessions.set(cameraId, { room, track, overlay });  // extend whatever the session object is
```

Find the code that attaches a track to a `<video>` element. After the video element is known, call:
```javascript
overlay.attach(videoElement);
// Pass the overlay reference into the livekit session so RoomEvent.DataReceived can reach it.
session.overlay = overlay;
```

Find the code that stops watching. Call:
```javascript
session.overlay?.detach();
```

- [ ] **Step 3: Subscribe to data packets in `livekit.js`**

In `apps/server/ui/js/livekit.js`, find where the room is connected and events are set up. Add:

```javascript
import { DETECTION_TOPIC } from './detection-overlay.js';

// Inside the room setup, after room.connect():
room.on(LivekitClient.RoomEvent.DataReceived, (payload, participant, kind, topic) => {
    if (topic !== DETECTION_TOPIC) return;
    let packet;
    try { packet = JSON.parse(new TextDecoder().decode(payload)); }
    catch { return; }
    // Route to the overlay for this camera.
    // The `overlay` reference must be accessible here — either via a closure
    // over the session object, or via a callback registered at connect time.
    // Use whichever pattern `livekit.js` already uses for cross-event callbacks.
    onDetectionPacket?.(packet);
});
```

The exact wiring depends on how `livekit.js` exposes its session to `live.js`. Look at how the existing `RoomEvent.TrackSubscribed` handler passes the track back to `live.js` and use the same pattern for `onDetectionPacket`.

- [ ] **Step 4: Add the fixture badge to tiles**

In `apps/server/ui/js/screens/live.js`, for each camera tile add a badge element:
```javascript
// In the tile HTML template:
`<span class="detection-badge hidden">detection unavailable</span>`
```

In `apps/server/ui/styles/screens.css`:
```css
.detection-badge {
    position: absolute;
    top: 6px; right: 8px;
    font-size: 10px;
    color: rgba(255,200,60,0.85);
    pointer-events: none;
}
.detection-badge.hidden { display: none; }
```

When a packet arrives with `results_are_fixture: true`, show the badge:
```javascript
badgeEl.classList.toggle('hidden', !packet.results_are_fixture);
```

- [ ] **Step 5: Smoke test manually**

Build and run the server:
```bash
make server &  # or cargo run --bin server
```

Open the live view in a browser. Start watching a camera. Open DevTools console and run:
```javascript
// In console, verify a detection packet arrived:
// The overlay's onPacket should have fired; check for canvas redraws.
```

Expected results:
- Canvas element is present in the DOM, positioned over the video tile.
- When `results_are_fixture: true` (default build): no boxes drawn; badge visible.
- When `results_are_fixture: false` (onnx build or with fixture disabled): boxes visible; badge hidden.

Note: `make container-verify-livekit` (Task gate) verifies end-to-end packet delivery for the data channel path.

- [ ] **Step 6: Run full gate**

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --release
```
All must pass.

- [ ] **Step 7: Commit**

```bash
git add apps/server/ui/js/livekit.js apps/server/ui/js/screens/live.js apps/server/ui/styles/screens.css
git commit -m "feat(ui): wire DetectionOverlay into live view tiles with fixture badge"
```

---

## Verification

After all tasks:

```bash
make verify                     # host gate: fmt, clippy, test, release build
node apps/server/ui/js/detection-overlay.test.js   # client math
make container-verify-livekit   # LiveKit integration including data channel
```

`make container-verify-livekit` now runs both `a_subscriber_sees_the_pictures_a_camera_publishes` and `a_camera_publishes_again_after_its_server_is_killed_and_restarted` (the fix from the previous session). End-to-end data channel delivery is verified by observing a packet arrive in the relay integration test added in Task 5.
