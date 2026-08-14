# Surveillance Platform Delivery Roadmap

## Purpose

This roadmap turns `analysis.md` into an executable sequence of production-oriented vertical slices. It is governed by `AGENTS.md`. The roadmap is not a promise of calendar dates: camera specifications, hardware, staffing, and performance objectives remain unresolved. Effort should be estimated only after Phase 0 closes those gaps.

The delivery rule is:

```text
Requirements -> Risk spike -> Contract -> Vertical slice -> Failure test -> Measurement -> Gate
```

No phase is complete because files or traits exist. A phase is complete only when its end-to-end behavior and failure behavior are demonstrated with reproducible evidence.

## Status Legend

- `[ ]` Not started
- `[~]` In progress
- `[x]` Verified complete
- `[!]` Blocked by a recorded decision or external dependency

Update checkboxes only with evidence. Put current handoff details in the latest dated `context/` snapshot, not in this durable roadmap.

## Target Outcome

The first production milestone supports 10 heterogeneous RTSP cameras on one server with:

- One owned upstream connection per camera.
- Continuous, segmented, indexed local recording.
- Independent reconnect and health state per camera.
- Bounded, observable media distribution.
- Time-range recording queries across segments.
- Configurable retention and storage-pressure handling.
- LiveKit publication independent of recording.
- Sampled human detection producing structured events.
- Persistent, validated configuration.
- Authentication, metrics, graceful shutdown, and operating runbooks.

Support for 20 cameras is a measured optimization milestone. Support for 100–200+ cameras is an architectural direction until load tests prove it.

## Dependency Map

```text
Phase 0: requirements and risk closure
    |
Phase 1: reproducible workspace and quality baseline
    |
Phase 2: one camera -> segmented recording
    |
Phase 3: multi-camera supervision and failure isolation
    |
Phase 4: indexed retrieval, reconciliation, and retention
    |--------------------|
Phase 5: LiveKit         Phase 6: detection and event bus
    |--------------------|
              |
Phase 7: configuration API, rules, ROI, and analytics
              |
Phase 8: security and production operations
              |
Phase 9: 10/20-camera qualification and evidence-led scaling
```

Security, observability, testing, and documentation are continuous work. Phase 8 closes production gaps; it does not defer basic security or telemetry until the end.

---

## Phase 0 — Confirm Requirements and Retire Critical Unknowns

### Objective

Replace assumptions that determine media topology, resource sizing, and product behavior with explicit decisions or time-boxed experimental evidence.

### Instructions

- [x] Confirm the initial camera qualification profile: 4 MP, 15 FPS, H.264 High Profile, 4 Mbps planning bitrate, two-second GOP, RTSP/TCP, no MVP audio/PTZ, and ONVIF Profile T. Actual models and measured bitrate remain qualification inputs.
- [ ] Inventory representative camera models and obtain sample streams for every codec/profile combination.
- [x] Confirm 10 cameras initially, 20-camera measured target, and continuous recording for the qualification workload.
- [ ] Confirm server OS, CPU, GPU, RAM, disk type/capacity/layout, and network capacity.
- [ ] Confirm detection model candidates, acceptable detection FPS, confidence semantics, and whether tracking is required.
- [ ] Define heatmap semantics: density, occupancy, movement frequency, dwell time, or another measure.
- [ ] Confirm viewer concurrency, LiveKit deployment ownership, codec compatibility, and target live latency.
- [ ] Confirm playback behavior: streamed response versus download, boundary accuracy, startup target, and pre/post-event buffers.
- [ ] Confirm availability, recovery-time, and acceptable data-loss objectives.
- [ ] Confirm disk-full policy and business priority among recording, live viewing, detection, and analytics.
- [ ] Agree on p90 objectives and the workloads used to measure them; report p50/p90/p99.
- [x] Correct the 200-camera storage estimate across explicit bitrate scenarios and label estimates as estimates (`docs/phase-0/resource-sizing.md`). Real camera profiles remain pending.

### Risk spikes

