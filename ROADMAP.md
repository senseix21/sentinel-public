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
- [x] Define and test the first SDK-neutral H.264 access-unit contract: validated depayloaded NALs, explicit marker/timestamp assembly, multi-slice grouping, SPS/PPS and IDR classification, malformed-input rejection, and discontinuity reset. Direct encoded publication *is* available in a released official LiveKit Rust API (`livekit` 0.8, `NativeVideoSource::new_encoded`); the earlier claim that it was not is corrected here. It is implemented and unusable on Apple targets — see ADR 011 Decision 5.
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

- [x] Capture the existing manifest failure as baseline evidence. Recorded in
  `context/2026-08-10/repository-status.md`: `cargo check` failed with
  `error inheriting ... from workspace root manifest` and `failed to find a
  workspace root`.
- [x] Replace orphaned `workspace = true` dependencies with a real root `[workspace]` and `[workspace.dependencies]` containing pinned compatible versions.
  Sixteen members; every shared dependency is pinned with `=`.
- [~] Introduce `apps/server` and only the crates needed by the first vertical slice: `common`, `config`, `camera`, `media`, `recorder`, `storage`, `events`, and minimal `api`.
  Every named crate exists and carries the slice. Later phases legitimately added
  `auth`, `database`, `detector`, and `livekit`, which are implemented. But
  `analytics`, `rules`, and `apps/worker` were introduced ahead of their phases
  and hold no implementation, and `api` was superseded in place. See the exit
  gate.
- [x] Preserve the prototype in a clearly labeled location or migrate individual mechanics only when covered by characterization tests.
  `AGENTS.md` names the original `cam_rtsp` code in the repository root `src/`,
  with its known defects. It is not a workspace member and is not compiled.
- [x] Remove public hardcoded RTSP URLs and credentials from runtime defaults and documentation examples.
  The only credentialed URLs remaining are fixtures inside `#[cfg(test)]`
  modules; `config/example.toml` names environment variables, never values.
- [x] Establish workspace lint policy without blindly enabling lints that conflict with FFI/media code.
  `unsafe_code` is forbidden; clippy `all` and `pedantic` warn at workspace
  level with no blanket allow list. Because the gate denies warnings, the
  compiler is part of that policy: `rust-toolchain.toml` pins 1.92.0, after CI
  on a 1.98 runner failed an unchanged tree on a lint that did not exist
  locally.
- [x] Add CI for format, clippy, tests, and release build.
  `.github/workflows/ci.yml` runs `make verify` on `macos-latest` for every push
  and pull request. `make check` keeps its meaning and is now `verify` plus
  `audit`; `.github/workflows/audit.yml` runs the advisory half daily and on
  demand, so a newly published advisory does not red an unrelated pull request.
- [~] Record required GStreamer/FFmpeg system packages for macOS development and target Linux deployment.
  macOS prerequisites are in `README.md` and checked by `make prerequisites`.
  The target Linux packages cannot be recorded while the deployment
  distribution is unconfirmed (Phase 8). GStreamer is used only by the
  uncompiled prototype.
- [x] Add a configuration example with safe local defaults and no secrets.
  `config/example.toml` binds loopback, leaves the mutating surface disabled,
  and states that credentials come from the Phase 8 secret boundary.

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

- [x] A fresh checkout builds using documented prerequisites. Verified on
  2026-10-01 by cloning to an empty directory and running `make verify` with no
  inherited build artifacts: 504 tests passed, 0 failed, across 42 targets. One
  caveat is recorded rather than hidden: a cold build must download a prebuilt
  WebRTC archive from GitHub releases inside the `webrtc-sys` build script,
  which hard-unwraps its HTTP result. That fetch failed three times on
  2026-10-01 (timeout, then connection reset) and succeeded on retry, so a cold
  build is network-dependent and not hermetic. See
  `context/2026-10-01-2135/repository-status.md`.
- [x] All required checks pass. `make check` passed on 2026-10-01: 504 tests
  passed, 0 failed, 30 ignored across 42 targets; Clippy with warnings denied;
  release workspace build; `cargo deny` advisories, bans, licences, and sources
  ok; `cargo audit` over 461 crates.
