# Phase 6 Handoff — Human Detection and First-Class Domain Events

Status: ready to implement from the Phase 5 checkpoint

Baseline date: 2026-08-12

## Mission

Add sampled human detection as an independently degradable consumer of each
camera's existing media path and publish lightweight, typed domain facts through
a bounded in-process event bus. Recording and LiveKit publication must continue
when decoding, inference, event publication, or an event subscriber is slow,
unavailable, restarted, or terminally misconfigured.

Phase 6 does not add ROI, tracking, occupancy, heatmaps, rules, alerts, dashboard
policy, or an external broker. It establishes the timestamped detection and
event contracts those later features require.

## Read first

Read these sources in order before changing detection or event behavior:

1. `AGENTS.md`
2. `context/README.md` and its latest snapshot
3. `ROADMAP.md`, Phase 6
4. `docs/requirements.md`
5. `docs/adr/001-media-pipeline.md`
6. `docs/adr/002-event-bus.md`
7. `docs/adr/003-configuration.md`
8. `docs/phase-0/timestamp-contract.md`
9. `docs/phase-0/media-distribution-contract.md`
10. `docs/phase-5/local-evidence-2026-08-12.md`
11. `crates/media/src/packet.rs`
12. `crates/media/src/distribution.rs`
13. `crates/camera/src/supervisor.rs`
14. `apps/server/src/one_camera.rs`
15. `apps/server/src/pipeline.rs`
16. `apps/server/src/camera_supervisor.rs`
17. `apps/server/src/health_http.rs`
18. `crates/detector/`
19. `crates/events/`

Read `analysis.md` only to resolve architectural ambiguity. It is a proposal,
not evidence of implemented behavior.

## Verified starting point

The workspace currently provides:

- one owned authenticated RTSP/TCP session per running camera;
- complete bounded H.264 access units with source media timestamps;
- independent recorder-preserving and live-freshness queues;
- independently supervised camera pipelines with reconnect epochs and joined
  shutdown;
- durable fragmented-MP4 recording, indexed retrieval, reconciliation, playback,
  and retention;
- an optional LiveKit consumer whose outage does not reject recorder admission;
- camera-scoped API health and bounded-cardinality Prometheus metrics;
- a clean workspace gate with 106 ordinary tests, warning-free Clippy, and a
  release build;
- real local RTSP/LiveKit/recording and LiveKit-startup-outage fixtures passing.

Phase 5 remains a checkpoint rather than a completed production qualification:
the released `lk` adapter owns its clock, and representative cameras, browser
decode, concurrency, and latency remain open. Phase 6 must not depend on those
gates and must not weaken them.

`crates/detector`, `crates/events`, `crates/media/src/frame.rs`, and
`crates/media/src/stream.rs` contain one-line placeholder modules. Their presence
is not an implementation, API commitment, or reason to preserve a speculative
shape. Characterize and replace them only through the first runnable slice.

## Product and hardware decision gates

The accepted qualification default is five sampled frames per second, but Erik
has not yet accepted:

- the production detector/model and version;
- CPU, GPU, accelerator, driver, and target operating system;
- required precision/recall or labeled evaluation dataset;
- confidence semantics and threshold ownership;
- acceptable detection freshness and p90 objective;
- whether tracking is required and for which business outcome.

Proceed locally with a deterministic fake inference adapter and reversible 5 FPS
sampling. A real model adapter may be spiked behind the narrow inference contract,
but no model accuracy, GPU support, or camera-count capacity claim is permitted
without the inputs above.

## Non-negotiable invariants

1. Detection consumes the camera-owned media path; it never opens another RTSP
   connection.
2. Recording continues when detection, decoding, inference, the event bus, or an
   event subscriber fails or saturates.
3. The encoded recorder and LiveKit paths are not decoded or re-encoded for
   detection.
4. Decode occurs once per sampled detection input. Do not create independent
   decoders for future analytics consumers without measured justification.