- [x] Prove synthetic H.264 and H.265 RTSP/TCP depayloading and copy-mode remuxing through MediaMTX. Representative production-camera qualification remains an exit-gate requirement.
- [~] Prove clean 30-second and 60-second segmentation with correct timestamps and playable finalized files. Synthetic H.264/30s passed; H.265/60s is playable but exposed negative PTS/boundary drift and remains open.
- [x] Compare normal and `SIGKILL` finalization for H.264/H.265 Matroska, fragmented MP4, and classic MP4; select fragmented MP4 plus an explicit partial-file lifecycle for the H.264 MVP.
- [~] Determine LiveKit ingest requirements for representative camera codecs. Synthetic and webcam single-slice H.264 sustained 15 FPS without pixel transcoding (10,464 packets forwarded, zero LiveKit drops). Multi-slice input exposed NAL-versus-access-unit pacing, and startup ordering caused pre-attach RTSP drops. A measured Rust access-unit publisher, representative High Profile cameras, browser playback/latency, and H.265 policy remain open.
- [x] Test a deterministic synthetic publisher stop/start, offline boundary, fresh timestamp epoch, and mid-GOP/keyframe join. Production-camera reconnect and the future Rust supervisor remain qualification work.
- [x] Test Mac webcam -> FFmpeg -> local MediaMTX -> RTSP as the manual development source, including explicit 4:2:2-to-`yuv420p` conversion and downstream copy remux.
- [~] Create deterministic RTSP fixtures for automated integration tests. Controlled synthetic reconnect/reset, authenticated Retina ingestion, live supervisor publisher stop/start recovery, and accelerated recording fixtures pass; a licensed representative-camera/file fixture remains open.
- [x] Define and test the first SDK-neutral H.264 access-unit contract: validated depayloaded NALs, explicit marker/timestamp assembly, multi-slice grouping, SPS/PPS and IDR classification, malformed-input rejection, and discontinuity reset. Direct encoded publication is not yet available in a released official LiveKit Rust API.
- [x] Implement bounded RFC 6184 non-interleaved H.264 depayloading for single-NAL, STAP-A, and FU-A payloads, including sequence-wrap/gap handling and an end-to-end RTP-to-access-unit test. Production limits remain a representative-camera configuration input.
- [x] Define and test SDK-neutral bounded distribution semantics: independent recorder/live queues, recorder backlog preservation with visible rejection, live oldest-item eviction, explicit closure, and queue counters. Async integration and measured byte budgets remain Phase 2 work.
- [x] Define and test the H.264 fragmented-MP4 segment lifecycle: validated partial/durable paths, consuming open-to-finalizing transitions, durable-file evidence, truthful completion fields, and typed failure records for reconciliation. Filesystem/muxer integration remains Phase 2 work.
- [x] Connect the bounded recorder subscription to a narrow H.264 access-unit sink contract. Tests prove ordered delivery, drain-before-close, explicit counters, and ownership return of a sink-rejected unit so failure is not silent.
- [x] Add validated `CameraId`/`RecordingId` contracts and storage-owned deterministic camera/UTC paths. Tests cover traversal rejection, calendar bounds, configured-root containment, partial/durable derivation, and same-second collision avoidance through recording identity.
- [x] Prove the deterministic encoded-media seam end to end: synthetic RTP/H.264 payloads traverse camera depayloading, complete access-unit assembly, bounded distribution, and recorder consumption while a stalled live queue sheds only its own stale media.
- [x] Implement cancellation-safe async queue wake-up and an owned recorder task. Tests prove no lost publication/closure wake-ups, drain-before-exit, cooperative cancellation, sink-failure ownership return, and no silent task detachment.
- [x] Define and test the adapter-independent camera lifecycle with state-specific ownership, session-bearing connected state, attempt/epoch identity, retryable versus terminal failures, credential-free snapshots, and transition counters.
- [x] Select Retina 0.4.19 provisionally for the deterministic H.264 RTSP/TCP spike with explicit qualification and reversal criteria (`docs/adr/004-rtsp-client-adapter.md`). No production support claim is made before the spike passes.
- [x] Prove the isolated Retina adapter against authenticated local MediaMTX: separate redacted credentials, interleaved TCP, H.264 capability validation, complete bounded access units, 90 kHz timestamps, keyframes, terminal authentication rejection, and bounded teardown. Representative-camera and reconnect-supervisor qualification remain open.

Keep spike code isolated. Promote it only after its contract and operational behavior are understood.

### Deliverables