- [!] No empty speculative crates exist. Four members hold no implementation:
  `crates/analytics` (8 lines), `crates/api` (12), `crates/rules` (8), and
  `apps/worker` (5). `analytics` and `rules` are blocked on the unconfirmed
  heatmap/analytics semantics and rule conditions already recorded in Phase 0
  and Phase 7; `apps/worker` is reserved until Phase 9 measurements justify
  extraction; `crates/api` is vestigial because the HTTP translation boundary
  was implemented in `apps/server/src/health_http.rs`. Closing this needs a
  decision to implement, remove, or re-scope, not a code change alone.
- [x] Prototype behavior and known defects remain documented in the latest context snapshot.
  Carried forward in `context/2026-10-01-2135/repository-status.md`.

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
- [x] Replace the `lk` CLI child process with in-process native encoded publication (`livekit` 0.8 `NativeVideoSource::new_encoded` / `capture_encoded_frame`). The child-process boundary is removed, not left in parallel. ADR 011.
- [!] Run native encoded publication on the development platform. Implemented, and refused at runtime on Apple targets: `webrtc-sys` 0.3.42 builds the first encoder from the negotiated SDP format, which carries no backend tag, so the requested pass-through backend is never selected and VideoToolbox dereferences the pre-encoded payload as a pixel buffer. Measured four times: exactly 9 access units published, then `SIGSEGV` on the `EncoderQueue` thread. Blocked on upstream; ADR 011 Decision 5.
- [x] Publish live video that a browser actually decodes, by decoding and re-encoding behind a viewer gate (ADR 012). `TranscodePublicationAdapter` implements the same `PublicationAdapter` contract as the pass-through adapter, so the owner, health model, failure categories, backoff, and the one-upstream-connection rule are untouched; `apps/server/src/pipeline.rs` selects it, and `NativePublicationAdapter` is kept rather than deleted so reversal is a one-line constructor change. Nothing decodes while `surveillance_livekit_viewers` is 0.
- [x] Make the transcode path's own behaviour observable. `PublicationAdapter::observe` returns three numbers the owner cannot derive — viewers, media produced, internal restarts — surfaced as `surveillance_livekit_viewers` (a gauge), `surveillance_livekit_published_frames_total`, and `surveillance_livekit_adapter_restarts_total`. The restart counter earned itself immediately: it exposed one `Overloaded` restart on every subscribe, which was FFmpeg process startup rather than backpressure (ADR 012 Decision 5, amended). Split write budgets fixed it; measured 0 restarts over a 60 s session afterwards.

### Tests

- [x] Publisher starts/stops without adding an upstream camera connection.
- [~] LiveKit startup outage and bounded slow writes do not interrupt recorder admission; real server stop/restart while recording remains to be deterministically qualified.
- [~] Shutdown leaves no publisher process; duplicate-track recovery after real server restart remains unverified.
- [x] A non-retryable adapter failure stops publication once instead of reconnecting forever, and recording continues. Covered by `a_non_retryable_send_failure_stops_publication_instead_of_reconnecting`, and measured live on 2026-08-16: `livekit_connection_attempts_total=1`, `send_failures_total=1`, `state=8` (Stopped), `camera_connection_epochs_total=1`, `recorder_segments_completed_total=8`. Evidence: `docs/phase-5/local-evidence-2026-08-16.md`.
- [~] Measure glass-to-glass or a named pipeline boundary at p50/p90/p99 under realistic viewers. Camera-to-glass remains refused: the transcode path reports `TimestampAuthority::AdapterClock`, because B-frame reordering makes the frame-to-access-unit mapping unrecoverable. A repeatable authenticated Chrome harness now measures Watch-to-first-rendered-frame and frame gaps. On the 2026-08-19 local 640x360@15 fixture, five reconnects within the five-second warm grace had 779 ms p50, 1,014 ms p90, no timeout, and a 233 ms maximum observed frame gap; five cold starts had 4,463 ms p50 and 5,407 ms p90. The first frame can be libwebrtc's black keepalive, so these are render-start figures, not usable-camera-picture or glass-to-glass figures. Representative cameras, target hardware, confirmed objectives, and a larger sample remain open. Evidence: `docs/phase-9/livekit-low-latency-2026-08-19.md`.

