# SENTINEL: Client System Guide

Document purpose: explain what SENTINEL is, how it works, what each component
does, what is working today, and what remains before production deployment.

Status date: 2026-09-01

## Executive summary

SENTINEL is a self-hosted, multi-camera surveillance platform written in Rust.
It connects to IP cameras, continuously records their video, provides live
browser viewing, indexes recordings for later search and playback, and provides
the foundation for person detection and future analytics.

The system is designed so that each camera is contacted only once. The incoming
video is then distributed inside SENTINEL to recording, live viewing, and
detection. This is important because it reduces load on the cameras and keeps
the features independent: for example, a failure in live viewing should not
stop recording.

The current product is best described as an **implemented and locally qualified
H.264 MVP**, not yet as a production-certified 10- or 20-camera deployment. The
core ingest, recording, playback, live-view, security, configuration, and
observability paths are working. Production person detection, ROI, rules,
analytics, deployment packaging, and target-hardware capacity qualification are
not complete.

## What problem the system solves

SENTINEL brings the main surveillance workloads under one self-hosted system:

- Connect to multiple RTSP cameras and monitor their state.
- Preserve original camera video in short, recoverable recording segments.
- Search recordings by camera and time instead of scanning directories.
- Watch live video from a browser through WebRTC.
- Prepare sampled video frames for person detection without disturbing
  recording.
- Control access by user role and camera scope.
- Expose health, metrics, logs, and audit history for operators.
- Keep camera video and operational data on infrastructure controlled by the
  client.

The initial business target is 10 cameras on one machine, followed by a measured
20-camera qualification. A path to larger installations is preserved, but
support for 100–200 cameras is a future architecture direction, not a current
capacity claim.

## System at a glance

```text
IP camera
   |
   | RTSP/TCP (one authenticated connection per camera)
   v
Camera ingestion owner
   |
   v
Bounded media distributor
   |------------------> Recorder --> fragmented MP4 files
   |                                      |
   |                                      `--> PostgreSQL recording index
   |
   |------------------> Live publisher --> LiveKit --> browser/WebRTC
   |
   `------------------> Frame sampler --> detector --> domain event bus
                                                        |
                                                        `--> future rules and analytics

Browser --> trusted identity proxy --> SENTINEL HTTP API and dashboard
                                      |--> camera control and health
                                      |--> recording search/playback
                                      |--> settings and identities
                                      `--> system status
```

There are three deliberately separate paths:

1. **Media plane:** encoded video packets and decoded frames.
2. **Event plane:** small facts such as camera state changes or detections.
3. **Control plane:** authenticated HTTP requests, configuration, and camera
   lifecycle commands.

Video is never placed on the event bus. This keeps memory use and failure
behavior predictable.

## How a camera moves through the system

### 1. Camera connection

For each configured camera, SENTINEL creates one ingestion owner. That owner
holds the RTSP connection, camera credentials, connection state, timestamps,
and reconnect policy. It moves through explicit states such as disconnected,
connecting, connected, and reconnecting.

If a connection is temporarily lost, that camera retries with bounded
exponential backoff. Other camera pipelines continue independently.

### 2. Internal distribution

The ingestion owner sends the received encoded video into a bounded media
distributor. Recording, live viewing, and detection subscribe to this internal
source. Adding one of those consumers does not create another camera connection.

Every queue has a capacity and an overload rule:

- Recording prioritizes preservation and reports when it cannot keep up.
- Live viewing may discard stale video to protect latency.
- Detection prefers recent frames and drops or skips stale work.
- Domain events use an explicit policy based on event importance.

This prevents a slow consumer from creating unlimited memory growth.

### 3. Recording

Compatible H.264 video is recorded without decoding and re-encoding. SENTINEL
writes fragmented MP4 segments under deterministic camera and UTC-based paths.
The intended qualification profile uses 60-second segments, although the value
is configurable.

A segment is first written as a partial file and then finalized. Its metadata is
obtained from the real output file rather than from assumed duration or format
values. PostgreSQL stores the camera, recording ID, exact time range, codec,
resolution, size, path, and lifecycle state.

