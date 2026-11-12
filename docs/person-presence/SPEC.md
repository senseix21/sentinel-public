# Continuous person presence with face-only overlays

Date: 2026-10-09
Status: implementation authorized; local development slice implemented. Production
qualification remains open. See VERIFICATION.md for evidence and limitations.

## 1. Outcome and scope

A person turning away from the camera must remain detectable through a person
model. Visible faces receive tight face rectangles. When a face disappears,
its rectangle clears while independently observed person presence continues.
A back-facing person entering the scene must be detectable without ever showing
a face. Stationary people must continue to receive periodic person observations.

This adopts the recommended interpretation of the discussion: face-only boxes
plus a person-presence readout. It preserves the user's 70% face threshold.
Head boxes, body boxes, recognition, names, embeddings, cross-camera identity,
unique visitor totals, entry/exit rules, heatmaps and durable event history are
outside this slice. Head/body overlays would require a separate product choice.

“Detected” means model evidence in a successful recent observation. Tracking
provides temporary continuity with explicit uncertainty; it cannot make an
invisible face visible or guarantee that a person is still present. Camera-local
track IDs are temporary associations and must not be presented as identities.

This is an enterprise qualification path, not an enterprise certification. The
implementation must prove the contracts below; supported camera counts and
accuracy require the declared workload and measured results.

## 2. Current implementation and required changes

Verified by source inspection:

- `DetectionMode` selects either person or face, never both. `InferenceResults`
  has separate result variants; the local launcher selects face.
- YOLOX and YuNet adapters exist. Their deployment accuracy is unqualified.
- The detection owner owns decode, sample, motion gate, inference and tracking.
  The current decoder produces model-sized letterboxed RGB, not source-sized RGB.
- The existing IoU tracker coasts using missed-frame counts, emits coasted boxes
  in person results and caps output. Its internal track collection can grow
  beyond the output cap; combined mode must explicitly bound internal state.
- Default sampling is 5 FPS; the motion gate heartbeat is 10 seconds. Those
  settings do not establish continuous, fresh presence.
- Face events and `face/v1` already preserve face semantics, bounds and provenance.
  The browser currently renders one selected mode per camera.
- Detection uses source media timestamps; live transcode uses its adapter clock.
  Current boxes cannot claim exact displayed-frame synchronization.

The new `person_face` mode replaces the one-mode restriction only for this
feature. Existing `person` and `face` modes remain explicit rollback choices.
[ADR 016](../adr/016-face-only-live-detection.md) stays historical; the proposed
successor is [ADR 017](../adr/017-person-presence-with-face-overlays.md).

## 3. Observable behavior

| Input / health | Person readout | Face overlay |
|---|---|---|
| Fresh front-facing person observation | People observed: N | Actual detected faces only |
| Fresh side/back-facing person observation | People observed: N | Only any actually visible faces |
| Brief successful observations without a match | People observed: 0; temporarily uncertain: N | Empty face result clears immediately |
| Reacquired match within grace period | Observed again; retain track if association is justified | New actual face boxes only |
| Grace interval expires | Remove uncertain track | No old face box |
| Fresh successful empty scene, no retained tracks | No people observed | No boxes |
| Person model failure or stale person output | Person detection unavailable / stale | Face lane may continue independently |
| Face model failure or stale face output | Fresh person readout continues | Clear boxes; face detection unavailable |
| Decoder/source failure | Both capabilities unavailable / stale | Clear boxes |

A successful model returning an empty set differs from a model that did not run.
Unavailable/stale data must never be converted to a fresh zero-person claim.
Counts are observations of this camera's selected detection area; they are not
certified physical occupancy. At capacity show “at least N” / saturation, since
truncated results cannot establish an exact count. Face count and person count
are independent; partial bodies or different schedules can make them disagree.
Do not attach a face to a person by a guessed body crop. Association is not
needed for the first slice, so it is omitted.

## 4. Ownership, data and processing topology

```text
one camera ingestion -> bounded encoded distributor -> recorder
                                                  -> LiveKit video
                                                  -> one detection decode owner
                                                       shared sampled RGB
                                                       |-> person lane -> presence tracker
                                                       `-> face lane
                                                        -> bounded event snapshots
                                                        -> camera-scoped live relay/UI