5. Every queue has a validated item capacity, byte/resource bound, overload
   policy, closure behavior, owner, and metrics.
6. Detection favors freshness. Stale encoded units and decoded frames are
   sampled or evicted rather than accumulating unbounded latency.
7. Sampling uses media timestamps and explicit connection epochs. Executor
   arrival time alone is not a sampling clock.
8. A reconnect, backward timestamp, or incompatible caps change resets sampler
   and decoder state explicitly.
9. Decoded frames have one owner across async boundaries and a configured
   maximum width, height, pixel format, and byte size.
10. Detection results are immutable camera- and timestamp-scoped facts with
    validated normalized geometry, confidence, class, and model version.
11. Packets, access units, frames, images, clips, tensors, and arbitrary binary
    payloads never enter the domain event bus.
12. Events are past-tense facts. Start/stop/reconfigure requests are commands,
    not events.
13. Event ordering, delivery, overflow, and shutdown semantics are explicit per
    class. Critical lifecycle facts cannot disappear silently.
14. A slow or failed subscriber cannot block media processing or unrelated event
    subscribers indefinitely.
15. Detection/model errors, logs, API resources, and metrics never expose camera
    credentials, media payloads, filesystem paths, or unbounded labels.
16. Every decoder, detector, event publisher/subscriber, retry timer, blocking
    inference job, and child process has singular ownership and a join policy.

## Target ownership and data flow

```text
CameraPipeline
├── Camera ingest owner
├── Recorder consumer                         encoded, preservation-oriented
├── LivePublicationOwner                     encoded, latency-oriented
└── DetectionOwner                           independently degradable
    ├── timestamp sampler                    freshness-oriented
    ├── one decode boundary
    ├── bounded decoded-frame handoff
    ├── narrow inference adapter
    └── DetectionObserved event publisher

ServerRuntime
├── bounded EventBus
├── camera/recording/storage event adapters
├── event health + metrics
└── joined shutdown
```

`media` owns encoded and decoded media representations. `detector` owns sampling,
decode/inference orchestration, and detection-result contracts. `events` owns
lightweight event schemas and bounded delivery. The camera pipeline owns task
composition. `analytics` and `rules` may consume events in Phase 7 but do not
shape Phase 6 producer policy.

## Contracts to design before implementation

### Detection input and sampling

Define a detection subscription independent of recorder and LiveKit queues. Do
not overload the current two-consumer distributor with informal shared ownership.
Specify:

- whether sampling occurs before decode, and which complete access unit is chosen;
- configurable sample rate and maximum retained items/bytes;
- newest-item preference and stale-drop behavior;
- keyframe/decoder warm-up behavior after startup and reconnect;
- timestamp rounding for arbitrary validated timebases;
- duplicate timestamps, variable FPS, gaps, backward time, and epoch reset;
- queue close, cancellation, and camera terminal behavior;
- counters for considered, sampled, skipped-by-rate, stale-dropped, decode failed,
  inferred, and rejected items.

Sampling before decode is the default because it avoids decoding frames that will
be discarded. Prove that the chosen decoder can begin from the retained stream
state; if inter-frame dependencies require continuous decode, record the cost and
choose a bounded alternative by ADR rather than assuming random frame decode.

### Decoded-frame contract

Define a narrow representation with:

- `CameraId`, connection epoch, media timestamp, and observation timestamp;
- validated width, height, pixel format, stride/plane layout, and byte length;
- immutable or singularly owned bounded pixel storage;
- explicit color-space/range assumptions needed by preprocessing;
- no fabricated UTC when source-to-wall mapping is unavailable.

Do not put foreign-library pointers or unsafe lifetimes in the public domain type.
Isolate blocking decoder calls from Tokio workers. Any unsafe/FFI boundary needs
documented safety invariants and focused tests.

### Inference adapter and result domain

Create a trait only at the real model substitution/test boundary. It accepts one
validated decoded frame or preprocessed tensor and returns a bounded collection
of detections. Define:

- model identifier and version;
- class contract limited to the accepted human/person outcome for Phase 6;
- confidence as a finite validated value with explicit range;
- normalized bounding boxes with coordinate origin, axis direction, and bounds;
- maximum detections per frame and deterministic excess-result policy;
- retryable, terminal, malformed-output, unavailable, and resource-exhausted
  failures;
- blocking/GPU execution boundary, cancellation limitation, and timeout policy.

The deterministic fake adapter is test infrastructure, not an accuracy claim.
Do not add tracking identifiers before tracking semantics are accepted.

### Event schema

Start with a closed, versioned domain-event enum or equally narrow sum type. At
minimum Phase 6 should support lightweight facts for:

- camera connection/state transition;
- recording segment completed/failed;
- storage warning/critical transition;
- person detection observed;
- detector state/degradation transition.

Each event carries an event ID, schema version, occurred-at time, source camera
when applicable, and typed payload. Detection events additionally retain source
media timestamp/epoch and model version. Persisted/external serialization must
be stable and versioned; in-process-only details remain private.

### Event-bus delivery classes

Resolve ADR 002 before composing producers. A recommended reversible split is:

- `CriticalLifecycle`: bounded admission that returns visible backpressure/error
  to a supervised adapter, with a reconciliation/outbox path where durable facts
  already exist; never silently dropped.
- `LatestState`: coalesces/replaces older state for the same bounded key.
- `Telemetry`: bounded lossy delivery with explicit drop counters.
- `Detection`: bounded per-camera ordering, subscriber-independent queues, and a
  documented drop/coalesce policy that favors recent facts.

Do not claim exactly-once delivery. Define per-producer ordering, at-most-once
in-process delivery unless stronger evidence exists, duplicate behavior after
recovery, subscription lifetime, and whether a publisher can observe that all
subscribers are closed.

### Supervision and failure behavior

`DetectionOwner` must own sampler, decoder, inference, publication, cancellation,
and join behavior. Define disabled, starting, running, degraded/backoff, terminal,
stopping, and stopped states. Model configuration/model-load failures are
terminal until configuration changes; transient device/inference failures use
bounded backoff when retry is safe.

If detection exits, the camera pipeline remains running and reports detection
degraded. Shutdown closes detection input, joins in-flight work within a bounded
policy, closes its event publisher, and never delays recorder finalization
indefinitely.

## Work packages

### P6.1 — Refresh detection and event decisions

- Characterize the placeholder crates and current pipeline seams.
- Update ADR 001 for the selected sample/decode topology.
- Resolve ADR 002 delivery classes, ordering, overflow, and shutdown semantics.
- Compare decoder/model adapters against platform, licensing, blocking, and
  deployment costs without selecting a production model by convenience.
- Record reversible fake-adapter defaults and explicit external gates.

Tests/spikes: H.264 sample/decode feasibility, variable timestamps, reconnect,
cancellation, bounded allocation, and model-adapter substitution.

Exit: ownership, decode topology, event semantics, and acceptance tests are
documented before the runnable pipeline depends on them.

### P6.2 — Implement event domain contracts

- Add validated event IDs, schema versions, occurred-at timestamps, source IDs,
  delivery classes, and closed typed payloads.
- Define normalized detection geometry, confidence, model identity/version, and
  maximum result count.
- Keep serialization stable and exclude media/file/credential data by type.

Tests: invalid IDs/versions/times, non-finite confidence, geometry bounds,
oversized result sets, serialization round-trip/version fixtures, and compile-time
or API tests preventing media payload inclusion.

Exit: invalid or media-bearing domain events cannot cross the public boundary.

### P6.3 — Implement the bounded in-process event bus

- Give each subscription an independent bounded queue and explicit delivery
  class policy.
