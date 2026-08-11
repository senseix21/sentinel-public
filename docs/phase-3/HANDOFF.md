# Phase 3 Handoff — Multi-Camera Supervision and Failure Isolation

Status: ready to implement

Baseline date: 2026-08-11

## Mission

Turn the qualified one-camera ingest-to-record vertical slice into multiple
independently owned camera pipelines in one process. A failure in camera N,
its recorder, or the API must not interrupt any other camera.

This phase extends the Phase 2 ownership model. It must not replace the Retina
ingestion path, introduce a second camera connection, or move media onto a
domain-event channel.

## Read first

Read these sources in order before changing architecture:

1. `AGENTS.md`
2. `context/README.md` and its latest snapshot
3. `ROADMAP.md`, Phase 3
4. `docs/requirements.md`
5. `docs/adr/001-media-pipeline.md`
6. `docs/adr/004-rtsp-client-adapter.md`
7. `apps/server/src/one_camera.rs`
8. `crates/camera/src/supervisor.rs`
9. `crates/camera/src/health.rs`

Read `analysis.md` when resolving an architectural ambiguity. It is the target
proposal, not a description of the current code.

## Verified starting point

Phase 2 currently provides:

- one authenticated or unauthenticated interleaved-TCP Retina session;
- explicit camera connection lifecycle and independent reconnect backoff;
- bounded recorder-preserving and live-freshness media queues;
- H.264 access-unit remuxing without pixel decode or re-encode;
- incrementally synced one-second fMP4 fragments inside 30–60 second files;
- atomic durable publication, actual FFprobe metadata, and crash partials;
- a structured owner joining camera and recorder on every exit path;
- loopback `/live`, `/ready`, and Prometheus `/metrics`;
- deterministic RTSP qualification for reconnect, saturation, SIGINT, SIGKILL,
  full segment decode, and startup reconciliation discovery.

The final Phase 2 gates passed: format, workspace Clippy with warnings denied,
71 ordinary tests, doc tests, release build, five ignored camera integration
tests, and one ignored recorder integration test.

## Current limitations that Phase 3 owns

- `apps/server/src/main.rs` constructs exactly one pipeline.
- `OneCameraRuntime` borrows configuration and storage dependencies; it is not
  yet an independently stored process-level pipeline owner.
- Health represents one camera rather than a bounded registry.
- The health listener and media pipeline are coordinated directly in `main`.
- No supervisor command contract exists for add, start, stop, or remove.
- No camera-scoped `/api` resource exists.
- No two-camera isolation or ten-camera soak fixture exists.
- Reconnect counters exist, but health timestamps, stable-session reset policy,
  and last redacted actionable error remain incomplete.

## Non-negotiable invariants

1. Exactly one upstream RTSP connection exists per running camera.
2. `CameraSupervisor` is the singular owner of every `CameraPipeline`.
3. Every spawned task has a shutdown signal, owner, and join/error policy.
4. Removing a camera cancels and joins it before removing registry ownership.
5. Camera and recorder failure are contained to their owning pipeline.
6. API or health-listener failure does not cancel recording pipelines.
7. Every queue is bounded and documents capacity, overload, closure, and metric.
8. Registry and metrics retain no video and expose no credential-bearing value.
9. Recorder failure provisionally stops only that camera's ingestion and marks
   it degraded; silently continuing to discard required footage is forbidden.
10. Terminal configuration/media failures do not enter an infinite retry loop.

## Target ownership

```text
ServerRuntime
├── CameraSupervisor task
│   ├── bounded command receiver
│   └── HashMap<CameraId, CameraPipelineEntry>
│       ├── validated camera configuration
│       ├── singular CameraPipeline owner
│       ├── shutdown sender
│       ├── health reader
│       └── owned join result
├── Camera API task
├── Health/metrics task
└── process shutdown owner
```

The API sends commands; it does not mutate the map or spawn pipelines. Health
reads snapshots; it does not become another task or media owner.

## Contracts to design before implementation

Do not freeze the exact types below without first testing cancellation and
concurrent command behavior. They describe required semantics, not prescribed
syntax.

### Supervisor commands

```text
AddCamera(validated config)
StartCamera(camera_id)
StopCamera(camera_id)
RemoveCamera(camera_id)
Shutdown
```

Every command needs a bounded request/reply lifetime and typed outcome. Define:

- duplicate add behavior;
- whether add starts immediately or creates a configured/stopped entry;
- idempotency of repeated start and stop;
- remove during connecting, connected, and reconnecting;
- command-channel capacity and full/closed behavior;
- response behavior if the caller is cancelled;
- whether stopped configuration remains inspectable.

### Pipeline states

At minimum distinguish:

```text
Configured -> Starting -> Running -> Stopping -> Stopped
                           |
                           `-> Degraded/Terminal
```

Camera connection state remains the existing lower-level state machine. Do not
collapse pipeline ownership state and RTSP session state into one ambiguous enum.

### Per-camera health

The snapshot must remain credential-free and bounded. It should eventually
include:

- `CameraId`;
- pipeline and connection states;
- connection attempts, epochs, reconnects, and terminal failures;
- last connected and last media timestamps;
- last redacted actionable failure category;
- recorder state and completed/failed counts;
- recorder/live queue capacity, depth, rejection, and drop counts.

Use validated `camera_id` as the only camera metric label. Never label by URL,
path, recording ID, or error text.

## Work packages

### P3.1 — Record the supervision decision

- Create an ADR covering ownership, command semantics, child-exit handling,
  recorder-failure policy, API isolation, and reversal criteria.
- Capture known facts, assumptions, and unresolved decisions.
- Write acceptance tests as names before implementation.

Exit: the intended owner and every child lifetime can be traced on paper.

### P3.2 — Extract an owned `CameraPipeline`

- Move transactional construction out of `main` behind a concrete factory.
- Own all data required for a `'static` structured task without leaking or
  cloning singular resources.