### Exit gate

- [~] A real browser subscriber decodes and renders the live stream. Measured 2026-08-17 on the local stack: the dashboard's `<video>` element reports `1280x720`, `readyState` 4, advancing `currentTime`, and 416 frames through `getVideoPlaybackQuality()`. Still open: representative High Profile cameras, browsers other than the Chromium the check ran in, H.265, and target concurrency — every figure here is one camera and one subscriber.
- [x] Published-versus-source frame rate is lossless under the qualified local workload. On 2026-08-18 the real Mac webcam produced independently probed 450-frame, 30.000 s segments (15.000 FPS), while an attached subscriber saw both `access_units_total` and `published_frames_total` advance 301 frames over the same timestamped 20 s sample (15.05/s), with zero stale drops and live evictions. Controlled 1280x720@15 runs published 926/926, 927/927, and 928/928 frames with 1, 2, and 4 viewers. This closes frame accounting, not time to first picture, browser rendering on representative cameras, or viewer-perceived latency. Evidence: `docs/phase-9/live-publication-qualification-2026-08-18.md`.
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

- [x] Implement precedence among defaults, configuration files, persisted settings, environment secrets, and API changes. `defaults < file < persisted < environment`; an API change writes the persisted layer, and a change the environment shadows reports `PersistedButShadowed` rather than a bare success. `crates/config/src/resolution.rs`, `environment.rs`, `file.rs`.
- [x] Validate settings through domain types and report whether each change applies immediately, at a safe boundary, or after restart. 79 settings classified by a table macro that cannot compile with an axis missing; boundaries reflect what each owner actually does, so only the two settings the HTTP adapter re-reads are `Immediate`. `crates/config/src/classification.rs`, `change.rs`.
- [x] Apply related changes transactionally; reject partial invalid configuration. Coupled groups re-validate through the same constructors the pipeline uses, so a batch cannot be accepted here and rejected at pipeline start. No coercion. `crates/config/src/change.rs`, `assemble.rs`.
- [~] Settings APIs with secret-safe reads and audit records exist behind a single `authorize` chokepoint, with the mutating surface default-off. **Not authenticated**: Phase 8 owns the identity model, and `authorize` grants on loopback until then. Secrets are neither persisted nor API-editable while ADR 003's secret store is open. `apps/server/src/authorize.rs`, `settings_service.rs`, `health_http.rs`.
- [!] Blocked on confirmed coordinate semantics. `crates/database/src/rois.rs` remains a stub; no invented coordinate system.
- [!] Blocked: the mechanism is implementable, the conditions are not known. Building it against invented conditions produces a rules engine nobody asked for.
- [!] Blocked with the rules item. No alert, dashboard, or webhook policy exists anywhere, which trivially satisfies the constraint but does not demonstrate it.
- [!] Blocked: the meaning of a heatmap — density, occupancy, or movement — is unconfirmed.
- [!] Blocked on the analytics decision above.

### Tests

- [x] Round-trip, precedence, invalid-update, restart-persistence, and rollback all covered. Restart persistence and audit rollback run against a real database in `scripts/phase7/config-fixture.sh`.
- [!] Application-boundary reporting is tested; **concurrent update behavior is not**. The transaction boundary and generation sequence make the design defensible, but ADR 008 forbids inferring transactional safety from code structure. Needs two concurrent writers against one database.
- [!] Blocked with ROI.
- [!] Blocked with rules.
- [!] Blocked with analytics.

### Exit gate

- [x] Eleven hardcoded values moved into the configuration domain, including `StorageWatermarks::new(80, 90, 97, 2)`. The environment-only retention loader was deleted rather than left as dead API, and its test was ported, not removed. See `docs/phase-7/local-evidence-2026-08-15.md`.
- [x] Rejection is whole-batch and names the offending setting or group; there is no coercion path. The schema refuses credential keys and malformed audit rows independently of the application.
- [!] Cannot close while the product gates above are open. Not reported as closed.
- [~] Configuration-change notification failure is proven not to roll back a committed change, and audit and notify outcomes are counted separately. Rule *actions* do not exist, so the gate's subject is only partly present.