```

The detector owns a supervisor, one decoder, sampled frame identity, two model
lanes and the person tracker. Each lane exclusively owns its mutable model
session and preprocessing scratch. The person-result owner exclusively mutates
tracking state. Frames are immutable, reference-counted media values, never
placed on the domain event bus. No second upstream connection or detection
FFmpeg decode process is allowed. The existing live consumer's codec-driven
transcode remains a separate documented boundary.

The decode contract must represent a canonical aspect-preserving RGB frame and
its validated source-to-canonical transform. Each model independently derives
its required tensor geometry, color order, normalization and inverse transform.
Do not reuse YuNet BGR tensors for YOLOX or assume their input dimensions match.
For the first spike, use the existing 640×640 canonical letterbox, explicitly
measuring the effect of deriving the smaller person input from it. Qualify this
against source-resolution preprocessing; reject the shortcut if it violates
small-person/face acceptance. Source-resolution or alternate canonical geometry
is a configurable measured decision, with a revised byte budget before startup.

Each admitted sample has a camera, detection session, stream epoch, sample
sequence, source timestamp, geometry and monotonic admission time. Model results
retain that identity. Each lane may skip samples independently; the consumer
must never assume its latest person and face results describe the same frame.
Presence processing rejects old session/epoch/sample results before mutation.

## 5. Scheduling, bounds and overload

Combined presence is an always-on sampled capability independent of viewers.
Person inference must not depend on face visibility or motion admission. Face
inference also runs on its declared periodic schedule for the first slice.
Motion gating is disabled in combined mode; later optimization requires proof
that stationary-person freshness and detection-delay limits are maintained.

Provisional development schedule: 5 person samples/second and 5 face samples/
second, individually configurable and no greater than the canonical sample rate.
These are offered rates, not promised completed throughput. Source media time
selects samples; monotonic runtime time measures queue age and liveness. UTC
clock changes must not extend track lifetimes.

| Boundary | Required bound and overload policy |
|---|---|
| Encoded subscription | Retain current validated packet/byte caps; detection favors freshness and resynchronizes on a random-access frame after loss |
| Decode owner | One active decode; existing command queue capacity 2; inventory child pipe, timestamp and frame buffering before changing it |
| Canonical admission | At most one frame being dispatched; no retained history |
| Each inference lane | One in-flight operation plus one replaceable pending frame; overwrite pending oldest, count drops |
| Lane results | Latest-value slot capacity 1 per lane; superseded observations counted |
| Person tracker | At most 64 total tentative/observed/coasting tracks, including internal state |
| Each detection result | At most 64 validated boxes; bounded artifact-specific candidate arrays and scratch |
| Live packet | At most 16 KiB; person and face packets each independently bounded |
| Browser tile | One latest person snapshot, one latest face snapshot; at most 64 entries of each kind |

Admission and lane references retain at most five distinct sampled frame buffers
(one dispatch plus two pending and two in flight). Decoder workspace/output and
model-specific tensor scratch are additional costs. At 640×640 RGB, five full
buffers cost at most 6,144,000 bytes before allocator overhead. This is not total
process memory: include decoder buffers, FFmpeg/ORT allocations, model weights,
thread stacks, encoded queues and session workspaces in measured peak RSS.

At saturation, deterministic tracker admission favors fresh matched tracks,
then qualified new observations over oldest uncertain tracks. Every eviction
and omitted observation increments a counter. Never allocate an unbounded
association matrix: with 64 tracks and 64 candidates, at most 4,096 pairs.

Use configured global inference concurrency and decoded-byte budgets across
cameras, with fair camera admission and bounded waits. Reserve work for the
person lane before optional face work when capacity is exhausted. Global limits
must be typed, validated configuration, not hidden constants. A slow camera/lane
cannot block another camera's media or grow a waiting task collection.

## 6. Presence state and narrow Rust contracts

Design public APIs around evidence before selecting a tracking library. Required
conceptual values (names may change during implementation):

- `DetectionSessionId`, `SampleId`, bounded `TrackKey` (session + nonzero ID).
- Validated `Observation<T>` containing sample identity and real/fixture provenance.
- `LaneOutcome<T>`: successful observation (including empty), typed failure, or
  skipped work. Failure/skipping cannot construct an empty observation.
- `TrackEvidence`: `Tentative`, `Observed`, `Coasting` with last-observed time and
  expiry; retired entries leave the collection.
- `PresenceSnapshot`: observed confirmed tracks, tentative count, uncertain tracks,
  sample identity, completeness/saturation and lane availability.
- Capability health independent of model provenance. “Real inference” is not a
  type-level claim of production qualification.

Constructors validate cardinality, timestamps, finite normalized bounds and state
relationships; fields stay private where mutation could violate those invariants.
No boolean combination may represent both a current observation and a prediction.
Library failures use specific error types; paths/tokens/frame content stay out
of external errors. Traits exist at model/test substitution boundaries, not
around every state holder.

Provisional development rules:

1. A qualified new person detection starts a tentative track. Confirm after two
   associated successful person samples within one second. A one-sample false
   positive must not immediately become confirmed presence.
2. A confirmed track matched in a successful recent sample is observed. A missed
   match makes it coasting; its last confidence/box remains historical evidence,
   never a newly observed detection.
3. Grace defaults to one second since last observed match, measured by monotonic
   admission time. Tentative tracks expire after one second without confirmation.
   Timer-driven expiry continues when results stop; sample counts cannot keep
   tracks alive indefinitely during overload.
4. Successful empty observations can move tracks into uncertainty; once all
   expire, the next fresh snapshot can report no people observed. Lane failure
   or freshness expiry changes availability, so absent results do not prove exit.
5. Reacquisition within grace retains a track only if association gates pass.
   Reset all tracking on stream epoch, model/config restart or decoder reset.
   ID exhaustion ends the session and resets with a new session ID; never wrap
   and silently reuse a key within one session.

Use the existing IoU matcher as a measured baseline. Compare a motion-aware
association method such as ByteTrack on crossings and occlusions before adopting
it. Its lower-score recovery threshold must differ from the threshold for
starting tracks; it cannot create confirmed presence from arbitrary low scores.
Any change requires pinned implementation/license, algorithm bounds, deterministic
tests and evidence of fewer fragmented/swapped tracks on our footage.

ROI filtering keeps the current normalized-coordinate center convention for
compatibility. Apply independently to person and face observations; do not claim
that a face visible outside an ROI proves presence inside it. New occupancy or
entry/exit semantics require separate requirements.

## 7. Events, wire compatibility and UI

Keep existing person and face event contracts/topics compatible. Combined mode
adds an explicit `PersonPresenceObserved` snapshot and versioned `presence/v1`
wire schema; do not retrofit prediction semantics into `FaceDetectionObserved`
or change `det/v1` for existing consumers. Face output remains `face/v1`.

Presence payload contains camera/session/epoch/sequence, sample media time,
provenance, observed/uncertain/tentative counts, at most 64 track evidence entries,
completeness and capability state. Encode u64 identifiers/ticks as decimal strings.
Reject invalid counts, duplicate track keys, illegal states and oversize data at
adapter boundaries. Bound strings/enums and demonstrate worst-case serialization
within 16 KiB before freezing the schema. Body geometry is optional internal
tracking data and should be omitted from the initial browser presence payload.

Use bounded lossy detection delivery: snapshots replace old state, gaps are
observable, and empty sets are delivered. A presence snapshot is not a durable
person-entered/person-left fact. Do not implement alarm delivery or exactly-once
counts on this transport. Persistent rules/events remain a separate slice.

Authorize by camera and filter the relay to the matching publication room.
Sequence/order/freshness are checked independently per capability. Include
session identity as well as epoch; an in-process restart can retain a stream
epoch while resetting tracking. Old publisher, session or mode packets must not
restore stale UI. Mode transitions atomically clear both capability caches.

In combined mode the UI subscribes to both topics. Only face entries reach the
box renderer. Person results feed “People observed” and a separate uncertainty
indicator. No cumulative unique count, person-ID label or body rectangle appears.
Fixture results never render as real presence. Separate person/face health badges
must survive DOM reconciliation without confusing them with live-video health.

Provisional freshness limit is one second per capability, configurable. Receipt
age is a UI fallback; server age checks discard already stale model results.
Sample/source timestamps remain diagnostic until an explicit video-clock mapping
is measured. Frame-exact face alignment is not promised by sequence validation.

## 8. Failure, cancellation and recovery

Each lane loads and fails independently. Face failure leaves person presence;
person failure leaves face boxes with person availability unknown. Shared decode
failure affects both, while recording/live remain independent. Invalid settings
are rejected transactionally before replacing a working pipeline.

Transient failures use bounded retry/backoff with jitter and visible state;
malformed artifact contracts are terminal until an operator changes configuration.
A model must not silently fall back to fixtures or reinterpret another artifact.
Every lane task has an owner, cancellation signal and join/error policy. Pending
frames/results are discarded on restart/shutdown. FFmpeg children are joined or
killed through the existing owned process boundary.

Important cancellation limit: an in-process foreign ONNX call cannot be reliably
interrupted by dropping an async future. A timeout marks stale/unavailable and
must not start another operation or free memory still in use. A hung face lane
must not exhaust the person lane's reserved execution slot. Configure admission
and permits accordingly. In-process shutdown after a permanently hung call has
no proven finite deadline. If finite recovery/shutdown from that failure is an
enterprise requirement, qualify cooperative runtime cancellation or propose a
supervised model process in an ADR with bounded shared media transport. Do not
claim this requirement solved by `tokio::time::timeout`.

## 9. Configuration and observability

Register `person_face`, separate pinned person/face artifacts and independent
thresholds/schedules, confirmation/grace/freshness durations, concurrency/byte
budgets and queue capacities in the existing layered transactional configuration.
Keep face confidence 70% inclusive. Person threshold initially retains existing
70% as a reversible default; tune it from person recall/false-alarm evidence,
independently of face confidence. Validate confirmation duration against sample
cadence and freshness against grace; reject nonsensical/zero/unbounded values.
Model/mode/geometry/schedule/tracker changes apply on pipeline restart for this
slice. API reads expose effective values and restart classification.

Metrics use bounded labels: camera, lane, typed reason. Measure admitted/completed/
dropped samples; queue depth/capacity; per-lane preprocessing and inference latency
p50/p90/p99; result age; decoder work; failures/restarts; tracking state counts,
evictions and resets; packet drops and saturation; peak measured memory. Do not
label metrics with track IDs, paths, hashes, coordinates or arbitrary errors.
Trace actionable lifecycle changes with camera/session/request identifiers.

## 10. Acceptance and qualification gates

Deterministic contract tests must cover:

- Front-to-back-to-front sequence: face boxes clear on empty faces; fresh person
  observations continue. Back-facing first arrival and stationary-person input.
- Confirmation, reacquisition, crossing/ambiguous association, timed grace expiry,
  zero/missing distinction, saturation and capacity eviction.
- Property tests for finite clipped geometry, state/cardinality bounds, arbitrary
  time gaps, ID exhaustion and epoch/session resets. Controlled monotonic clocks.
- Different model shapes/color layouts and exact inverse transforms; malformed,
  corrupt, missing and mismatched artifacts; face threshold below/equal/above70%.
- Slow/hung lane simulation, exhausted global permits, bounded pending buffers,
  stale result rejection, failure/recovery, cancellation and owned shutdown.
- Browser dual capability state, empty faces, uncertain presence, reorder, wrong
  camera, old session, oversize packets, fixture guards and mode reconciliation.

Real end-to-end qualification must use labeled file-backed RTSP and supplement
it with the physical webcam. Record one upstream session and one detection
decoder per camera; verify recording continuity through model failures/overload
and live publishing outages. Test two actual simultaneous Chromium viewers and
Firefox separately; mocks cannot close a browser gate.

The labeled matrix includes front/side/back views (including first arrival from
behind), stationary subjects, turn-away sequences, low light, crowd/crossings,
occlusion, minimum subject sizes, edges and empty/negative scenes. Record model
hash/license, geometry, thresholds, source FPS/bitrate, subject sizes, hardware,
precision/recall, false alarms per camera-hour, detection delay, track fragmentation/
ID switches, result freshness and inference/memory distributions. Split tuning
and held-out qualification footage. Measure face accuracy separately from person
accuracy. Include comparison against the current person-only and face-only paths.

Local development gates prove specified fixture behavior and bounds. Production
release gates additionally require product-approved numerical accuracy, false-
alarm, freshness, recovery and latency targets for named camera conditions.
Targets are open; they must be agreed before evaluation, not selected after
seeing results. Qualify 10 cameras, then 20, on realistic footage/viewer workloads
with CPU/GPU/RSS/disk/network/drop/recovery measurements. No scale claim follows
from one webcam or architecture alone.

Run proportional focused checks during implementation; finish with workspace
formatting, all-target/all-feature Clippy, all-feature tests, release build, UI/
launcher tests and required security checks. Report actual skips and host linker
workarounds; current missing audit tools and Firefox qualification remain open.

## 11. Delivery order and rollback

1. Reproduce labeled turn-away and stationary scenarios; inventory current queues,
   memory and gate behavior. Measure canonical preprocessing and dual model cost.
2. Implement validated sample/evidence/state contracts and a deterministic,
   bounded, time-based tracker before runtime integration.
3. Add combined decode dispatch and independently owned model lanes; exercise
   overload, stale output and failure isolation with controlled adapters.
4. Add presence snapshots/health/wire contract and face-only dual-capability UI.
5. Run real model/RTSP/browser/recovery qualification and update verification.
   Only then switch the local launcher default to `person_face`.
6. Close numerical targets and target-hardware 10/20-camera production gates.

A failed proposed-config load preserves the prior accepted configuration. A
running lane failure visibly degrades its capability without stopping recording.
Rollback to `face` or `person` requires pipeline restart and clears both caches/
tracks. No database migration for recognition or durable presence is introduced.
The local launcher now defaults to `person_face`; production qualification remains open.