- Preserve per-producer order for accepted events.
- Make overflow, coalescing, closure, and absent subscribers observable.
- Own async wake-up without lost notifications and support drain/stop policy.
- Avoid synchronous mutex guards across `.await` and unbounded cloning.

Tests: ordering, each overflow policy, slow/dead subscriber isolation, no lost
wake-up, producer/subscriber close, concurrent publish/unsubscribe, cancellation,
and bounded retained memory.

Exit: event delivery semantics are deterministic, bounded, and honestly weaker
than a durable external broker.

### P6.4 — Implement timestamp-aware sampling

- Add validated sample-rate and lag/buffer configuration.
- Select input by media timestamp and connection epoch.
- Prefer current media under overload and count every skip/drop category.
- Reset deterministically on gaps, backward timestamps, timebase changes, and
  reconnect epochs.

Tests: fixed/variable FPS, nonzero origins, rational timebases, duplicate/gapped/
backward timestamps, epoch reset, queue saturation, and shutdown.

Exit: the sampler produces the configured maximum rate without executor-timing
drift or unbounded lag.

### P6.5 — Add one bounded decode boundary

- Select and isolate the initial H.264 decoder adapter.
- Validate caps and decoded-frame dimensions/format/byte bounds.
- Keep blocking/FFI work off Tokio workers.
- Define decoder warm-up, keyframe wait, reset, malformed media, and teardown.
- Prove detection decode does not alter encoded recorder/LiveKit data.

Tests: parameterized keyframe startup, inter-frame sequence, corrupt access unit,
dimension/caps change, reconnect reset, oversized frame, cancellation, and leak
checks.

Exit: sampled H.264 yields bounded decoded frames under an owned lifetime.

### P6.6 — Implement the inference boundary and fake detector

- Add the narrow inference adapter and typed failures.
- Implement deterministic fake inference for fixtures.
- If authorized inputs exist, add one isolated real model spike without making
  accuracy or capacity claims.
- Enforce maximum results, confidence, geometry, timeout, and cancellation policy.

Tests: zero/one/many people, malformed/non-finite/excess output, adapter timeout,
terminal model load, retryable device failure, and cancellation.

Exit: deterministic person results can be produced without coupling a model SDK
to camera, recorder, API, or event-bus ownership.

### P6.7 — Implement one owned `DetectionOwner`

- Compose sampling, decode, inference, event publication, health, retry/backoff,
  cancellation, and join behavior.
- Drop stale work while inference is slow.
- Keep critical media owners independent when detection degrades or stops.
- Expose fixed-cardinality per-camera health and counters.

Tests: requested shutdown in every state, input close, slow decode/inference,
queue saturation, retry reset, terminal configuration, event-bus saturation,
subscriber absence, and no detached work.

Exit: detection repeatedly starts, degrades, recovers, and stops with bounded
memory and deterministic ownership.

### P6.8 — Attach detection and lifecycle producers

- Add one detector subscription to each enabled camera pipeline without adding
  another RTSP session.
- Keep detection-disabled cameras free of decoder/model tasks.
- Adapt camera, recording, and storage lifecycle facts into the event bus at
  their existing ownership boundaries.
- Ensure event failures cannot terminate recorder, LiveKit, API, or unrelated
  cameras.

Tests: recorder + LiveKit + detector share one ingest; two-camera isolation;
detector failure while segments/live publication continue; camera reconnect;
database/LiveKit outage composition; and graceful shutdown ordering.

Exit: the runnable server emits real lifecycle and deterministic person events
without changing media failure priority.

### P6.9 — Add API, observability, and local qualification

- Expose credential-free detector/event health and bounded metrics.
- Add a camera-scoped development event/detection query or stream only if its
  retention and authorization boundary is explicit; do not invent persistence.
- Record queue capacity/depth/drops, sampling, decode/inference throughput and
  failures, event overflow, consumer lag, and p50/p90/p99 processing latency.
- Build repository-owned deterministic fixtures for overload, crash, recovery,
  shutdown, and event validation.