---

## Phase 8 — Security, Operability, and Production Readiness

### Objective

Close the cross-cutting gaps required to operate the 10-camera system safely and recover it under failure.

Decisions settled in ADR 009 on 2026-08-15: bearer tokens for human and service
identities, all API reads authenticated with `/live`/`/ready`/`/metrics` open,
a reverse proxy terminating TLS with proof of origin, a permission-enforced
secrets file, and RPO one segment / RTO 15 minutes. The **deployment target
remains unconfirmed**, which blocks items 8 and 9 in part.

### Instructions

- [x] Threat-model camera, API, database, filesystem, LiveKit, dashboard, and deployment trust boundaries. Seven boundaries in `docs/phase-8/threat-model.md`, each naming what crosses, who is trusted, what an attacker in that position can do, and the control. Six accepted risks are recorded with reasons and revisit conditions rather than left implicit.
- [x] Add authenticated users/service identities and camera-scoped authorization. `crates/auth` owns principals, roles, camera scope, and Argon2id-hashed bearer tokens; `migrations/0003_auth.sql` refuses any non-Argon2id credential and any token without an expiry at the schema level. `authorize` now decides surface → shutdown → authentication → role → camera scope, and out-of-scope answers `404` so scope cannot enumerate the estate. `every_protected_route_reaches_the_chokepoint` covers every entry in the protected-route table, now including `GET /api/system`, and `the_playback_stream_reaches_the_chokepoint` covers the playback stream, which enters above the router and was initially missed. Browser identity is settled by **ADR 009 Decision 6**: the reverse proxy authenticates the human and asserts the principal in `X-Authenticated-Principal`, which is read only after proof of origin holds. The proxy asserts *who*, never what they may do — role and camera scope still come from the local `principals` table, so an asserted principal that is unknown or suspended here is refused. Six socket-level tests drive the real listener; each was verified to fail when its fix is reverted.
- [x] Terminate TLS directly or document and enforce a trusted reverse-proxy boundary. Reverse proxy per ADR 009 Decision 3, with proof of origin as a peer allowlist **and** a constant-time shared-secret header; the rightmost `X-Forwarded-For` entry is taken, and a failed proof discards the forwarded value rather than defaulting. `HealthBindNotLoopback` now relaxes only when a trust boundary and an enabled principal both exist, enforced as validation in `security_config::resolve_bind`.
- [x] Store secrets outside ordinary configuration responses; define rotation and file permissions. `crates/config/src/secret_store.rs` refuses a file wider than `0600` and reports the observed mode; per-key environment override; per-camera credentials expressible as `camera.<id>.rtsp_password`, which the Phase 7 environment layer could not express. Ownership enforcement is **not** implemented — it needs a `libc` call that `unsafe_code = "forbid"` rules out — and is recorded as a residual in `docs/phase-8/SPEC.md`.
- [x] Add audit logging for configuration, access, camera, recording export, and authorization changes. `migrations/0004_access_audit.sql` plus `crates/database/src/access_audit.rs`, written at the chokepoint by `authorize_audited` so every protected request is covered by construction. All five categories now have a route that produces them: configuration from Phase 7's same-transaction trail, access and camera from the chokepoint, **export** because `/recordings/playback` is authorized as `ExportRecording` and followed by a row carrying camera, exact range, and byte count, and **identity** from the new `/api/identities` surface — creating principals and minting or revoking tokens was previously only possible through raw SQL, which no audit could observe. Verified live: correct rows for grants, each denial reason, the full identity lifecycle, and export authorization, with zero silent audit failures. Categories and outcomes are checked by the database; export rows must carry principal, camera, exact range, and byte count together or be refused; sensitive detail is a salted fingerprint, never a value.
- [x] Distinguish liveness, readiness, and per-camera degradation. Already true from Phases 3–7; unchanged and re-verified.
- [~] Export Prometheus-compatible metrics and create dashboards/alerts for critical failure modes. `surveillance_auth_*`, `surveillance_authz_*`, and `surveillance_proxy_proof_*` are exported with proven-bounded cardinality and no identity in any label — and the proxy-asserted path adds one series (`surveillance_auth_proxy_asserted_total`) and one failure reason (`unknown_principal`), both asserted present and bounded by `security_metrics_have_bounded_cardinality_and_leak_no_identity`, confirmed live, and always present rather than omitted when no identity store is configured (an absent series would make every alert on it silently never fire). `docs/phase-8/alerts.yml` carries thirteen rules and `alerts_test.yml` a firing and a silent case for each critical one — **but `promtool` is not installed here, so the rule tests have not been executed.** Three Grafana dashboards now exist in `docs/phase-8/dashboards/` (fleet overview, per-camera detail, security), **unrendered** because no Grafana is available. What *is* checked is the failure that is otherwise silent: `scripts/phase8/check-metric-names.py` verifies every metric named by an alert or panel against a running server's `/metrics`, and it runs in the Phase 8 fixture. That check caught seven invented metric names in the first draft of `alerts.yml` — `surveillance_storage_used_ratio`, `surveillance_database_available`, `surveillance_camera_connection_state`, `surveillance_queue_depth`, `surveillance_configuration_stale`, `surveillance_livekit_publication_state`, `surveillance_export_bytes_total` — every one of which would have made its alert silently inert.
- [!] Package the service for the confirmed Linux target with restart and resource limits. Blocked: no confirmed distribution, unit type, storage layout, or upgrade owner. ADR 009 declines to guess one.
- [~] Write deployment, upgrade, rollback, backup/restore, disk-full, camera-offline, database-outage, and LiveKit-outage runbooks. The four target-independent runbooks are written in `docs/phase-8/runbooks/`. Deployment, upgrade, rollback, and backup/restore are blocked on the same unconfirmed target. Of the four written, three were exercised and the LiveKit one was not; none was executed by a separate operator following it literally, which is the standard the handoff sets.
- [x] Add dependency policy with `cargo audit`/`cargo deny` and document system-library patching. `deny.toml`, `.cargo/audit.toml`, and `make audit` in the gate; both pass. `docs/phase-8/dependency-policy.md` documents the FFmpeg/RTSP patching cadence and trigger. RUSTSEC-2023-0071 is ignored with verification that `rsa` is not in the build graph, not with a blanket exclusion.
- [!] Fuzz externally supplied parsers and range/configuration inputs where useful. Not implemented: `cargo-fuzz` needs a nightly toolchain, and adding one to a gate pinned to stable 1.88 with `unsafe_code = "forbid"` is its own decision. Targets are named and prioritised in `docs/phase-8/dependency-policy.md`. Existing hand-chosen rejection tests are not a substitute and are not reported as one.