- [x] `docs/requirements.md` containing confirmed direction, reversible defaults, and explicitly labeled open requirements.
- [x] Proposed ADRs for media pipeline, event semantics, configuration behavior, and initial persistence choice. Acceptance remains gated by open evidence.
- [x] Camera/media capability matrix created; production camera rows remain intentionally incomplete.
- [x] Resource-sizing worksheet with formulas and bitrate sensitivity inputs.
- [x] Reproducible file-backed, RTSP reconnect, container-crash, webcam bridge, and synthetic H.264 LiveKit spike commands/results are documented. Representative-camera experiments remain pending.

### Exit gate

- [ ] Every topology-changing unknown is answered or has a reversible documented default.
- [ ] Representative H.264/H.265 recordings are playable and their cost is measured.
- [ ] Product owner accepts the initial reliability and performance objectives.
- [x] No current sizing estimate is presented as measured production capacity.

### Out of scope

Production feature implementation, Kubernetes, external event brokers, distributed scheduling, and broad crate scaffolding.

---

## Phase 1 — Restore the Build and Establish the Workspace

### Objective

Create a reproducible Rust workspace that builds cleanly and establishes enforceable engineering checks without changing product behavior accidentally.

### Instructions

- [ ] Capture the existing manifest failure as baseline evidence.
- [ ] Replace orphaned `workspace = true` dependencies with a real root `[workspace]` and `[workspace.dependencies]` containing pinned compatible versions.
- [ ] Introduce `apps/server` and only the crates needed by the first vertical slice: `common`, `config`, `camera`, `media`, `recorder`, `storage`, `events`, and minimal `api`.
- [ ] Preserve the prototype in a clearly labeled location or migrate individual mechanics only when covered by characterization tests.
- [ ] Remove public hardcoded RTSP URLs and credentials from runtime defaults and documentation examples.
- [ ] Establish workspace lint policy without blindly enabling lints that conflict with FFI/media code.
- [ ] Add CI for format, clippy, tests, and release build.
- [ ] Record required GStreamer/FFmpeg system packages for macOS development and target Linux deployment.
- [ ] Add a configuration example with safe local defaults and no secrets.

