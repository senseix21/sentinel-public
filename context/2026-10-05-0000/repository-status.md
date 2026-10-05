# Repository Status — 2026-10-05 00:00 +06

Last verified by `make verify` + a webcam-driven end-to-end run on
2026-10-04 23:50 +06. The commit on `main` is `4efca75`, the merge
commit of PR #11 (`feature/detection-inference` → `main`).

## What landed in PR #11

Twenty-three commits over three days: a deterministic restart test, a
real ONNX inference path, an in-browser detection overlay, a
cross-frame tracker, per-tile person counts, four env knobs, and an
ROI exclusion mask that drops a fixed region before NMS.

### Media and detection

- `OnnxPersonInference` now runs YOLOX-Tiny (416×416) by default and
  YOLOX-S (640×640) when configured. The session is built from
  verified bytes, the input tensor is not cloned per inference, and
  execution providers are registered. Backend initialisation runs on
  the blocking pool. The detection claim, decode geometry and gate run
  off the live backend; `results_are_fixture` reports what is actually
  loaded, not what the build contains.
- `PersonTracker` assigns a stable `TrackId` across frames via greedy
  IoU matching with coasting and retirement. The tracker caps a frame
  at 64 detections, retires the oldest coasters first, and makes every
  returned `PersonDetection` carry `track_id: Some(_)`.
- `filter_detections_outside` drops a detection whose box centre falls
  inside a configured rectangle. The rectangle lives on
  `OneCameraConfig::detection_roi_exclude` and is passed through to
  `DetectionOwner::start_observed`.
- `DetectionRelay` subscribes to `PersonDetectionObserved`, serialises
  a bounded `DetectionPacket`, and publishes on the LiveKit data
  channel under topic `det/v1` with `reliable: false`. The relay is
  owned by the LiveKit session and shuts down with it.

### Live view and overlay

- `apps/server/ui/js/detection-overlay.js` composites a `<canvas>` over
  the live `<video>` with `object-fit: contain` letterbox maths. It
  renders on `requestAnimationFrame`, treats a frame older than three
  seconds as stale, and reattaches itself when the owner replaces the
  video stage during reconcile.
- The owner exposes `reattachOverlays()` and `restorePersonCounts()`,
  called after `reattachLive()` so a stage swap doesn't wipe the
  canvas or the badge.
- `setPersonCount` reports active (currently drawn boxes) and
  session-cumulative (distinct `TrackId`s since the first subscribe)
  counts per tile.
- The publisher token now carries `can_publish_data: true` so the
  server-side relay can publish detection packets.

### Config and env

- Four new env readers in `apps/server/src/runtime_config.rs`:
  `DETECTION_SCORE_THRESHOLD_PERCENT`, `DETECTION_NMS_IOU_PERCENT`,
  `DETECTION_INPUT_WIDTH`/`_HEIGHT`, and `DETECTION_ROI_EXCLUDE` (as
  `x,y,w,h` normalised floats).
- When the configured input geometry exceeds the budget, the loader
  raises `decoded_frame_max_bytes` to `width * height * 3` so YOLOX-S
  does not fail with `DecodedFrameBudgetTooSmall`.
- `scripts/dev/local-webcam-stack.sh` now respects
  `SURVEILLANCE_DETECTION_ENABLED` and `SURVEILLANCE_CARGO_FEATURES`,
  so the dev stack can bring up a detection-enabled binary.

### Classification correctness

PR #11's review raised two classification bugs. One is landed, one is
rejected with a documented reason:

- `CameraDetectionScoreThresholdPercent` and
  `CameraDetectionNmsIouPercent` are classified `PipelineRestart`
  because `OnnxPersonInference` captures `DecodeThresholds` at
  construction (`crates/detector/src/onnx.rs:134`, `:158`) and reuses
  `self.thresholds` in `infer` (:371). Reclassifying as `Immediate`
  would make the config system report "applied immediately" for a
  setting that only takes effect on restart. Make the field live
  first, then flip. The reasoning sits as a comment at the setting
  definition in `crates/config/src/classification.rs` and in test
  `captured_detection_thresholds_and_gate_settings_require_pipeline_restart`.
- `CameraDetectionMotionThresholdPercent`, `_GateCooldown`,
  `_GateHeartbeat` carry the same shape: `GatePolicy` captures them at
  construction and `MotionGate` never re-reads. The classification
  matches the code.

### Restart test

The 1-in-3 restart failure that opened the session is fixed in
`cbdb737`. The SDK takes a resume path (not a reconnect) after a hard
kill when the connection is still fresh; the fixture now sleeps past
the resume window before killing the server, and the assertion is
split between the resume case (track republished without a reconnect
event) and the reconnect case (reconnect event, then republish).

## What is NOT addressed

- Classification `Immediate` on inference and gate settings. Reviewer
  is right that these are a lie if the capture sites don't change;
  reviewer's remedy (classify as `Immediate` without the capture-site
  change) is also a lie. The code is honest; neither side of the knob
  is live yet.
- Phase 6's accuracy gate. 73.7% recall and 36.9% small-subject recall
  were measured on COCO; nobody has agreed that photographs predict
  surveillance footage. The ROI exclude helps a specific camera; it
  does not substitute for a labelled dataset.
- Phase 5 remaining items that need an in-container LiveKit server and
  `promtool`. The restart test is now deterministic; the four open
  Phase 5 recovery scenarios still are not.

## Verified commands

- `make verify` on `752df1c` and `14b5867`: `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets --all-features -- -D
  warnings`, `cargo test --workspace --all-features`, `cargo build
  --workspace --release`. All green.
- End-to-end: `local-webcam-stack.sh` with `SURVEILLANCE_CARGO_FEATURES=onnx-inference`,
  `DETECTION_MODEL_PATH=...yolox_s.onnx`, `DETECTION_INPUT_WIDTH=640`,
  `DETECTION_INPUT_HEIGHT=640`, `DETECTION_SCORE_THRESHOLD_PERCENT=85`,
  `DETECTION_ROI_EXCLUDE=0.0,0.0,0.35,1.0`. Overlay drew one box per
  person, person-count badge counted distinct `TrackId`s, clothes and
  fan false positives dropped.

## Not verified in this session

- Representative-camera accuracy. Only a laptop webcam and MediaMTX
  were on the loop; a doorway at twenty metres has not been seen.
- Multi-camera concurrency with a real model. The dev stack runs one
  camera.
- Alert rule firing against `promtool`. Still blocked on the
  development image.