### Operator dashboard (not a Phase 8 instruction; recorded because it ships)

- [~] A framework-free browser dashboard exists at `GET /` and `GET /ui`.
  Its shell, four CSS files, composition root, domain modules, components,
  nine screen modules, and vendored LiveKit SDK are compiled into the Rust
  binary and served only through an exact static registry with same-origin and
  `no-store` behavior. Node's test runner executes focused metrics, playback,
  routing, state, theme, camera-not-found, and LiveKit lifecycle tests through
  `make check`; Rust tests exercise every registered route, MIME type, exact
  body, unknown path, and traversal-shaped path instead of inspecting
  concatenated JavaScript text. On 2026-08-19 the authenticated fixture served
  every asset and primary API with `200`, and headless Chrome executed the real
  module graph and rendered a non-empty dashboard. Still `[~]`: the browser
  walk covers one Chromium build and one synthetic camera, not representative
  cameras, multiple browsers, or target viewer concurrency.
- [~] Driven in a browser against the local webcam loop on 2026-08-15 (webcam →
  FFmpeg → MediaMTX → Retina → 60 indexed segments across four real gaps →
  proxied dashboard). Three defects that the byte-level tests could not reach
  were found and fixed: a silent dead player on a gapped range, a browser-cached
  dashboard surviving a server upgrade, and playback boundaries truncated from
  microseconds to milliseconds so that every run playback was refused with `409
  recording_range_not_contiguous`. Each now has a mutation-verified test.
  Playback then verified live at full precision (`readyState 4`, 1280×720,
  advancing), and `seekable` measured as a zero-width range. Still `[~]`: one
  browser, one camera, one session, no automated browser test, and no
  multi-camera or concurrent-viewer measurement. Evidence in
  `docs/phase-8/ui-spec.md` §11.
