# Architecture Context — 2026-08-10

## Product intent

Build a self-hosted security and video-analytics platform for homes and malls. Initial deployment is 10 cameras, target deployment is approximately 20, and the architecture should permit later evolution toward 100–200+ cameras without requiring an immediate distributed deployment.

Core operator questions are: what is happening now, which cameras are healthy, where people are concentrated, what happened at a given time, and what footage corresponds to an event/time range?

## Protected system shape

```text
RTSP Camera
    |
    v
Camera Ingestion (one owner, one upstream connection)
    |
    v
Bounded Media Distributor
    |--------------------|----------------------|
    v                    v                      v
Recorder             Detector              LiveKit Publisher
    |                    |                      |
    v                    v                      v
Segments             Domain Events          WebRTC viewers
    |                    |
    v                    v
Storage/index       Rules/API/Analytics
```

Configuration supplies validated policy across components but does not own their runtime state.

## Critical distinctions

### Encoded packet

Carries compressed camera media with codec, timestamps/timebase, keyframe information, camera/stream identity, and immutable payload. Recording should consume this representation to avoid unnecessary transcoding.

### Decoded frame

Carries pixel data required by detection or image analytics. Decoding is an explicit cost and should occur only for consumers that need frames. Detection samples frames rather than automatically processing the camera's full FPS.

### Domain event

A lightweight immutable fact such as `camera.offline`, `recording.completed`, `person.detected`, or `storage.critical`. Domain events must not contain video packets or frames. Commands and configuration updates are not events.

## Non-negotiable invariants

- Ingest each camera once; downstream consumers do not open their own camera sessions.
- Every queue is bounded and has an overload/drop/backpressure policy plus metrics.
- Recording, live publication, and detection fail independently.
- One camera's failure cannot stop other cameras.
- Encoded recording remains independent from decoded analytics.
- File metadata comes from actual media and lifecycle state, never assumed constants.
- Recording queries use indexed metadata rather than directory scans.
- Deployment-varying operational values are validated configuration, not core-code constants.
- Camera credentials never reach dashboard clients or logs.
- Scale claims require a documented workload and measurements.

## Ownership

| Concern | Owner |
|---|---|
| Camera connection, authentication, reconnect state | `camera` |
| Packet/frame representation and distribution | `media` |
| Segment creation and finalization | `recorder` |
| File placement, metadata, retrieval, retention | `storage` |
| Sampling and inference | `detector` |
| ROI, occupancy, heatmap derivation | `analytics` |
| Event contracts and pub/sub | `events` |
| Conditions and actions | `rules` |
| Real-time publication | `livekit` |
| Validated settings and persistence | `config` |
| HTTP translation/auth boundary | `api` |
| Persistence adapters and migrations | `database` |

## Failure priorities

Working priority, pending product confirmation:

1. Recording/data preservation.
2. Camera connectivity.
3. Live streaming.
4. Detection.
5. Derived analytics.

Database failure must not immediately stop recording. The implementation needs bounded metadata buffering/reconciliation. Disk failure cannot be hidden: monitor pressure, clean according to retention, protect recent footage according to confirmed policy, and report unrecoverable writes.

## Deployment evolution

- 10 cameras: one modular Rust deployment, local storage, PostgreSQL metadata, bounded in-process event bus, one LiveKit deployment.
- 20 cameras: profile and optimize decoding, GPU sampling, disk, network, and buffers.
- 50–100 cameras: extract only measured heavy workloads.
- 100–200+ cameras: consider distributed scheduling, storage placement, and external messaging with explicit semantics.

Kubernetes, Kafka/NATS, distributed databases, service meshes, and ubiquitous microservices are intentionally deferred.

## Local development source

Use the Mac webcam through:

```text
AVFoundation -> FFmpeg -> local MediaMTX -> RTSP ingestion
```

This exercises the same RTSP boundary as production cameras. Physical webcam testing remains manual; automated tests use deterministic file-backed RTSP fixtures. Device indices and permissions belong in local configuration/documentation, never core defaults.