If the database is temporarily unavailable, media preservation does not stop
immediately. Durable files remain on disk and reconciliation can index them when
the database recovers. Partial files are retained as evidence for recovery
instead of being silently discarded.

### 4. Search and playback

The browser or API requests a camera and UTC time range. SENTINEL queries the
PostgreSQL index for all finalized segments that overlap that range and streams
the assembled MP4 content over HTTP.

The current behavior is segment-aligned and stream-copy oriented. It is suitable
for retrieving recorded intervals, but it does not promise frame-exact start and
end boundaries or a fully seekable newly synthesized file.

### 5. Live viewing

When a remote viewer is present, SENTINEL starts the live publication path for
that camera. The current reliable browser path decodes the camera stream and
feeds frames to LiveKit/WebRTC using the WebRTC encoder. When no viewer remains,
the decoder and publication are stopped after a configurable short grace period.
This avoids paying the decode cost for unwatched cameras.

Live viewing remains independent of recording. A LiveKit or live-transcoder
failure must not terminate the recorder or camera connection.

Direct encoded H.264 pass-through was investigated and implemented, but is not
usable on the current Apple development target. The viewer-gated transcode path
is therefore the active design.

### 6. Detection and events

Detection receives sampled frames through its own bounded, freshness-first
queue. It decodes only for the detection consumer and does not change the
original encoded recording path. Detection results are camera-scoped,
timestamped facts with normalized bounding boxes, confidence, and model version.

Those facts can be published to the bounded domain event bus alongside camera,
recording, and storage events. The event contracts and overload behavior are
implemented. However, the only current inference engine is deterministic fake
inference for systems testing. SENTINEL does **not** yet provide production human
detection or an accuracy claim.

## Component guide

| Component | Responsibility | Current state |
|---|---|---|
| SENTINEL server | Owns the running modular monolith, starts components, and serves the current HTTP/dashboard surface | Working |
| Camera | Owns one RTSP session per camera, lifecycle, timestamps, reconnect, and credentials | Working for the qualified H.264/TCP path |
| Media | Defines encoded packets/frames and bounded distribution to consumers | Working |
| Recorder | Creates and finalizes fragmented MP4 segments from encoded H.264 | Working |
| Storage | Owns paths, recording metadata, indexed retrieval, reconciliation, and retention coordination | Working, with deployment storage still unqualified |
| Database | Implements PostgreSQL repositories and migrations for cameras, recordings, settings, identities, and audit | Working |
| LiveKit publisher | Publishes live video independently of recording and stops expensive work when unwatched | Working with documented latency and compatibility limits |
| Detector | Samples and decodes frames, runs an inference adapter, and produces structured detections | Pipeline working; production model absent |
| Events | Carries bounded, lightweight domain facts with explicit delivery and overflow behavior | Working |
| Configuration | Resolves defaults, files, persisted settings, and environment overrides; validates changes transactionally | Working |
| Authentication | Owns principals, roles, camera scope, and Argon2id-hashed bearer tokens | Working at the server boundary |
| HTTP API/dashboard | Presents camera, playback, live, system, settings, and identity operations | Working in `apps/server`; dedicated `crates/api` remains a stub |
| Rules | Would convert events and configured conditions into actions | Not implemented; product conditions are undefined |
| Analytics | Would own ROI-derived occupancy, paths, or heatmaps | Not implemented; semantics are undefined |
| Worker application | Reserved for a future separately deployed workload if measurements require it | Empty stub; not deployed |
| Development proxy | Provides a loopback-only browser login and trusted identity assertion for local use | Working development fixture; not a production identity provider |
| External RTSP fixture | Previously provided deterministic RTSP and webcam-backed sources | Removed by ADR 013; replacement not implemented |
| LiveKit Server | Delivers WebRTC media to browser viewers | External runtime dependency |
| PostgreSQL | Stores operational state and indexes, not video payloads | External runtime dependency |
| FFmpeg/FFprobe | Supports local camera bridging, decoding/transcoding, and media inspection | External runtime dependency |

The original `cam_rtsp` prototype under the repository root is reference
material only. It is outside the Rust workspace, is not compiled, and is not
part of the shipped system.