- [~] Rebuilt to the reference design as nine screens with honest empty states,
  and a Live View that owns N concurrent LiveKit rooms rather than one (ADR 010
  Decisions 4 and 5). Driven in a browser on 2026-08-16 against
  `scripts/phase9/livekit-load-fixture.sh` with six cameras, and separately
  against a PostgreSQL-backed server where every `/api` call answers `401`.
  Defects found and fixed in that walk: the tile footer clipped its watch, stop
  and fullscreen controls at 5× density; camera identifiers in tables were real
  links rendered as plain text; the idle overlay kept painting over a decoding
  video because the user-agent's `[hidden]` rule loses to any class rule that
  sets `display`; "Watch all" joined rooms for cameras whose own tile offers no
  watch button; and — the one that mattered — an unreadable fleet rendered as
  `0 / 0 · no cameras configured` on five surfaces plus a System screen frozen
  on "Loading capabilities…", all of which asserted fleet facts a refused
  request never returned. Counts, tables, service rows, the trend series and
  the capability screen are now gated on whether the read succeeded; two
  mutation-verified tests lock it. Footer behaviour re-verified at 2×/3×/4×/5×:
  every footer one line, every control inside its box, the LiveKit words giving
  up room to a dot and a tooltip at 4× and above. Still `[~]`: one browser, one
  session, no automated browser test.
- [~] What N concurrent live tiles cost is now measured, not asserted:
  `docs/phase-9/live-grid-tiles-2026-08-16.md`. On an i9-9880H host each
  subscribed 1280×720 30 fps tile costs the browser tree ≈0.17 of one core and
  the SFU ≈0.046, while `surveillance-server` stayed between 4.5% and 5.9%
  across zero to six tiles — the subscriber side is the binding constraint and
  the ingest-once topology holds. **No tile-count claim is made**: only two
  homogeneous points were run, nothing above six was tried, and five of six
  tracks came from an `lk --publish-demo` publisher because `AddCameraRequest`
  carried no LiveKit field at the time of that run. That limitation was removed
  on 2026-08-16 (see the next item); the measurements above were not re-taken on
  the newer binary and are unchanged by it.
- [~] Playback and Settings driven through an authenticated session for the
  first time, on 2026-08-16:
  `docs/phase-9/authenticated-ui-2026-08-16.md`. Both screens call `/api/*`,
  which Phase 8 authenticates, and the dashboard holds no credential by design
  (ADR 009 Decision 6), so neither had ever been exercised — a proxy proving the
  trust boundary is the only way to reach them.
  `scripts/phase9/authenticated-ui-fixture.sh` stands that up and refuses to
  declare itself ready unless the proxied API answers `200` **and** the direct
  listener answers `401`. Four defects, three of them in shipped, reviewed code:
  every Playback search answered `400` because the page sent `from`/`to` while
  the route reads `start`/`end`; the add-camera form was wiped by the five-second
  repaint; every live tile went black on every poll because the `<video>` node
  was destroyed and the track re-attached to a cold one; and `api.io_timeout`
  cancelled any playback client that paces its reads, which is every browser.
  The last is fixed with a separate `api.playback_stall_timeout` (default `60s`,
  documented as a reversible guess, not a measurement) and confirmed by
  `surveillance_playback_cancelled_total`, not by inference. Verified in that
  run: a camera added through the form publishes to LiveKit — `AddCameraRequest`
  now carries `live_publication_enabled` — a playing tile survives five poll
  cycles as the same DOM node, Playback search returns 12 real segments whose
  displayed metadata `ffprobe` confirms against the files on disk, and the
  Settings write path accepts (`200`, generation `0 → 1`), refuses (`400
  invalid_value`, no generation change) and reverts (`200`, generation `2`).
  Still `[~]`, and the reason is specific: **playback does not play through
  inside the running dashboard page in headless Chromium and I could not explain
  why** — the server completes the stream and a same-page `fetch()` pulls all
  69.8 MB, yet the page's element stalls at ≈4 s and rewinds, while the same URL
  in a bare `<video>` on the same origin plays through. Headless main-thread
  contention is a hypothesis, not a finding. Playback is not to be described as
  working end to end in the UI until a real browser reproduces it.

