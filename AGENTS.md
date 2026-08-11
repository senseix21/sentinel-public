# Engineering Instructions for the Surveillance Platform

## Mission

Build a production-grade, self-hosted, multi-camera surveillance and video analytics platform in Rust. The first deployment target is 10 cameras on one machine, followed by 20 cameras. Preserve a credible path to 100–200+ cameras without paying distributed-systems costs before measurements justify them.

Work as a principal Rust systems engineer: reason from first principles, make ownership and failure boundaries explicit, use Rust's type system to encode invariants, and optimize only from evidence. Deliver in small vertical slices that compile, run, and can be tested end to end.

Do not imitate a personality. Apply the engineering habits associated with Jon Gjengset's teaching: understand the machinery beneath abstractions, make costs visible, distinguish what is known from what is assumed, design APIs before implementations, and use measurements to settle performance questions.

## Sources of Truth

Read `analysis.md` before making architectural changes. It is the target architecture proposal, not a description of the current implementation.

Use this precedence when requirements conflict:

1. The user's current instruction.
2. Accepted requirements and architecture decision records in `docs/adr/`.
3. This file.
4. `analysis.md`.
5. Existing prototype behavior.

The existing `cam_rtsp` code is a prototype and reference implementation. It is not an architectural constraint. Preserve proven RTSP, GStreamer, FFmpeg, and H.265 mechanics where useful, but do not preserve accidental coupling or single-camera assumptions.

## Current Repository Reality

Assume the following until the code proves otherwise:

- The checked-in crate does not build because its dependencies inherit from a missing workspace root.
- It accepts one stream URL and uses a fixed `default_stream` identity.
- Recording, detection, and forwarding open independent connections to the same source.
- Recording is H.264-specific, writes one continuous file, and ignores the configured recordings directory.
- The GStreamer bus loop can stop draining after its first idle timeout.
- Recording metadata and one-hour durations are fabricated rather than probed or indexed.
- Clip selection scans the filesystem, selects only one file, and uses keyframe-imprecise stream-copy seeking.
- Configuration endpoints and some response objects are placeholders.
- Failure isolation, authentication, persistent configuration, metrics, database indexing, retention, and automated tests do not yet exist.
- `StreamProcessor`, `FileRecorder`, and MediaMTX-related paths contain substantial dead or parallel implementations.

Verify the relevant fact before relying on it because the repository will evolve. Never present an uncompiled path as working.

## Non-Negotiable Architecture

### Ingest once and distribute many

One ingestion owner maintains one upstream connection per camera. Downstream features must not independently reconnect to the camera.

```text
Camera -> Camera Ingestion -> Media Distributor
                                  |-> Recorder
                                  |-> Detector -> Domain Event Bus -> Rules/API/Analytics
                                  `-> LiveKit Publisher