### Required checks

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --workspace --release
```

### Deliverables

- Reproducible workspace and lockfile.
- CI workflow and development prerequisites.
- Characterization inventory identifying prototype code to reuse, rewrite, or delete later.
- Minimal server binary that starts, reports version/build information, and shuts down cleanly.

### Exit gate

- [ ] A fresh checkout builds using documented prerequisites.
- [ ] All required checks pass.
- [ ] No empty speculative crates exist.
- [ ] Prototype behavior and known defects remain documented in the latest context snapshot.

---

## Phase 2 — One-Camera Ingest-to-Record Vertical Slice

### Objective

Deliver one real RTSP camera connection that distributes encoded media to one segmented recorder with truthful metadata and clean lifecycle management.

### Domain contracts

- `CameraId`: validated stable identity, never a free-form default string.
- `CameraConfig`: validated RTSP source and connection policy; secrets are redacted.
- `ConnectionState`: explicit legal state transitions.
- `EncodedPacket`: codec, timestamps/timebase, keyframe flag, immutable payload, and camera/stream identity.
- `MediaDistributor`: bounded subscription contract with explicit overflow behavior.
- `RecordingSegment`: lifecycle states such as open, finalizing, completed, and failed.

Do not freeze trait shapes before cancellation, ownership, and backpressure semantics are known.

### Instructions

- [x] Implement one ingestion owner for one upstream RTSP connection. The runnable server constructs one H.264 supervisor owning exactly one authenticated interleaved-TCP Retina session, survives publisher loss with a fresh epoch, and returns all ownership on exit.
- [x] Model explicit disconnected, connecting, connected, reconnecting, and stopped camera lifecycle. The supervisor drives the typestate contract, owns explicit session teardown, and publishes every latest lifecycle state through a constant-memory health channel; the fixture proves reconnect/resumed media.
- [x] Probe or validate codec/caps; support only explicitly qualified combinations and fail clearly for others. The runnable path selects H.264 explicitly, compares negotiated dimensions with required configured dimensions, rejects mismatches and unsupported media as terminal failures, and exposes actual dimensions/RFC 6381 codec. Representative-camera interoperability remains an external qualification gate, not an unverified support claim.
- [x] Replace the stalled bus-loop behavior with continuously serviced message/error/EOS handling. The new Retina path continuously polls media/errors/EOF under the supervisor and does not use the prototype GStreamer bus loop.
- [x] Implement bounded encoded-packet distribution. Real RTSP access units enter independently bounded queues; recorder saturation stops the camera visibly and live eviction remains isolated. Transactional configuration computes the worst-case queue-plus-two-fragment byte bound, and the loopback metrics endpoint exports queue capacity, depth, acceptance, rejection, and drop counters.
- [x] Record without decoding/re-encoding where format compatibility permits. The H.264 sink remuxes existing access units; no pixel decode or encoder is present.
- [x] Write configurable 30–60 second segments beneath the configured storage root using camera/UTC paths. Transactional runtime configuration supplies the validated duration, memory/sample bounds, root, clock policy, and identity; the accelerated fixture produces three playable 30-second segments.
- [x] Ensure the configured recordings directory is the sole recording root. `SegmentFileStore` rejects paths outside its owned `RecordingLayout` before I/O.
- [x] Finalize files atomically where the media/container library permits. The store exclusively creates `.partial.mp4`, flushes and syncs it, renames atomically, and syncs the parent directory without overwriting a durable target.
- [x] Enforce the segment lifecycle contract. The recorder drains init data and complete bounded one-second fMP4 fragments into an exclusively created `ActiveSegmentFile`; each emission is synced, normal completion probes and atomically publishes the file, append/finalization errors publish typed failure facts, and process death leaves discoverable partial evidence. No complete recording file is retained in memory.
- [x] Derive start, end, codec, resolution, and size from actual media/file state. The runtime applies epoch-aware media-to-UTC mapping; FFprobe supplies codec/dimensions/duration and filesystem metadata supplies size, with no fabricated defaults in the new path.
- [x] Supervise tasks and implement graceful shutdown that finalizes or explicitly marks an incomplete segment. The joint application owner concurrently polls camera and recorder, stops and joins the sibling on either exit, and orders ingest closure before recorder drain. The composed-process fixture proves graceful SIGINT and abrupt SIGKILL behavior.
- [x] Add structured logs and metrics for state, packets, queue depth/drops, segments, bytes, and failures. The server exposes loopback-only `/live`, camera-dependent `/ready`, and Prometheus text `/metrics`; counters cover lifecycle, access units, packet loss, teardown, queue bounds/outcomes, segments, clock epochs, and bytes without media retention or credential labels.

### Tests

- [x] Unit tests for identifiers, state transitions, timestamp conversion, path construction, and configuration validation. Consolidated validation tests include a valid transaction, embedded-secret rejection, and calculated memory-budget rejection.
- [x] Integration test from deterministic RTSP fixture to multiple playable segments. Both the direct runtime and the actual composed server produce three authenticated RTSP-derived 30-second durable fMP4 files; the server receives SIGINT, exits cleanly, and every file fully decodes.
- [x] Codec tests cover every Phase 2 supported combination: video-only H.264/AVC without B-frames. H.265, audio, and H.264 composition offsets are explicitly unsupported rather than silently accepted; Phase 0 spikes retain evidence for future qualification.
- [x] Slow-recorder/queue-saturation test proving bounded memory and visible failure behavior. Queue contracts preserve recorder backlog, and the real supervisor fixture stops visibly on saturation.
- [x] Source EOS and malformed-stream tests. Publisher loss drives EOF/reconnect recovery; RFC 6184 tests reject malformed, truncated, oversized, and discontinuous payloads.
- [x] Graceful and forced shutdown tests, including incomplete-segment reconciliation behavior. Joint-owner tests cover camera-first, recorder-first, and requested shutdown; the actual server passes SIGINT finalization; SIGKILL leaves a decodable partial built from synced complete fragments; restart discovers it as reconciliation evidence without promoting or deleting it.
- [x] Manual Mac webcam smoke test through local MediaMTX. The documented AVFoundation → FFmpeg → MediaMTX → RTSP copy-remux smoke test passed in Phase 0.

### Exit gate

- [x] Exactly one upstream connection exists for the camera. The runnable application constructs one supervisor/session and passes its distributor to the recorder; adding health or lifecycle consumers does not create a source connection.
- [x] At least three consecutive segments are playable and metadata matches their contents for the synthetic H.264 qualification source. Representative-camera qualification remains Phase 0's external gate.
- [x] Memory remains bounded under a deliberately slow consumer. The real supervisor saturation fixture stops visibly; validated configuration rejects the queue plus current-and-emitted-fragment worst-case byte bound above the operator budget, while storage independently bounds the file.
- [x] Graceful shutdown is deterministic in unit and actual-server fixtures. Forced exit preserves a decodable active partial, and restart discovers it through the bounded reconciliation scan.
- [x] No fabricated media metadata remains in the new path.

---

## Phase 3 — Multi-Camera Supervision and Failure Isolation

### Objective

Run independent camera pipelines so one camera or consumer failure cannot terminate the server or unrelated recordings.

### Instructions

- [x] Implement a supervisor owning per-camera pipeline lifetimes and join results. The bounded command owner retains validated configuration, cancellation senders, health readers, supervisor-owned operation joins, and terminal outcomes; shutdown joins every pipeline.
- [x] Add per-camera connection state, health timestamps, reconnect counters, and last actionable error. Credential-free bounded registry snapshots include pipeline/connection state, last-connected/media times, bounded failure categories, recorder completion/failure counts, and queue state.
- [x] Implement bounded exponential reconnect backoff with jitter, reset behavior, and cancellation. Backoff resets only after the configured 30-second stable-session interval; unit tests and MediaMTX stop/start qualification prove reset/cancellation behavior.
- [x] Make camera configuration additions/removals explicit lifecycle operations. Add/start/stop/remove are typed, bounded commands; removal replies only after cancellation and join.
- [x] Ensure consumer failure is contained and restarted or marked degraded according to policy. Recorder exit/backpressure stops and degrades only its camera; explicit start is the reversible restart policy.
- [x] Ensure API failure does not cancel recording tasks and recording failure does not terminate the API. The API listener is an independently joined sibling and focused tests prove listener failure leaves a running pipeline owned.
- [x] Add `/api/cameras`, `/api/cameras/{id}`, and `/api/cameras/{id}/health` backed by domain state. Loopback-only lifecycle endpoints validate bounded JSON and return stable credential/path-free resources.
- [x] Use bounded-cardinality camera labels in metrics. Only validated camera IDs and fixed consumer labels are emitted across all registry entries.

### Tests

- [x] Two-camera and ten-camera synthetic integration fixture (`scripts/phase3/multi-camera-fixture.sh`). Representative 4 MP sources remain an external gate.
- [x] Disconnect one source while all other recordings continue. Camera B completed another segment with zero reconnects while A was offline.
- [x] Reconnect repeatedly without leaked tasks, file descriptors, or duplicate upstream connections. Existing repeated reconnect fixture plus the ten-source fixture count one initial reader per camera and one reader for each qualified reconnect; every owned task joins. Extended OS-level soak remains part of the representative gate.
- [x] Kill/restart a recorder consumer without affecting ingestion policy beyond documented backpressure behavior. Recorder-first and saturation tests prove camera-local stop/degradation while unrelated fixture cameras continue.
- [x] Stop the API task and prove camera pipelines remain alive under supervision.
- [x] Test configuration removal during connecting, connected, and reconnecting states. Focused command tests cover connecting/starting concurrency; the MediaMTX fixture removes connected and reconnecting cameras and verifies unrelated health remains connected.

### Exit gate

- [!] Ten representative 4 MP/15 FPS/High Profile sources run for an agreed soak period. External cameras, target hardware, and agreed duration are unavailable; the ten-source 640x360 synthetic orchestration run passed and is not a product-capacity claim.
- [x] Failure of camera N does not interrupt N-1 other pipelines for the deterministic synthetic workload, including source outage, invalid credentials, dimension mismatch, recorder backpressure, and API listener failure.
- [x] No detached tasks or unowned child processes remain in the implemented runtime. Pipeline, supervisor operation, API, and process tasks all have cancellation and join policy; fixture shutdown finalizes viable segments and matches every too-short retained partial with an explicit shutdown-failure fact.
- [x] State and recovery are observable per camera through the bounded registry, camera API, health/readiness, and Prometheus metrics.

---

## Phase 4 — Recording Index, Time-Range Retrieval, and Retention

### Objective

Make recordings queryable and maintainable through durable metadata rather than filesystem scans.

### Instructions

- [x] Add PostgreSQL migrations and repository contracts for cameras and recording segments. SQLx migrations and the real PostgreSQL fixture enforce the Rust lifecycle model.
- [x] Store lifecycle status so open/incomplete/completed/failed files are distinguishable. Completed, failed, incomplete, inconsistent, deletion-pending, and deleted evidence is constrained and idempotent.
- [x] Commit file and metadata changes in an order with a defined crash-recovery story. Durable rename/probe precedes asynchronous metadata delivery; filesystem evidence drives repair.
- [x] Implement startup reconciliation for completed files missing metadata and metadata pointing to absent/incomplete files. The owned bounded pass also handles corrupt durable media and stale partials continuously.
- [x] Query all segments overlapping `[start, end)` by camera with appropriate indexes. The fixture records an index-only overlap plan.
- [x] Implement camera-scoped recording-search API using UTC/RFC 3339 boundaries. Results and detail resources never expose storage locators.
- [x] Define playback behavior for keyframe alignment, remuxing, and when accurate transcoding is necessary. ADR 004 records the reversible segment-aligned H.264 stream-copy policy.
- [x] Assemble or stream ranges spanning multiple segments. The three-segment output fully decodes.
- [x] Implement configurable retention from indexed metadata. Versioned restart-applied defaults and bounded oldest-first claims are implemented.
- [x] Add storage warning/critical watermarks, cleanup policy, and recent-footage protection. Watermark hysteresis and claim-before-unlink recovery are enforced.
- [x] Define bounded behavior while PostgreSQL is temporarily unavailable. Recording remains active, queue saturation is visible, and durable orphan catch-up is proven.

### Tests

- [x] Property-style exhaustive tests for overlapping half-open time ranges and offset-to-UTC normalization.
- [x] Retrieval across three or more segments.
- [x] Missing file, corrupt file, overlapping segment, and metadata-gap cases.
- [x] Database unavailable while durable media is published and successful later reconciliation.
- [!] Retention boundary, deletion failure, warning, critical, and disk-full simulations. Boundaries, hysteresis, protected footage, interrupted deletion, missing files, and exact bytes are covered; target-filesystem permission and disk-full injection require the deployment filesystem and remain an external operational gate.
- [x] Path traversal and cross-camera authorization-preparation tests.

### Exit gate

- [x] Production APIs do not scan directories to answer recording queries.
- [x] A multi-segment request returns the correct interval under the documented accuracy policy.
- [x] Retention never deletes outside the configured recording root and preserves policy-protected recent footage.
- [x] Database interruption does not immediately stop recording and reconciliation is proven.

---

## Phase 5 — Independent LiveKit Publication

### Objective

Publish live camera feeds with low latency without coupling LiveKit availability to ingestion, recording, or playback.

### Instructions

- [x] Record an ADR for codec/pass-through/transcode behavior and LiveKit deployment assumptions.
- [x] Implement the publisher as a bounded media consumer.
- [x] Keep credentials outside client-visible configuration and redact them from telemetry.
- [x] Define publisher connection state, retry policy, queue/drop policy, and subscriber observability.
- [x] Issue development browser access through subscribe-only LiveKit tokens rather than camera credentials; production authorization remains a Phase 8 gate.
- [x] Expose per-camera publication state through API/metrics.

### Tests

- [x] Publisher starts/stops without adding an upstream camera connection.
- [~] LiveKit startup outage and bounded slow writes do not interrupt recorder admission; real server stop/restart while recording remains to be deterministically qualified.
- [~] Shutdown leaves no publisher process; duplicate-track recovery after real server restart remains unverified.
- [!] Measure glass-to-glass or a named pipeline boundary at p50/p90/p99 under realistic viewers. The pinned CLI emitted clock-skew evidence, so no latency claim is accepted.

### Exit gate

- [!] H.264 reaches a real LiveKit subscriber, but full subscriber decode, representative High Profile cameras, browsers, and target concurrency remain external/local qualification gates.
- [~] LiveKit startup failure affects only live viewing; deterministic kill/restart recovery remains open.
- [!] Latency is not yet accepted: no product objective exists and the CLI sender-report clock-skew warnings invalidate a forwarding-latency claim.

---

## Phase 6 — Human Detection and First-Class Domain Events

### Objective

Add sampled human detection as an independently degradable media consumer and publish structured, bounded domain events.

### Instructions

- [x] Decode frames once for detection consumers; do not alter the recorder's encoded path.
- [x] Implement configurable sampling based on media timestamps, not executor timing alone.
- [x] Favor fresh frames under overload; expose dropped/skipped counts and consumer lag.
- [x] Define normalized bounding-box, confidence, model-version, camera, and timestamp types.
- [x] Implement a bounded in-process event bus with explicit ordering scope, delivery guarantees, overflow behavior, subscription lifetime, and shutdown.
- [x] Separate immutable past-tense events from commands.
- [x] Publish camera, recording, storage, and detection lifecycle events without transporting media payloads.
- [x] Persist externally relevant event versions where required.

### Tests

- [x] Sampling accuracy for variable FPS and timestamp discontinuities.
- [x] Detector saturation test proving bounded memory and preference for recent frames.
- [x] Detector crash/unavailability while recording and live viewing continue.
- [x] Event ordering and overflow tests for each event class.
- [x] Subscriber crash/unsubscribe and graceful shutdown tests.
- [!] Accuracy evaluation on an agreed labeled dataset, separated from systems throughput benchmarks. Production model/dataset/criteria remain external product gates; deterministic systems workload evidence is recorded separately.

### Exit gate

- [x] Structured person detections are camera- and timestamp-scoped.
- [x] Slow detection cannot grow memory without bound or stop recording.
- [x] No packet/frame appears on the domain event bus.
- [x] Event semantics are documented well enough to evaluate a future external-broker adapter honestly.

---

## Phase 7 — Persistent Configuration, Rules, ROI, and Analytics

### Objective

Make operational behavior safely configurable and add derived business behavior without coupling it to producers.

### Instructions

- [ ] Implement precedence among defaults, configuration files, persisted settings, environment secrets, and API changes.
- [ ] Validate settings through domain types and report whether each change applies immediately, at a safe boundary, or after restart.
- [ ] Apply related changes transactionally; reject partial invalid configuration.
- [ ] Add authenticated settings APIs with secret-safe read behavior and audit events.
- [ ] Define ROI polygon coordinate system, validation, versioning, and camera association.
- [ ] Implement rules as event consumers with typed conditions and actions.
- [ ] Keep alert/dashboard/webhook policy out of detection code.
- [ ] Implement only the confirmed heatmap/occupancy semantics.
- [ ] Add tracking only if the confirmed analytics require identity over time.

### Tests

- [ ] Configuration round-trip, precedence, invalid update, restart persistence, and rollback tests.
- [ ] Concurrent configuration read/update and safe application-boundary tests.
- [ ] Property tests for ROI polygons and normalized coordinates.
- [ ] Rule matching, non-matching, duplicate event, action failure, and replay/idempotency tests.
- [ ] Analytics correctness against fixed event fixtures.

### Exit gate

- [ ] No expected-to-change operational value is hardcoded in core pipeline logic.
- [ ] Invalid configuration cannot become active.
- [ ] ROI/rule/analytics behavior matches confirmed product semantics.
- [ ] Action failure does not corrupt event production or media processing.

---

## Phase 8 — Security, Operability, and Production Readiness

### Objective

Close the cross-cutting gaps required to operate the 10-camera system safely and recover it under failure.

### Instructions

- [ ] Threat-model camera, API, database, filesystem, LiveKit, dashboard, and deployment trust boundaries.
- [ ] Add authenticated users/service identities and camera-scoped authorization.
- [ ] Terminate TLS directly or document and enforce a trusted reverse-proxy boundary.
- [ ] Store secrets outside ordinary configuration responses; define rotation and file permissions.
- [ ] Add audit logging for configuration, access, camera, recording export, and authorization changes.
- [ ] Distinguish liveness, readiness, and per-camera degradation.
- [ ] Export Prometheus-compatible metrics and create dashboards/alerts for critical failure modes.
- [ ] Package the service for the confirmed Linux target with restart and resource limits.
- [ ] Write deployment, upgrade, rollback, backup/restore, disk-full, camera-offline, database-outage, and LiveKit-outage runbooks.
- [ ] Add dependency policy with `cargo audit`/`cargo deny` and document system-library patching.
- [ ] Fuzz externally supplied parsers and range/configuration inputs where useful.

### Failure qualification

- [ ] Camera disconnect and flapping.
- [ ] Network partition.
- [ ] Detector and LiveKit process failure.
- [ ] PostgreSQL outage and recovery.
- [ ] Disk warning, critical, full, and write error.
- [ ] Abrupt machine/process restart during segment finalization.
- [ ] Invalid/expired credentials and unauthorized cross-camera access.
- [ ] Graceful SIGTERM under active recording and API requests.

### Exit gate

- [ ] An operator can diagnose and recover the defined failures from documented telemetry and runbooks.
- [ ] Secrets are neither logged nor returned to unauthorized clients.
- [ ] Upgrade and rollback are demonstrated.
- [ ] Critical alerts are actionable and tested.

---

## Phase 9 — Capacity Qualification and Evidence-Led Scaling

### Objective

Prove the 10-camera target, optimize for 20 cameras, and identify the first measured bottleneck before changing deployment topology.

### Workload matrix

Record for every run:

- Camera count, resolution, FPS, codec/profile, bitrate mode/range, audio.
- Segment duration and storage/filesystem hardware.
- Detection model, sample FPS, input size, batch behavior, and GPU.
- Viewer count and viewed camera distribution.
- Test duration, reconnect/failure injection, and software versions.

Measure CPU, GPU, RAM, allocations where relevant, disk MB/s/latency/capacity, network Mbps/loss, queue depth/drops, reconnects, recording gaps, and p50/p90/p99 latencies.

### Instructions

- [ ] Establish an unoptimized one-camera baseline.
- [ ] Qualify 10 realistic cameras through a sustained soak with recording, live viewing, and detection enabled.
- [ ] Profile the limiting workload; fix correctness and topology problems before micro-optimizing.
- [ ] Change one variable at a time and retain optimizations only with reproducible improvement.
- [ ] Qualify 20 cameras or document the exact measured constraint preventing it.
- [ ] Test degraded modes under overload and prove bounded memory.
- [ ] Model 30-day storage using measured bitrate distributions and safety margin.
- [ ] Document extraction criteria for workers, recording nodes, LiveKit nodes, or an external event broker.

### Exit gate

- [ ] The supported-camera claim names the exact qualified workload and hardware.
- [ ] Ten-camera production target passes reliability and performance objectives.
- [ ] Twenty-camera result is measured, not inferred.
- [ ] Any proposed distribution step addresses an observed bottleneck and has an ADR with operational cost and reversal criteria.

---

## Deferred Scale Phases

Only after Phase 9 evidence:

- **50–100 cameras:** consider extracting the measured heavy workload—typically detection, recording, or LiveKit—not the entire monolith.
- **100–200+ cameras:** introduce scheduling, placement, storage tiering, and external messaging only with explicit delivery and consistency requirements.
- Re-run failure, security, capacity, and cost qualification after every topology change.

## Definition of Done for Every Roadmap Item

An item is done only when:

- The requirement and acceptance criterion are explicit.
- The implementation compiles with required features.
- Success, boundary, failure, recovery, and shutdown paths are tested as applicable.
- Queues, task ownership, cancellation, and overload policy are known.
- Logs and metrics make the behavior diagnosable.
- Security and secret handling were considered.
- Performance claims include workload and measurements.
- Documentation and the latest context snapshot reflect the result.
- Remaining limitations are stated without euphemism.

## Context Snapshot Maintenance

At the end of a material work session or phase gate:

1. Create `context/YYYY-MM-DD/` or, if multiple snapshots are needed that day, `context/YYYY-MM-DD-HHMM/` using local project time.
2. Copy the topic template from `context/README.md`.
3. Record repository facts from commands/tests, not memory.
4. Separate decisions, assumptions, and open questions.
5. State the next executable action and its prerequisites.
6. Update `context/README.md` so `Latest snapshot` points to the new directory.
7. Never rewrite an old snapshot to make history look current; correct it in a newer snapshot.