### Failure qualification

- [~] Camera disconnect and flapping. Runbook written and exercised against synthetic sources; representative cameras remain unavailable.
- [!] Network partition. Not qualified: needs a multi-host environment that does not exist here.
- [~] Detector and LiveKit process failure. Detector failure has Phase 6 evidence; the LiveKit runbook is written but **unexercised** — no LiveKit server is available.
- [x] PostgreSQL outage and recovery. Exercised by stopping and restarting a local PostgreSQL; recording continued, `/ready` degraded, reconciliation restored the index. Restore-from-backup was **not** exercised, because the backup procedure is blocked on the deployment target.
- [~] Disk warning, critical, full, and write error. Watermarks and retention have Phase 4 evidence and the runbook is written; a genuine disk-exhaustion run on target hardware has not been done.
- [~] Abrupt machine/process restart during segment finalization. Phase 2 SIGKILL evidence stands; not re-qualified from the operator's side.
- [x] Invalid/expired credentials and unauthorized cross-camera access. Every `AuthFailure` variant is tested, cross-camera access answers `404`, and the exhaustive role×action matrix passes. The schema-level refusals are qualified against a real database in `scripts/phase8/security-fixture.sh`.
- [~] Graceful SIGTERM under active recording and API requests. Phase 2/3 shutdown evidence stands, and mutations are now refused while draining while reads continue; the combined case is not separately qualified.
- [!] Concurrent settings updates, overlapping and non-overlapping. Carried from Phase 7 and **still not tested**. Needs two concurrent writers against one database.
- [!] A boundary-scoped configuration change during live recording. Carried from Phase 7 and still unqualified.

### Exit gate

- [~] An operator can diagnose and recover the defined failures from documented telemetry and runbooks. Four runbooks exist against real telemetry, but none has been followed literally by a separate operator, and four more cannot be written. Not claimed as met.
- [x] Secrets are neither logged nor returned to unauthorized clients. `Secret<T>` redacts `Debug`/`Display`/`Serialize`; tokens never reach a log, a metric label, or an audit row; the store holds only Argon2id hashes; secret load failures never render the value. Tested in `crates/common`, `crates/auth`, `crates/config`, and `apps/server`.
- [!] Upgrade and rollback are demonstrated. Blocked on the deployment target.
- [!] Critical alerts are actionable and tested. Rules and tests are written; `promtool` is absent so nothing has been executed. An alert that has only been written is a belief, not a control.

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
- [~] Qualify the direct physical-camera RTSP topology through
  `docs/direct-camera-rtsp/SPEC.md` and its executable `PLAN.md`. The owned
  direct-RTSP harness, resource/metric capture, recording verification, evidence
  template, and camera-scoped metric gate are implemented. Execution remains
  blocked on representative cameras, disruption authority, target hardware, and
  approved objectives. MediaMTX retirement and the replacement FFmpeg webcam
  source are specified in `docs/ffmpeg-webcam-source/`; proxy-backed fixtures
  remain transitional until their individual behaviors have executable
  replacements.
  On 2026-09-01, `make check`, the
  authenticated Retina fixture, the synthetic 10-camera/two-camera isolation
  fixture, and the direct-report regression suite passed; these results validate
  the tooling baseline, not representative-camera interoperability or capacity.
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