- Update `ROADMAP.md` and the latest dated `context/` snapshot with evidence.

Exit: the local failure matrix passes, performance is measured for a named fake
or accepted model workload, and external accuracy/hardware gates are separated.

## Required failure matrix

At minimum qualify:

- detection disabled;
- decoder unavailable or terminally incompatible;
- malformed/oversized encoded media and decoded frame;
- timestamp gap, backward jump, timebase change, and reconnect epoch;
- sample, decode, inference, and event queues saturated independently;
- inference timeout, crash/unavailability, and malformed output;
- no event subscribers and one slow/dead subscriber;
- critical lifecycle event admission failure;
- camera outage while detector is active;
- LiveKit outage while detector and recorder remain active;
- PostgreSQL outage while detector and recorder remain active;
- detector shutdown during active inference;
- process shutdown with queued events and active recording;
- two cameras where one detector fails terminally.

For every case record expected recording, camera, LiveKit, detector, event-bus,
readiness, metric, retry, and cleanup behavior. Do not infer isolation from unit
architecture alone.

## Observability requirements

Use bounded camera/model/failure labels only. Expose at least:

- detector enabled/state and last bounded failure category;
- encoded items considered, sampled, skipped, stale-dropped, and rejected;
- decode attempts/success/failure and decoded-frame bounds;
- inference attempts/success/failure/timeout and detections produced;
- sampling, decode, inference, and end-to-event latency histograms or summaries
  with a precisely named clock boundary;
- detector queue capacity/depth/high-water/drop counters;
- event publications by stable event kind and delivery class;
- event subscriber capacity/depth/delivered/dropped/coalesced/closed counters;
- reconnect/reset and shutdown/join failures.

Never label by event ID, bounding box, confidence, arbitrary subscriber name,
path, URL, token, or error string.

## Local and production exit gates

Phase 6 is locally complete only when:

- one camera connection feeds recorder, LiveKit when enabled, and detection;
- deterministic H.264 produces bounded decoded samples and camera/timestamp-
  scoped structured person detections;
- slow or failed detection cannot grow memory without bound or interrupt
  recording/live publication/unrelated cameras;
- timestamp sampling and reconnect resets pass variable-FPS tests;
- event ordering, delivery, overflow, subscription, and shutdown semantics are
  implemented and documented;
- no packet, access unit, frame, tensor, image, clip, or binary media appears on
  the event bus;
- camera, recording, storage, detector, and detection facts use typed past-tense
  contracts;
- the required local failure matrix and task/resource leak checks pass;
- processing latency/resource use is measured for a named deterministic workload;
- all workspace and Phase 0–5 regression gates still pass.

The production gate additionally requires an accepted model and labeled dataset,
accuracy criteria, target GPU/CPU/driver/OS, representative 4 MP cameras, agreed
5 FPS/freshness objectives, and 10-camera combined recording/live/detection soak.

## Validation commands

Run focused tests during each package. At the Phase 6 local gate run:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --workspace --release
git diff --check
```

Also run the repository-owned Phase 0 RTSP/crash fixture, Phase 3 multi-camera
isolation fixture, Phase 4 PostgreSQL/outage/playback/retention fixture, Phase 5
LiveKit/outage fixtures, and the new detector/event overload/recovery fixtures.
Report unavailable cameras, model/GPU, target hardware, browsers, and production
deployment separately.

## Explicit first implementation action

Update ADR 002 and write contract tests for a minimal bounded event bus before
adding a decoder or model dependency. Start with one `PersonDetected` fixture
containing validated camera identity, connection epoch, media timestamp, UTC
observation time, model version, finite confidence, and normalized bounding box.
Prove that media payload types cannot be embedded, accepted events retain
per-producer order, a saturated detection subscriber follows its documented
freshness policy, a critical lifecycle overflow is visible, and shutdown wakes
and joins every subscription. Only then attach a deterministic fake detector to
one camera pipeline.
