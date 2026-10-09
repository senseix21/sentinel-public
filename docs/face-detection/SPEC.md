# Face-only live detection — specification

Date: 2026-10-09
Status: implemented local-development slice; production accuracy qualification
remains open. Face-only boxes and the 70% default are user-confirmed. This document supersedes the person-box rendering
requirement for the requested face mode, not the existing person event contract.

Follow-up proposal: [continuous person presence with face-only overlays](../person-presence/SPEC.md).
It addresses back-facing people without changing the currently implemented mode.

## 1. Outcome and scope

The live dashboard draws a green box around each detected **face**, excluding the
shoulders and torso. This requires real face localization. Cropping the upper part
of a YOLOX person box is not face detection and cannot satisfy this requirement.

The score threshold defaults to **70%**, inclusive: a face score below 0.70 is
rejected; a score equal to 0.70 is eligible. Model confidence is a score, not a
claim of 70% accuracy. No change to recording pixels or recorded files is needed.

“Square” is interpreted as the existing outline box: an axis-aligned rectangle
following the face detector's bounds. Forcing equal width and height is not required
and can add background. A strictly square display style would be a separate,
reversible presentation option if requested.

The first slice detects visible faces in the live view. Recognition, names,
identity matching, embeddings, enrollment, face crops, demographic inference,
face-blurring, historical detection search, and occupancy analytics are outside
this slice. Face boxes and optional landmarks used internally for decoding remain
in memory; this feature adds no face-image or biometric database.

## 2. Verified starting point

- `crates/detector/src/inference.rs` exposes `PersonInference`; the real adapter
  is ONNX YOLOX in `onnx.rs`. YOLOX class 0 produces person bounds, not face bounds.
- `DetectionOwner` owns decoding, sampling, gating, inference, tracking, and event
  publication. Encoded recording and live publication are independent consumers.
- `crates/events/src/event.rs` has `PersonDetection`, `DetectionSet` (maximum 64),
  and `PersonDetectionObserved`. These names carry person semantics.
- `crates/livekit/src/relay.rs` sends person detections on `det/v1`; the browser
  overlay uses normalized source-frame coordinates and `object-fit: contain`.
- The person tracker includes coasted boxes. The browser currently clears packets
  after three seconds without receipt. Neither behavior proves a face is visible.
- Live video uses the adapter clock; detections carry source media time. Existing
  delivery does not support frame-accurate alignment.
- Compiled and versioned confidence defaults are now 70%; explicit overrides
  remain valid. Existing person-model accuracy evidence does not qualify a face model.

## 3. Architecture and ownership

Preserve the singular camera connection and existing media distributor:

```text
Camera -> ingestion -> encoded distributor -> recorder
                                        |-> LiveKit video
                                        `-> detection owner
                                             decode/sample/preprocess
                                             face inference
                                             FaceDetectionObserved
                                                -> event bus
                                                -> owned LiveKit relay
                                                -> browser face overlay