```

Adding a consumer must not require changing camera connectivity.

### Keep media and domain events separate

The media plane transports encoded packets and decoded frames. The event plane transports lightweight domain facts.

- Encoded packets feed recording and, where compatible, live publishing.
- Decoding occurs only for consumers that require frames, principally detection.
- `EventBus` carries events such as camera state changes, detections, completed segments, and storage warnings.
- Never put video packets, frames, or unbounded binary payloads on the domain event bus.
- Do not decode and re-encode a stream unless a documented consumer or codec boundary requires it.

### Bound every queue

Every channel must have a documented capacity, overload policy, and metric.

- Recording prioritizes preservation and must surface inability to keep up.
- Detection favors recent frames; sample or drop stale frames rather than accumulating latency.
- Live viewing may drop stale media to preserve latency.
- Domain-event overflow policy must be explicit per event class. Critical lifecycle events must not disappear silently.
- No producer may feed an unbounded queue.

### Isolate failure domains

A camera, detector, LiveKit publisher, API, or database failure must not terminate unrelated recording pipelines. Supervise long-running tasks explicitly and define restart policy, backoff, cancellation, and terminal-error behavior.

Priority is provisionally:

1. Preserve recordings.
2. Maintain camera connectivity.
3. Serve live video.
4. Detect humans.
5. Produce derived analytics.

Treat this ordering as an assumption until the product owner confirms it.

### Make ownership singular

- `camera` owns camera connectivity, authentication, state, timestamps, and reconnect behavior.
- `media` owns packet/frame representations and in-process distribution semantics.
- `recorder` owns segment creation and recording lifecycle.
- `storage` owns recording metadata, file placement, retrieval, and retention coordination.
- `detector` owns sampling, preprocessing, and detection results.
- `analytics` owns ROI, occupancy, and heatmap derivation.
- `events` owns domain event contracts and pub/sub behavior.
- `rules` owns configurable event conditions and actions.
- `livekit` owns real-time publication, not archival storage.
- `config` owns validated settings and persistence.
- `api` translates external requests into application use cases; it does not own domain state.
- `database` implements persistence repositories and migrations; it does not contain business policy.

Do not create shared mutable state with multiple informal owners.

## Target Workspace

Evolve toward this workspace, adding a crate only with its first real vertical slice:

```text
apps/server
apps/worker                 # add only when work is actually deployable separately
crates/common
crates/config
crates/camera
crates/media
crates/recorder
crates/storage
crates/events
crates/api
crates/detector
crates/analytics
crates/rules
crates/livekit
crates/database
migrations
config
docs/adr
tests
deploy
```

Do not pre-create a forest of empty crates. The initial executable slice should normally use `server`, `common`, `config`, `camera`, `media`, `recorder`, `storage`, `events`, and the minimum API needed to observe it.

Dependencies must point inward toward stable domain contracts. Avoid a generic `common` dumping ground. Put a type in `common` only when it is genuinely cross-domain and has stable semantics.

## Rust Design Rules

### Start from data and invariants

Before choosing a crate or framework, answer:

1. What data enters and leaves this component?
2. Who owns it, and for how long?
3. Is it an encoded packet, decoded frame, command, query, or domain event?
4. What is the maximum buffered amount?
5. What happens when the consumer is slow or unavailable?
6. Which failures are transient, permanent, or operator-actionable?
7. Which measurements prove the design works?

### Design narrow contracts first

Define public APIs from required behavior, then implement them. Keep traits at real substitution or test boundaries, not around every struct. Do not invent a broad `CameraSource`, `Recorder`, or `EventBus` trait before media ownership, cancellation, error, and backpressure semantics are clear.

Prefer:

- Newtypes such as `CameraId`, `RecordingId`, `SegmentDuration`, and validated URLs.
- Enums for codec, media kind, connection state, and typed failure categories.
- Constructors that validate invariants.
- Private fields with behavior-preserving methods.
- Borrowing in hot paths and explicit ownership transfer at async boundaries.
- RAII for child processes, temporary files, subscriptions, and pipeline cleanup.
- Cancellation-safe async operations and structured task lifetimes.

Make invalid states difficult or impossible to represent. For example, do not model a connected camera without its active session, or a completed segment without known start/end timestamps and a durable path.

### Errors

- Libraries expose specific `thiserror` error types.
- Application boundaries may add context with `anyhow`.
- Preserve source errors and operational context without leaking credentials.
- Classify retryable failures instead of retrying every error.
- Do not use `unwrap`, `expect`, `panic!`, or silent error suppression in production paths unless an invariant is locally proven and documented.
- Retried operations use bounded exponential backoff with jitter and observable state transitions.

### Async and concurrency

- Never block a Tokio worker with synchronous process, filesystem, database, or media work.
- Use `tokio::process` for managed child processes.
- Isolate blocking foreign-library calls with an appropriate dedicated thread or blocking boundary.
- Avoid detached tasks. Every spawned task needs an owner, shutdown signal, and join/error policy.
- Do not hold synchronous mutex guards across `.await`.
- Treat cancellation and channel closure as designed state transitions, not incidental errors.

### Unsafe and performance work

- Unsafe Rust is a last resort. Document its safety invariants and cover the boundary with focused tests or Miri where applicable.
- Do not introduce allocation tricks, custom buffers, or zero-copy claims without a benchmark and profile.
- Retain encoded media as reference-counted immutable data where measurements show copying matters.
- Optimize algorithms and media topology before micro-optimizing syntax.
- Record the baseline and keep an optimization only when it produces a meaningful measured improvement without harming correctness or operability.

## Domain Requirements

### Camera lifecycle

Model lifecycle explicitly, including at least disconnected, connecting, connected, and reconnecting behavior. Define legal transitions and expose transition metrics. One camera's reconnect loop must not affect another camera.

Support heterogeneous camera properties. Do not hardcode H.264, assumed audio, resolution, FPS, bitrate, or one global stream URL. Codec-specific behavior must be selected from probed or configured media capabilities and validated against actual packets.

### Recording and playback

- Record the original encoded stream when container and codec compatibility allow.
- Create configurable 30–60 second segments; the exact default remains a decision to validate.
- Use deterministic paths based on camera and UTC time, such as `camera-id/YYYY/MM/DD/HH-MM-SS.ext`.
- Finalize files atomically where practical.
- Derive metadata from the actual segment, never fabricated defaults.
- Persist camera ID, exact start/end, path, codec, resolution, size, and lifecycle status.
- Queries use indexed metadata, not directory scans.
- Time-range playback must select all overlapping segments and clearly define keyframe accuracy, remuxing, and transcoding behavior.
- Retention operates from indexed metadata plus observed storage pressure. Protect recent footage and make unrecoverable disk pressure visible.
- Database unavailability must not immediately stop media preservation; specify a bounded metadata reconciliation strategy.

### Detection and analytics

Detection is an independently degradable consumer. Sampling rate and queue policy are configuration, not constants. Produce timestamped, camera-scoped structured results. Add tracking only when occupancy duration, paths, entry/exit, or unique-person counting requires it.

ROI coordinates must use a documented coordinate system and validation rules. Do not implement a heatmap until its intended meaning—density, occupancy, movement, or another metric—is confirmed.

### Configuration

Operational values expected to vary must not be hardcoded. Defaults belong in versioned configuration; deployment values are validated, persisted, and updateable through controlled APIs.

Examples include retention, segment duration, detection sample FPS, confidence threshold, reconnect policy, channel capacities, storage watermarks, camera credentials, and publisher settings.

Define for every dynamic setting whether it applies immediately, on the next segment/reconnect, or after restart. Apply multi-field changes transactionally or reject them. Never expose or log secrets.

### Events and rules

Start with a bounded in-process `EventBus` behind a contract whose semantics are explicit: ordering scope, delivery guarantees, overflow behavior, subscription lifetime, and shutdown. Do not claim that swapping in NATS or Kafka is trivial; an external broker changes delivery, persistence, ordering, and failure semantics.

Events are immutable facts in past tense, versioned when persisted or exposed externally. Commands are not events. Rules consume events and configuration; producers must not embed alerting or dashboard policy.

### API and security

Use logical camera-scoped resources under `/api`; do not expose storage paths or camera credentials. Validate identifiers, times, ranges, polygons, URLs, and configuration before invoking domain logic.

An API bound beyond loopback requires an explicit authentication, authorization, TLS/proxy, and secret-management decision. Development-only insecure behavior must be opt-in and visibly labeled. Avoid returning raw internal errors.

## Observability and Operations

Use structured tracing with camera, stream, recording, and request identifiers. Log state transitions and actionable failures, not every media item. Redact credentials and signed tokens.

Expose bounded-cardinality metrics for:

- Camera connection state, bitrate/FPS, reconnects, and packet loss when measurable.
- Queue capacity, depth, dropped items, and consumer lag.
- Segment success/failure, write throughput, and storage pressure.
- Detection throughput, latency, sampling, and dropped frames.
- LiveKit publication state and failures.
- API rate, error rate, and p50/p90/p99 latency.

Health endpoints must distinguish process liveness from dependency readiness and per-camera degradation. Support graceful shutdown that finishes or marks in-progress segments consistently.

## Engineering Workflow

### Before changing code

1. Read the relevant requirements and implementation completely.
2. Reproduce the current behavior or failure with the smallest reliable command or test.
3. State known facts, assumptions, and unresolved questions.
4. Trace data ownership, task lifetime, queueing, and failure propagation.
5. For a material or hard-to-reverse choice, create or update an ADR with options, trade-offs, decision, and reversal criteria.
6. Define observable acceptance criteria.

Do not perform speculative refactors unrelated to the active vertical slice. Preserve user changes in a dirty worktree.

### Implement vertically

Use the loop:

```text
Understand -> Minimal reproduction/spike -> Contract/test -> Implementation
           -> Integration test -> Measure -> Refactor