### Production and development media paths

The intended physical-camera production path is direct:

```text
IP camera -> one owned Retina RTSP/TCP session -> bounded distributor
                                              |-> recorder
                                              |-> detector
                                              `-> LiveKit publisher -> browser
```

LiveKit is a downstream WebRTC delivery system; it does not ingest RTSP from the
camera or replace the camera ingestion owner. The former proxy-backed automated
RTSP tests and macOS webcam bridge were removed by ADR 013. The executable
physical-camera qualification contract is
in `docs/direct-camera-rtsp/SPEC.md` and `docs/direct-camera-rtsp/PLAN.md`.

## Working crates and important files

A Rust **crate** is a separately compiled module with a defined responsibility.
The workspace combines the crates below into the `surveillance-server`
application. The file list highlights the implementation entry points; test
modules and supporting types exist in addition to the files named here.

### Server application: `apps/server`

This is the executable composition layer. It connects the domain crates,
defines application ownership and shutdown behavior, and currently contains the
HTTP adapter.

| File or directory | What it does |
|---|---|
| `src/main.rs` | Process entry point: loads configuration and starts the server runtime. |
| `src/pipeline.rs` | Assembles the camera, recorder, detector, LiveKit, event, and metadata components into an owned pipeline. |
| `src/one_camera.rs` | Builds and runs the end-to-end pipeline for one camera. |
| `src/camera_supervisor.rs` | Owns the multi-camera pipeline map and typed add/start/stop/remove operations. |
| `src/health_http.rs` | Implements the dashboard, health endpoints, metrics endpoint, and current protected HTTP API. This is the active API implementation. |
| `src/authenticate.rs` | Extracts and verifies bearer or trusted-proxy authentication. |
| `src/authorize.rs` | Central role, action, and camera-scope authorization chokepoint. |
| `src/trust_boundary.rs` | Verifies reverse-proxy peer and shared-secret proof before accepting asserted identity or forwarded information. |
| `src/security_config.rs` | Resolves and validates listener and security-boundary configuration. |
| `src/runtime_config.rs` | Translates resolved settings into component runtime configuration. |
| `src/settings_service.rs` | Implements settings reads and transactional update use cases. |
| `src/metadata_writer.rs` | Persists completed recording metadata without making recording depend directly on database availability. |
| `src/reconciliation_owner.rs` | Owns startup and continuous repair of durable recording files versus the database index. |
| `src/retention_owner.rs` | Owns periodic retention and storage-pressure coordination. |
| `src/playback.rs` | Plans and assembles segment-aligned recording playback. |
| `src/phase4_metrics.rs` | Bridges database/storage state into bounded Prometheus metrics. |
| `ui/` | Framework-free browser dashboard, CSS, JavaScript modules, LiveKit client asset, and UI tests. |

`health_http.rs` is large because the planned extraction into `crates/api` has
not happened. This is a known code-organization limitation, not a missing API
capability.

### `crates/common`

Contains only stable types genuinely shared across domains.

| File | What it does |
|---|---|
| `src/types.rs` | Validated shared identities such as camera and recording IDs. |
| `src/secret.rs` | Secret wrappers, redacted formatting/serialization, and safe fingerprints. |
| `src/lib.rs` | Exposes the cross-domain contracts. |

`error.rs` and `time.rs` are currently placeholders; error and time policy stays
with the domain that owns it until a stable shared contract exists.

### `crates/camera`

Owns camera connectivity and the singular upstream RTSP session.

| File | What it does |
|---|---|
| `src/connection.rs` | Explicit connection lifecycle, state transitions, epochs, and failure classification. |
| `src/rtsp.rs` | Authenticated Retina RTSP/TCP session adapter and packet ingestion. |
| `src/h264_depay.rs` | Bounded RFC 6184 H.264 RTP depayloading and access-unit assembly. |
| `src/supervisor.rs` | Owns camera task lifetime, cancellation, retry, and child-task outcomes. |
| `src/health.rs` | Credential-free camera health snapshots and state reporting. |

`reconnect.rs` is presently a placeholder; the active reconnect mechanics are
implemented with the connection/supervisor ownership rather than this file.

### `crates/media`

Owns the in-process media contracts. It does not own camera connectivity or
business events.

| File | What it does |
|---|---|
| `src/packet.rs` | Encoded packet and H.264 access-unit representations, timestamps, and bounds. |
| `src/frame.rs` | Decoded frame representations used by consumers that require pixels. |
| `src/distribution.rs` | Independent bounded subscriptions, queue counters, and overload policies for recorder/live/detection consumers. |
| `src/sps.rs` | Parses and validates H.264 sequence parameter information needed by downstream media paths. |

### `crates/recorder`

Owns conversion of encoded H.264 access units into recoverable recording
segments.

| File | What it does |
|---|---|
| `src/fmp4.rs` | Fragmented MP4 muxing and codec/container mechanics. |
| `src/segment.rs` | Type-safe partial, finalizing, completed, and failed segment lifecycle. |
| `src/recorder.rs` | Recorder state and access-unit consumption behavior. |
| `src/runtime.rs` | Runtime segment rotation and muxer integration. |
| `src/task.rs` | Owned async recorder task, channel drain, shutdown, and failure reporting. |
| `src/clock.rs` | Recording time source and deterministic test clock support. |
| `tests/retina_fmp4.rs` | External RTSP-to-fragmented-MP4 integration coverage. |
| `tests/rtp_to_recorder.rs` | RTP depayloading through recorder integration coverage. |

Retention policy is owned by `crates/storage`; `recorder/src/retention.rs` is a
placeholder and is not an alternate implementation.

### `crates/storage`

Owns recording placement, metadata, retrieval, reconciliation, and retention
coordination.

| File | What it does |
|---|---|
| `src/filesystem.rs` | Filesystem abstraction, durable-file operations, and storage observations. |
| `src/segment_store.rs` | Deterministic camera/UTC paths and exclusive partial/final segment creation. |
| `src/metadata.rs` | Truthful recording metadata and probing contracts. |
| `src/recordings.rs` | Recording repository contracts and overlapping time-range queries. |
| `src/reconciliation.rs` | Compares files with indexed state and plans safe import/repair work. |
| `src/retention.rs` | Retention candidates, watermarks, hysteresis, and deletion coordination. |

### `crates/database`

Implements PostgreSQL adapters. It persists state but does not own media or
business policy.

| File | What it does |
|---|---|
| `src/recordings.rs` | PostgreSQL recording index, overlap queries, reconciliation evidence, and retention candidates. |
| `src/settings.rs` | Persisted settings and same-transaction configuration audit trail. |
| `src/auth.rs` | Principal and bearer-token persistence for the authentication contracts. |
| `src/access_audit.rs` | Validated access, authorization, camera, export, and identity audit records. |
| `src/config.rs` | Validated database connection configuration. |

`cameras.rs`, `events.rs`, and `rois.rs` are placeholders. Camera persistence
needed by the current application is handled through the implemented repository
paths; durable event history and ROI persistence are not implemented.

### `crates/events`

Owns lightweight domain facts and bounded in-process publication.

| File | What it does |
|---|---|
| `src/event.rs` | Versioned camera, recording, storage, and detection event contracts. |
| `src/bus.rs` | Bounded subscriptions, ordering scope, overflow policy, counters, and shutdown. |
| `src/publisher.rs` / `src/subscriber.rs` | Narrow publisher/subscriber handles over the owned bus. |

This crate deliberately cannot carry encoded packets or decoded frames.

### `crates/detector`

Owns the sampled detection systems pipeline, but not a production model.

| File | What it does |
|---|---|
| `src/decoder.rs` | Owns the managed FFmpeg decode process and decoded-frame output. |
| `src/sampler.rs` | Media-timestamp sampling, freshness preference, and discontinuity handling. |
| `src/detector.rs` | Bounded detector task, queue behavior, lifecycle, metrics, and event publication. |
| `src/inference.rs` | `PersonInference` boundary and `DeterministicFakeInference` test implementation. |
| `examples/fake_workload.rs` | Deterministic systems-throughput fixture; not an accuracy demonstration. |

### `crates/livekit`

Owns live publication and its failure boundary.

| File | What it does |
|---|---|
| `src/owner.rs` | Owns room connection, viewer gating, publication lifecycle, retry, cancellation, and metrics. |
| `src/transcode.rs` | Active FFmpeg-decode-to-WebRTC-frame publication adapter. |
| `src/decode.rs` | Managed decoding process and frame conversion mechanics. |
| `src/native.rs` | Native LiveKit source integration, including the investigated encoded path. |
| `src/adapter.rs` | Publication adapter contract shared by encoded and transcode approaches. |
| `src/rtp.rs` | RTP/access-unit transformation and pacing support. |
| `src/token.rs` | Short-lived, camera-scoped viewer token creation. |
| `src/config.rs` | Validated LiveKit connection and publication settings. |
| `src/domain.rs` | Publication state, timestamp authority, and typed outcomes. |

`publisher.rs` is a placeholder; `owner.rs` contains the active publication
owner.

### `crates/config`

Owns validated operational settings and secret loading.

| File | What it does |
|---|---|
| `src/defaults.rs` | Versioned compiled defaults. |
| `src/file.rs` | TOML configuration-file loading and validation. |
| `src/environment.rs` | Environment override parsing without exposing secrets. |
| `src/resolution.rs` | Resolves `defaults < file < persisted < environment` and records each value's origin. |
| `src/classification.rs` | Defines every setting's type, secrecy, scope, validation group, and application boundary. |
| `src/change.rs` | Whole-batch transactional validation and change outcomes. |
| `src/assemble.rs` | Builds domain runtime configuration from resolved settings. |
| `src/cache.rs` | Holds validated resolved configuration generations. |
| `src/repository.rs` | Persistence contract used by the PostgreSQL adapter. |
| `src/secret_store.rs` | Loads `0600` secret files and per-key environment overrides. |
| `src/settings.rs` / `src/value.rs` | Setting identities and typed values safe for API/audit use. |

### `crates/auth`

Owns identity and credential verification. Authorization policy remains beside
the server actions it protects.

| File | What it does |
|---|---|
| `src/principal.rs` | Principal IDs, human/service kinds, roles, and camera scope. |
| `src/token.rs` | Expiring bearer-token format, IDs, secrets, and issuance. |
| `src/verify.rs` | Bounded Argon2id verification and typed authentication failures. |
| `src/store.rs` | Token-store contract and in-memory test implementation. |
| `src/clock.rs` | System and deterministic clocks for expiry behavior. |
| `examples/mint-token.rs` | Administrative token-generation example. |

### Present but not working product crates

These directories compile as workspace members, but their domain files contain
only module declarations or placeholder comments. Their presence must not be
interpreted as a delivered feature.

| Path | Intended responsibility | Actual state |
|---|---|---|
| `crates/api` | Move HTTP translation out of `apps/server` | Stub; current HTTP implementation is `apps/server/src/health_http.rs` |
| `crates/analytics` | ROI, occupancy, and heatmap derivation | Stub; product semantics are unconfirmed |
| `crates/rules` | Evaluate event conditions and execute actions | Stub; conditions and actions are unconfirmed |
| `apps/worker` | Separately deployed heavy workload | Reserved executable that only prints a message; extraction requires Phase 9 evidence |

### Database migrations and operational files

| Path | What it does |
|---|---|
| `migrations/0001_recordings.sql` | Recording metadata schema and indexes. |
| `migrations/0002_settings.sql` | Persistent settings and configuration audit schema. |
| `migrations/0003_auth.sql` | Principals and Argon2id bearer-token schema constraints. |
| `migrations/0004_access_audit.sql` | Security/access audit schema and category constraints. |
| `migrations/0005_retire_livekit_cli_settings.sql` | Removes settings belonging to the retired CLI publication path. |
| `config/default.toml` | Versioned default operational values. |
| `config/example.toml` | Deployment-oriented configuration example. |
| `scripts/dev/local-stack.sh` | Starts and owns the complete local development stack. |
| `scripts/dev/trust-boundary-proxy.py` | Loopback-only development login and trusted-principal proxy. |
| `scripts/phase0` through `scripts/phase9` | External integration, failure, browser, security, and load qualification fixtures. |
| `deploy/systemd/README.md` / `deploy/docker/README.md` | Records deployment prerequisites and blockers; these are not completed packages. |
| `docs/adr/` | Accepted architectural decisions and reversal criteria. |
| `docs/phase-*/` | Contracts, runbooks, and measured evidence for each delivery phase. |

## What the operator can do today

After authentication, the dashboard and API currently support:

- View the cameras visible to the signed-in principal.
- Add, inspect, start, stop, and remove camera pipelines.
- See per-camera connection and subsystem health.
- Open a live browser view through LiveKit.
- Search recordings by camera and time range.
- View recording metadata and stream selected recorded video.
- Inspect system capability, dependency, storage, and readiness state.
- Read effective settings and see where their values came from.
- Update allowed settings transactionally when the mutating surface is enabled.
- Administer identities and issue or revoke service bearer tokens, subject to
  role permissions.
- Observe public liveness, readiness, and Prometheus metrics endpoints.

The local development stack also supplies a login page and session through a
loopback-only proxy. That convenience login is not intended for production.

## Security model

Protected API requests reach one authorization chokepoint. The server checks the
authenticated principal, role, requested action, and camera scope. A user who is
not allowed to see a camera receives `404`, avoiding disclosure that the camera
exists.

Two identity paths are supported by the server:

- Service identities use expiring bearer tokens stored only as Argon2id hashes.
- Human identities are expected to be authenticated by an identity-aware reverse
  proxy. SENTINEL trusts an asserted principal only after verifying both the
  proxy network peer and a constant-time shared-secret proof.

Secrets are excluded from ordinary settings responses and logs. The secret file
must have `0600` permissions, and sensitive audit detail is stored as a
fingerprint rather than a raw value. Access, authorization, camera lifecycle,
recording export, configuration, and identity actions are audited.

The public `/live`, `/ready`, and `/metrics` routes intentionally do not require
an API identity. A server bind beyond loopback is refused unless the configured
trust boundary and an enabled principal make that exposure explicit.

Production TLS termination, identity-provider selection, MFA, session policy,
and secret ownership/rotation procedures still belong to the deployment design.

## Reliability and failure behavior

SENTINEL uses explicit ownership and bounded queues so one failure does not
automatically cascade through the platform.

| Failure | Intended and implemented behavior | Remaining qualification |
|---|---|---|
| One camera disconnects | Only that camera reconnects; other camera pipelines continue | Representative-camera flapping/reconnect tests |
| Recorder fails | Failure is visible and that consumer stops without silently losing ownership | Genuine disk-full behavior on target storage |
| PostgreSQL is temporarily unavailable | Recording files continue within the media/storage boundary; readiness degrades; reconciliation restores metadata | Backup/restore and target deployment recovery |
| LiveKit or publisher fails | Live view degrades while recording remains active | Deterministic production-like kill/restart recovery |
| Detector fails or falls behind | Detection degrades independently; stale frames are discarded; recording continues | Production model/runtime failure tests |
| API listener fails | It does not send a shutdown signal to the media pipelines | Deployment supervisor behavior |
| Process shuts down | Owned tasks receive cancellation and in-progress segments are finalized or left with explicit partial evidence | Combined active-load shutdown qualification |

The provisional priority is recording, camera connectivity, live viewing,
detection, then derived analytics. The client still needs to confirm this order
as business policy.

## Observability and audit

SENTINEL exposes:

- Structured logs for state transitions and actionable failures.
- `/live` for process liveness.
- `/ready` for dependency- and camera-aware readiness.
- Per-camera health through the protected API.
- Prometheus metrics for cameras, bounded queues, recordings, storage,
  reconciliation, detection, LiveKit, API behavior, authentication, and
  authorization.
- Database audit records for sensitive access and administrative actions.

Alert rules and Grafana dashboard definitions exist. Their metric names were
checked against a running local server, but the alert rules have not been run
through `promtool`, and the dashboards have not been rendered and reviewed in a
real Grafana instance. They must not yet be described as operationally proven.

## Current delivery status

### Working and supported by local evidence

- A real Rust workspace and modular-monolith server.
- One authenticated RTSP/TCP H.264 ingestion connection per camera.
- Explicit camera lifecycle, reconnect backoff, and multi-camera failure
  isolation.
- Bounded distribution to recorder, live publisher, and detector.
- Fragmented MP4 segmented recording with real-file metadata.
- PostgreSQL camera and recording indexes with overlapping-time queries.
- Startup and continuous recording reconciliation.
- Retention coordination and storage warning/critical watermarks.
- Camera APIs, recording search/detail/playback, settings, identities, health,
  readiness, and metrics.
- Browser live video through viewer-gated transcoding and LiveKit/WebRTC.
- Detection sampling, bounded overload behavior, and domain event contracts.
- Layered, validated, persisted, and audited configuration.
- Authentication, camera-scoped authorization, secret redaction, proxy-origin
  proof, and access audit.
- Unit and in-process integration coverage for the direct ingestion contracts.
- A guarded direct-camera qualification harness that runs the release server
  without an RTSP proxy, samples metrics/resources, proves the
  server-owned RTSP/TCP peer, and validates recorded segments. Its local report
  and regression gates pass; it has not yet been executed against a physical
  production camera.

### Implemented but only partly qualified

- Direct physical-camera RTSP is the intended production topology, but camera-
  model interoperability, disruption recovery, sustained soak, and 10/20-camera
  target-hardware capacity remain external qualification gates.
- Real browser live rendering has been demonstrated on controlled local H.264
  sources and Chromium, but not across the final camera/browser matrix.
- Controlled local frame accounting showed no publisher loss at 15 FPS for one
  camera with one, two, and four viewers. This is not a production concurrency
  claim.
- Warm reconnect render-start measurements exist for a local fixture. They are
  not camera-to-glass latency measurements, and cold start is not accepted.
- Database outage recovery, synthetic camera reconnect, abrupt recording
  termination, and several subsystem isolation paths have focused evidence, but
  not all have been repeated on target deployment hardware.
- Metrics, alert files, dashboard definitions, and partial runbooks exist, but
  the full operator validation gate remains open.

### Not implemented or not yet valid to claim

- Production person-detection model, measured accuracy, or production GPU
  runtime.
- ROI administration and coordinate semantics.
- Rules, alert actions, tracking, occupancy analytics, and heatmaps.
- H.265 production support and the full H.264 High Profile/multi-slice camera
  matrix.
- Audio, PTZ, and a product-qualified ONVIF workflow.
- Frame-exact arbitrary-boundary playback.
- Durable domain-event history or replay.
- A separately deployable worker or internal gRPC control plane.
- Confirmed production packaging, service manager configuration, TLS/identity
  integration, upgrade/rollback, and complete backup/restore procedures.
- Demonstrated 10-camera or 20-camera sustained capacity on named target
  hardware.
- Any demonstrated 100–200-camera capacity.

## External services and deployment boundary

A complete deployment needs SENTINEL plus several supporting services:

| Service | Why it is needed | Ownership decision required |
|---|---|---|
| PostgreSQL | Operational data, configuration, identity, audit, and recording index | Host, backup, restore, retention, and availability |
| LiveKit | WebRTC delivery for browser live view | Self-hosted topology, capacity, TLS, and upgrade owner |
| FFmpeg/FFprobe | Decode/transcode and media inspection | Package/version patch policy and hardware acceleration choice |
| Reverse proxy and identity provider | TLS, human login, and trusted principal assertion | Product/provider, MFA/session policy, certificates, and HA |
| Recording filesystem | Durable MP4 storage | Capacity, filesystem, RAID/redundancy, mount layout, and disk-failure policy |
| Monitoring stack | Scrape metrics, evaluate alerts, and display dashboards | Prometheus/Grafana placement, retention, and on-call ownership |

The development launcher starts local versions of the necessary services, but
those settings and credentials are deliberately not production configuration.

## Decisions required from the client

These decisions affect acceptance criteria or architecture and should not be
silently guessed.

### Cameras and media

- Exact camera makes/models and representative sample streams.
- Codec/profile, real average and peak bitrate, resolution, FPS, and GOP for
  every supported model.
- Whether audio, PTZ, or ONVIF administration is required.
- Expected behavior during extended camera outage or clock drift.

### Live view and playback

- Maximum concurrent users and live tiles per user.
- Accepted live first-picture and camera-to-glass latency objectives.
- Browser and device support matrix.
- Whether playback must support exact-frame boundaries, download, seeking, and
  event pre/post roll.

### Detection and analytics

- Production person-detection model, labeled evaluation set, confidence
  semantics, and accuracy objective.
- Available CPU/GPU hardware and required detection sampling rate.
- Whether tracking or unique-person counting is required.
- Exact meaning of ROI, occupancy, dwell, and heatmap outputs.
- Rules, notification channels, escalation behavior, and audit expectations.

### Deployment and operations

- Linux distribution and exact server CPU, GPU, RAM, network, and storage.
- Recording retention period, storage safety margin, and disk-full behavior.
- Availability, recovery-time, and acceptable data-loss objectives.
- TLS termination, identity provider, roles, MFA, and session policy.
- Backup/restore, upgrades, rollback, monitoring, and on-call ownership.
- Final business priority when resources cannot serve every subsystem.

## Recommended path to production

1. **Close product decisions.** Confirm camera models, workloads, user
   concurrency, detection/analytics semantics, hardware, and operational SLOs.
2. **Qualify representative cameras.** Test ingest, timestamps, segmentation,
   reconnect, live view, and playback with every required camera profile.
3. **Integrate production detection.** Select the model/runtime and validate
   accuracy separately from systems throughput.
4. **Complete product features.** Implement only the confirmed ROI, rule, and
   analytics semantics with observable acceptance criteria.
5. **Define and package deployment.** Select the target OS and storage, then
   finish TLS/identity integration, service packaging, backup, upgrade, rollback,
   and runbooks.
6. **Run production-like failure tests.** Include disk exhaustion, network
   partition, camera flapping, LiveKit restart, database recovery, and shutdown
   under active workloads.
7. **Qualify capacity.** Establish a baseline, then run sustained 10-camera and
   20-camera workloads on the named target hardware while measuring CPU, GPU,
   RAM, disk, network, drops, gaps, recovery, and percentile latency.
8. **Scale only from evidence.** Extract a worker or add distributed
   infrastructure only if measurements identify a real bottleneck.

## How success should be accepted

A production acceptance should name the exact workload rather than use a broad
statement such as “supports 20 cameras.” At minimum, the acceptance report
should record:

- Camera count and models.
- Resolution, FPS, codec/profile, GOP, and bitrate distribution.
- Segment duration and retention policy.
- Detection model, sample FPS, and compute hardware.
- Number of viewers and viewed-camera distribution.
- Test duration and injected failures.
- Server CPU/GPU/RAM, disk/filesystem, and network.
- CPU, GPU, memory, disk and network utilization.
- Queue depth/drops, recording gaps, reconnects, and subsystem failures.
- Live, detection, playback, and API p50/p90/p99 latency against agreed targets.

Until that evidence exists, the honest description is: **the core H.264
surveillance platform works end to end in the qualified local environment; its
production feature set, deployment, camera compatibility, and 10/20-camera
capacity still require client decisions and target-environment validation.**

## Supporting documents

- `README.md` — developer quick start and current repository capabilities.
- `docs/API.md` — complete current HTTP API reference.
- `docs/presentation/SENTINEL-system-overview.pptx` — stakeholder presentation.
- `ROADMAP.md` — authoritative item-by-item delivery status.
- `docs/architecture.md` — short technical architecture boundary.
- `docs/adr/` — accepted architecture decisions and reversal criteria.
- `docs/phase-8/threat-model.md` — security boundaries and accepted risks.
- `docs/phase-9/` — current live-view and capacity qualification evidence.
- `analysis.md` — target architecture proposal; it is not a statement of current
  implementation.