```

For the initial implementation, select **one detection mode per camera**, `person`
or `face`. Face mode is the local launcher's new default after the face adapter is
qualified. Person mode remains explicit and retains existing semantics. Running
both models simultaneously is deferred; it must not be introduced through a second
upstream connection or decoder. Do not pass face output through `PersonInference`
or emit faces as `PersonDetectionObserved`.

Reuse the owner, decoder, sampling, cancellation and bounded execution machinery.
Add a narrow face inference boundary and explicit mode dispatch at composition.
Shared mechanics may be extracted only where both real paths need them. Model
input geometry must come from the selected adapter: reusing the existing
416x416 person preprocessing without validation can erase small-face detail.

Each camera owns its model/session and task lifetime. ONNX execution remains off
Tokio workers under the existing bounded blocking boundary. Model startup failure
makes face detection unavailable while recording and live video continue; no
silent switch to fake or person boxes. No detached relay or inference tasks.

## 4. Model qualification gate

Select a face-specific ONNX model through a small local spike before integrating.
Record official artifact source, SHA-256, redistribution/license terms, runtime
compatibility, input dimensions/layout/color order/normalization/padding, output
schema, score activation, bounding-box convention and NMS behavior. YuNet 2023mar is the reversible local model selected by
[ADR 016](../adr/016-face-only-live-detection.md), with a verified 640x640 contract.
Do not guess a replacement model's decoder from a similarly named model.

Compare candidates on the available CPU using consented or redistributable labeled
frames: close frontal face, full-body scene with a smaller face, side/profile,
multiple faces, occlusion, low light, no-face negatives, and frame edges. Include
source resolution and face size in pixels. Measure precision/recall at a stated
IoU, inference p50/p90/p99, preprocessing time and peak memory at the 70% threshold.
Record misses and false positives, not just successful screenshots. Numeric
production accuracy/latency criteria and minimum usable face size remain open.
This gate blocks a production claim, not deterministic contract work.

## 5. Contracts and delivery

Add distinct validated `FaceDetection`, `FaceDetectionSet` (cap 64), and
`FaceDetectionObserved`. Each event carries camera identity, source media time and
connection epoch, observation time, model name/version, fixture provenance and
normalized face boxes with confidence. Coordinates refer to the original camera
frame, origin top-left: finite left/top in [0,1], strictly positive width/height,
and right/bottom at most 1. Clip a valid partly visible prediction to the frame;
reject empty, non-finite or malformed output. Invert preprocessing exactly once.

Keep `PersonDetectionObserved` and `det/v1` unchanged. Introduce a separate proposed
wire topic `face/v1` containing `camera_id`, epoch and sequence encoded as decimal
strings (avoid JavaScript u64 precision loss), source media ticks/timebase, fixture
flag and up to 64 face entries (`left`, `top`, `width`, `height`, `confidence`).
Set an explicit serialized packet bound of 16 KiB; enforce it before send and
before browser parsing. The camera-scoped relay subscribes only to authorized
room camera events. Viewer tokens remain subscribe-only.

Use the existing lossy detection delivery class, dropping oldest on overflow and
counting drops. Latest state wins. Advance a monotonic per-epoch sequence for
emitted face observations; reject reordered/duplicate packets and reset state on
stream epoch/session replacement. Accept the epoch established by the current
stream/session; delayed packets from an older epoch cannot restore old boxes.

The browser retains at most one packet and 64 boxes per tile. Source timestamps
are diagnostic, not frame alignment claims. Empty results clear boxes immediately.
For v1, draw only actual detections: no coasted or guessed face boxes and no
identity labels. Clear on fixture/unavailable state, stop, disconnect or epoch
change. A configurable overlay age limit defaults to the current three seconds;
receipt-age expiry is only a fallback after lost packets, not evidence of current
face presence. Report sampling/gating gaps and visible overlay lag honestly.

## 6. Configuration and operator behavior

Proposed camera settings: detection mode, face artifact path and SHA-256, model
input configuration validated against its descriptor, face confidence percent
(default 70), NMS IoU, sample FPS, queue/frame limits and overlay maximum age.
Keep person and face model/threshold overrides distinct; the existing person
model override must not be interpreted as a face model. Reuse existing applicable
sample/queue controls rather than inventing duplicate knobs.

Register settings in the validated layered configuration system and audit changes
without logging secret values or frame content. Mode/model/geometry/threshold
changes apply on pipeline restart; expose that classification truthfully. Validate
multi-field artifact changes transactionally. Size decoded-frame budgets for the
qualified geometry before constructing a pipeline.

In face mode the UI shows “Faces detected” for the current qualified packet, not
“People” or unique-person totals. Faces are not a proxy for people who turn away,
and no cumulative unique-face count is offered. Show face detection unavailable
when disabled, fixture-backed or failed, separately from live video availability.
The downloaded default is checksum-pinned and cached using the existing launcher
pattern; downloads must remain optional when detection is disabled.

## 7. Bounds, failure and observability

Before implementation, inventory every existing decoder/frame/event/relay queue
with its configured numeric capacity, drop policy and metric. Preserve them or
justify changed values with measured frame geometry; no new unbounded queue is
allowed. The owner processes one frame at a time; stale frames are dropped, not
accumulated. Bound raw model candidates and scratch buffers from the artifact
shape, and bound output to 64 faces after thresholding and NMS. Count output
truncation and malformed results. Reuse bounded-cardinality counters/latency
histograms and add model failure, face result/drop and relay oversize counters
where existing measurements do not cover them. Never label metrics by face ID,
coordinates, artifact path or event sequence.

Model load, malformed tensor, inference timeout, relay loss and slow viewers must
not stop recording or unrelated cameras. Shutdown signals and joins the owner,
decoder and relay; no packets from the prior session remain renderable. Preserve
typed retryable/terminal failures and redact internal paths in API errors.

## 8. Acceptance criteria

1. A labeled face fixture yields a face-localized box; a full-body fixture yields
   no body box in face mode. No-face input emits an empty set and clears the UI.
2. Exact score-boundary tests cover below/equal/above 70%, and explicit overrides.
3. Model preprocessing/inverse tests cover aspect ratios, edges, padding, scaling,
   finite bounds and malformed tensors. Browser tests cover bars, resize and DPR.
4. Person events/packets remain compatible; face data is never counted as people.
5. Fixture, wrong-camera, malformed/oversize, reordered, stale and old-epoch packets
   never render; disconnect and restart clear prior boxes.
6. Face failures and overload preserve recording, one ingestion session per camera,
   bounded memory and graceful task shutdown in integration tests.
7. A real face model publishes through LiveKit into the dashboard, with separate
   Chromium and Firefox evidence. Existing Firefox connection problems remain
   independent blockers; mocked browser tests cannot close this gate.
8. Qualification records model/hash, workload, hardware, score threshold and
   limitations. Production accuracy and camera capacity remain unclaimed until
   representative labeled data, criteria and scale measurements exist.

## 9. Decisions still open

Model/artifact and license; deployment accelerator; supported face size/pose and
lighting; quantitative accuracy/latency goals; whether a stricter age limit is
needed after measured overlay lag. The proposed one-mode MVP, rectangle styling,
no tracking labels and separate face event/wire contracts are reversible design
choices. Model and mode tradeoffs are recorded in ADR 016; deployment qualification
remains a separate open gate.
