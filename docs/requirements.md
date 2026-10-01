# Product and System Requirements

Status: Phase 0 draft with accepted 2026 MVP qualification baseline

Last updated: 2026-09-01

This document separates confirmed direction from reversible defaults and unanswered product decisions. A working assumption is not an accepted requirement and must not be used to claim capacity or production readiness.

## Problem and stakeholders

The system provides reliable real-time and historical visibility into homes and malls through self-hosted camera ingestion, recording, live viewing, and people analytics.

- Product owner/requirement approver: Erik.
- Operators: security or property staff using live view, health, playback, and analytics.
- Administrators: people configuring cameras, retention, regions, rules, users, and infrastructure.
- Maintainers/on-call: engineers responsible for recovery, upgrades, storage, and observability.

## Confirmed architectural direction

- Initial target: 10 cameras on one server.
- Target deployment: approximately 20 cameras.
- Future direction: 100–200+ cameras without constraining later workload extraction.
- Input protocol: authenticated IP cameras connect directly to the server over
  RTSP/TCP. No RTSP proxy is part of the current codebase or production
  topology. ONVIF remains optional/unconfirmed.
- Recording: continuous, segmented, local, camera/time indexed, with configurable retention defaulting provisionally to 30 days.
- Live view: publish through LiveKit/WebRTC independently of recording.
- Detection: detect people from sampled decoded frames independently of recording.
- Analytics: support configurable ROI and a heatmap/occupancy capability once its semantics are confirmed.
- API: expose logical camera, health, recording/playback, event, analytics, ROI, and settings resources.
- Architecture: one owned upstream connection per camera; bounded media distribution to recorder, detector, and LiveKit consumers.
- Persistence: video files remain outside PostgreSQL; PostgreSQL stores metadata and indexes.
- Deployment: a modular monolith is the default for 10–20 cameras.

## Accepted 2026 MVP qualification profile

Erik accepted the initial profile in `docs/phase-0/2026-mvp-baseline.md` on 2026-08-10:

- 10 continuously recorded cameras initially, followed by measured 20-camera qualification.
- 4 MP at 15 FPS.
- H.264 High Profile required; H.265 remains an experimental recording/compatibility path.
- 4 Mbps average planning bitrate with 3–5 Mbps expected range and measured VBR peaks.
- Two-second GOP/keyframe interval.
- RTSP interleaved over TCP.
- ONVIF Profile T, digest/TLS-capable authentication, wired PoE, and NTP.
- Audio and PTZ excluded from the MVP baseline unless explicitly reintroduced.
- Sixty-second recording segments, 30-day configurable retention, and 5 FPS detection sampling.
- H.264 LiveKit publication for browser compatibility.

These values define the first qualification workload. They do not prove capacity and may be revised through evidence with a recorded decision.

Direct-camera qualification tooling is implemented under
`docs/direct-camera-rtsp/`, but no representative physical-camera evidence has
yet been collected. Camera-model interoperability, recovery under authorized
network/camera disruption, sustained soak, and 10/20-camera target-hardware
capacity remain open acceptance gates.

## Reliability requirements

- A camera failure affects only that camera's pipeline.
- Detector failure must not stop recording, live viewing, playback, or API operation.
- LiveKit failure must not stop recording, detection, or playback.
- API failure must not terminate media pipelines.
- Temporary database failure must not immediately stop recording; bounded metadata reconciliation is required.
- Every long-running task has an owner, cancellation path, and terminal-error policy.
- Every queue is bounded, observable, and has an explicit overload policy.
- The system reconnects transiently disconnected cameras automatically with observable state transitions.
- Disk pressure is detected before exhaustion; unrecoverable recording failure is surfaced rather than hidden.

Exact recovery-time, availability, and data-loss objectives require product approval.

## Security requirements

- Camera and LiveKit credentials never reach dashboard clients or logs.
- APIs exposed beyond loopback require authentication, authorization, and an explicit TLS boundary.
- Users may access only authorized cameras and recordings.
- Dynamic configuration is validated and auditable; secret values are not returned by ordinary settings APIs.
- Filesystem paths are internal implementation details and are not exposed as API resources.

Identity provider, roles, certificate termination, secret storage, credential rotation, and audit retention remain open.

## Performance and capacity requirements

Capacity claims must name camera count, resolution, FPS, codec/profile, bitrate, audio, detection FPS/model/hardware, viewer workload, storage hardware, test duration, and software versions.

Measure at least:

- CPU, GPU, RAM, disk throughput/latency/capacity, and network throughput/loss.
- Camera input FPS/bitrate, reconnects, recording gaps, and queue depth/drops.
- Detection throughput, consumer lag, and p50/p90/p99 latency.
- Live latency and API/playback-start p50/p90/p99.

No exact percentile objective is confirmed. See `docs/phase-0/resource-sizing.md` for planning calculations, which are estimates rather than benchmarks.

## Remaining reversible defaults for engineering spikes

These defaults permit experiments but are not frozen product requirements:

| Setting | Spike default | Application boundary |
|---|---:|---|
| Camera reconnect base delay | 1 second | Next retry |
| Camera reconnect maximum delay | 30 seconds | Next retry |
| RTSP transport | TCP | Next connection |
| Time representation | UTC internally | Always |
| Event bus | Bounded in-process | Process startup |

Queue capacities, watermarks, and retry-attempt behavior must be derived from latency/resource budgets rather than copied from this table.

## Product decisions required from Erik

### Camera and media

- Representative camera models and count?
- Which actual camera models satisfy the accepted qualification profile?
- What measured average/peak bitrate and timestamp behavior do representative cameras produce?

### Detection and analytics

- Detection model/accuracy objective and available production GPU?
- Required detection FPS and acceptable p90 freshness?
- Is tracking required, and for which business outcome?
- Does heatmap mean density, occupancy, dwell, or movement frequency?
- Who configures ROI polygons and aggregation windows?

### Playback and live view

- Stream, download, or both?
- Exact boundaries requiring transcode, or keyframe-aligned playback?
- Playback-start p90 and pre/post-event buffering?
- Simultaneous viewers and streams per viewer?
- Live latency target and LiveKit deployment ownership?

### Deployment and failure policy

- Production Linux distribution, CPU/GPU/RAM, disks/filesystem/redundancy, and network?
- Required availability, recovery time, and acceptable recording gap/data loss?
- Behavior after power loss and during database/disk failure?
- Confirmed priority among recording, connectivity, live view, detection, and analytics?

## Phase 0 acceptance

Phase 0 is complete only when topology-changing questions have accepted answers or reversible defaults, representative H.264 and H.265 RTSP paths are validated, segmentation/timestamp behavior is understood, and Erik accepts reliability/performance objectives.