- Expose explicit shutdown and join behavior.
- Return typed startup and terminal errors with redacted context.
- Preserve all existing Phase 2 behavior and fixture results.

Tests: startup rollback, requested stop, camera-first exit, recorder-first exit,
and repeated stop behavior.

Exit: one pipeline can be stored, stopped, and joined by another owner.

### P3.3 — Implement `CameraSupervisor`

- Add a bounded supervisor command channel.
- Own a map keyed by validated `CameraId`.
- Implement add, start, stop, remove, and shutdown.
- Reject duplicate identity and unknown-camera operations explicitly.
- Observe unexpected child completion without blocking other commands.
- Cancel and join every child during process shutdown.
- Retain enough terminal outcome for diagnosis without retaining media.

Tests: independent two-entry lifecycle, duplicate add, unknown operations,
concurrent stop/remove, shutdown during backoff, and no detached tasks.

Exit: two in-memory pipeline owners can be controlled independently.

### P3.4 — Generalize health and metrics

- Introduce a bounded per-camera health registry.
- Add last-connected, last-media, and last-actionable-failure fields.
- Render metrics for all registry entries with bounded labels.
- Define stopped and removed camera visibility.
- Test concurrent reads and removals.

Exit: all configured cameras are independently diagnosable without media locks.

### P3.5 — Add the minimal camera API

Required loopback-only endpoints:

```text
GET    /api/cameras
POST   /api/cameras
GET    /api/cameras/{camera_id}
GET    /api/cameras/{camera_id}/health
POST   /api/cameras/{camera_id}/start
POST   /api/cameras/{camera_id}/stop
DELETE /api/cameras/{camera_id}
```

- Bound request size, request time, and concurrent work.
- Validate all external values before sending a supervisor command.
- Return stable external errors rather than raw internal errors.
- Keep credentials out of responses and tracing.
- Prove API listener failure does not terminate the supervisor.

Exit: API lifecycle commands are translations onto supervisor ownership.

### P3.6 — Prove two-camera isolation

Create a deterministic MediaMTX fixture with two paths and publishers:

1. Start both cameras and produce playable recordings.
2. Stop publisher A while B continues.
3. Observe A reconnecting and B completing segments.
4. Restart A and observe a fresh connection epoch.
5. Verify neither camera creates a duplicate upstream connection.
6. Shut down and join both pipelines.
7. Fully decode selected output from both camera roots.

Exit: failure of camera A has no recording or lifecycle effect on camera B.

### P3.7 — Failure and lifecycle qualification

Test removal during connecting, connected, and reconnecting; invalid credentials;
dimension mismatch; recorder saturation/storage failure; unexpected child exit;
API failure; repeated reconnect; health-client timeout; and process shutdown with
multiple active segments.

Track tasks and file descriptors across repeated reconnects. Do not infer absence
of leaks from Rust ownership alone.

Exit: failure behavior is reproducible and observable.

### P3.8 — Ten-camera qualification

- First run ten deterministic sources with recorded CPU, RAM, disk, network,
  queue, reconnect, gap, and decode evidence.
- Then run representative 4 MP/15 FPS/High Profile cameras for the agreed soak.
- Record exact hardware, storage, camera firmware, bitrate, and duration.

The synthetic run qualifies orchestration mechanics. Only the representative
run can support a ten-camera product claim.

## Required test matrix

| Scenario | Camera A | Camera B | Required result |
|---|---|---|---|
| Normal operation | recording | recording | both produce playable segments |
| A source offline | reconnecting | recording | B remains uninterrupted |
| A credentials invalid | terminal | recording | B remains uninterrupted |
| A recorder full | degraded/stopped | recording | failure contained and visible |
| API listener stops | recording | recording | both pipelines remain alive |
| Remove A | joined/removed | recording | no A task or connection remains |
| Process shutdown | finalizing | finalizing | all owners joined consistently |

## Validation commands

Run focused tests while implementing. At the Phase 3 gate run:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --workspace --release
git diff --check
```

Also run the existing Phase 2 qualification and the new two-camera fixture.
Report representative-camera and ten-camera hardware tests separately if the
external equipment is unavailable.

## Phase exit gate

Phase 3 is complete only when:

- multiple camera pipelines run concurrently;
- each running camera owns exactly one upstream connection;
- failure of camera N does not interrupt N-1 cameras;
- recorder and API failures are contained according to policy;
- all tasks have an owner, cancellation path, and join result;
- add/start/stop/remove behavior is tested across lifecycle states;
- state, queue pressure, and recovery are observable per camera;
- the two-camera isolation fixture passes reproducibly; and
- ten representative sources pass the agreed soak, or that external gate is
  explicitly reported as blocked rather than claimed complete.

## First executable action

Do not start with HTTP routes or a generic trait. Start by writing the Phase 3
supervision ADR and focused tests for an owned `CameraPipeline` that can be
started, stopped, and joined. Preserve the existing one-camera fixture as the
regression gate. Once singular pipeline ownership is proven, introduce the
bounded supervisor command loop and only then attach health and API adapters.