```

Tackle high-risk unknowns and integration seams early. Each phase must produce a runnable capability, not only types and empty modules. Prefer the smallest design that satisfies current requirements while leaving a clean extraction boundary.

### Continue an authorized phase to its gate

When the user explicitly authorizes implementation of an entire roadmap phase,
continue through every locally implementable item in that phase without waiting
for repeated `proceed` messages. Follow the roadmap order, update `ROADMAP.md`
and the latest dated `context/` snapshot at material milestones, and reserve the
complete workspace/external validation gate for the end of the phase. Focused
tests and compilation checks remain required while developing each contract.

Continuous execution does not override decision gates, destructive-action
rules, external hardware/product-owner requirements, or evidence standards. If
an item requires unavailable physical equipment, a product decision, new
authority, or unsafe scope expansion, record it as explicitly unverified or
blocked and continue with independent phase work. Never fabricate completion to
keep execution moving.

### Validate proportionally

For ordinary Rust changes, run the applicable subset and finish with:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --workspace --release
```

If system libraries, cameras, GPUs, LiveKit, or a database are unavailable, run everything independent of them and report exactly what remains unverified. Do not weaken or delete a test merely to make CI green.

Add regression tests for every fixed bug. Prefer deterministic tests with controlled clocks and sources. Use property tests for time ranges, retention, geometry, and state transitions where they add value. Use Loom or focused concurrency tests for subtle synchronization contracts. Benchmark claims about scale or performance with realistic codec, resolution, FPS, bitrate, camera count, detection FPS, and viewer count.

### Review before declaring completion

Check:

- The behavior meets an explicit requirement.
- Ownership, bounds, overload, cancellation, and failure behavior are visible.
- No new hardcoded deployment values or credentials exist.
- No unnecessary camera connection or decode/re-encode path was introduced.
- Errors preserve context and secrets remain redacted.
- Tests cover success, edge, failure, recovery, and shutdown where relevant.
- Metrics make degradation diagnosable.
- Documentation and ADRs match the implementation.
- The reported verification is factual and reproducible.

## Migration Sequence

Do not grow the prototype sideways. Migrate through tested vertical slices:

1. **Restore a reproducible build.** Create a real workspace, pin shared dependency versions, and establish formatting, linting, tests, and CI. Move nothing until baseline behavior and known failures are recorded.
2. **Validate critical unknowns.** Confirm camera codec, resolution, FPS, bitrate, audio/PTZ needs, hardware/GPU, storage, viewer concurrency, LiveKit constraints, and p90 objectives. Use spikes for RTSP depayloading, segmentation, LiveKit codec compatibility, and recovery behavior.
3. **One-camera ingest-to-record slice.** Introduce camera identity, explicit lifecycle, encoded packet representation, bounded distribution, segmented recording, actual metadata, and graceful shutdown. Reuse validated prototype mechanics behind new boundaries.
4. **Multi-camera supervision.** Run independent per-camera pipelines, reconnect independently, expose per-camera health, and prove one failure does not interrupt another.
5. **Indexed retrieval and retention.** Add PostgreSQL repositories/migrations, overlapping-segment queries, metadata reconciliation, storage watermarks, and tested retention policy.
6. **Live publication.** Add LiveKit as an independent consumer and prove its outage does not affect recording.
7. **Detection and events.** Decode/sample once for detection, publish structured domain events through the bounded in-process bus, and demonstrate overload behavior.
8. **ROI, rules, and analytics.** Add only after semantics and acceptance criteria are confirmed.
9. **Scale from measurements.** Load test 10 and 20 realistic cameras before extracting a worker or introducing an external bus. Extract only the measured bottleneck.

Every step must be independently demoable, observable, and reversible enough to diagnose regressions.

## Local macOS Webcam Testing

Use the Mac webcam as a development source through an RTSP bridge so tests exercise the production RTSP ingestion path:

```text
AVFoundation webcam -> FFmpeg -> local MediaMTX -> RTSP ingestion
```

Treat direct AVFoundation ingestion as a separate adapter only if product or test requirements justify it. Keep deterministic file-backed RTSP fixtures for automated tests; a physical webcam is for manual smoke and recovery testing, not CI.

Never bake device indices into core code. Discover/select the device in local configuration, document required macOS camera permission, and keep the default server configuration free of public RTSP URLs.

## Explicit Decision Gates

Do not silently freeze these open requirements:

- Camera resolution, FPS, codec, bitrate, RTSP/ONVIF, audio, and PTZ.
- Detection model, hardware/GPU, sample FPS, confidence behavior, and whether tracking is required.
- The semantic definition of heatmap and ROI administration.
- Playback accuracy, stream/download behavior, startup target, and event pre/post roll.
- Viewer concurrency, live-latency target, and LiveKit topology/codec requirements.
- Server hardware, operating system, storage layout, and deployment model.
- Availability objectives, data-loss tolerance, disk-failure behavior, and subsystem priority.
- Exact p90 targets and the test workloads used to measure them.

When work reaches one of these gates, either obtain the requirement or implement a reversible, explicitly documented default that does not masquerade as a confirmed requirement.

## What Not to Introduce Yet

Do not add Kubernetes, Kafka, NATS, pervasive microservices, distributed databases, service meshes, cloud video storage, or speculative abstraction layers without a measured constraint and an ADR. A bounded modular monolith is the default for 10–20 cameras.

Do not claim support for 20 or 200 cameras from architecture alone. Support is demonstrated only by a documented workload and repeatable measurements of CPU, GPU, memory, disk, network, queue drops, recovery, and percentile latency.
