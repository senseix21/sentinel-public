# Decisions and Open Questions — 2026-08-10

## Accepted architectural decisions

- Rust is the implementation language for the media/control plane.
- Begin as a modular monolith for 10–20 cameras.
- One ingestion owner maintains one connection per camera.
- A bounded media-distribution layer fans out to recorder, detection, and LiveKit consumers.
- Encoded packets and decoded frames are different domain representations.
- Record encoded media without decode/re-encode whenever compatibility permits.
- Recording is critical and independent from detection and LiveKit.
- Record short configurable segments, initially expected to be 30–60 seconds.
- Store video on filesystem/object-style storage and metadata in PostgreSQL; do not store video blobs in PostgreSQL.
- Index recordings by camera and time; do not scan directories for normal API queries.
- Start with a bounded in-process domain event bus.
- Keep domain events separate from media data.
- Treat configuration and rules as first-class, persisted, validated behavior.
- Use the Mac webcam through a local RTSP bridge for manual development testing.
- Do not introduce distributed infrastructure until measurements justify extraction.

## Working assumptions, not confirmed requirements

- Recording is the highest business priority.
- Thirty days is the default retention target, but must remain configurable.
- Detection begins with people detection at a sampled rate near 5 FPS.
- A single server and single LiveKit deployment are adequate for the first 10 cameras.
- PostgreSQL is reachable most of the time, with bounded outage reconciliation.
- Local storage is sufficient for the initial deployment after sizing.

Code and documentation must label these as assumptions until accepted.

## Product questions requiring Erik's decision

### Cameras

- Exact models and count for initial qualification?
- Resolution, FPS, codec/profile, average/peak bitrate?
- RTSP and preferred transport confirmed?
- Is audio recorded/published?
- Is ONVIF discovery/control needed?
- Is PTZ required?
- Wired versus Wi-Fi distribution?

### Detection

- Which model and acceptable accuracy/false-positive profile?
- What CPU/GPU hardware is available?
- Required detection FPS and p90 freshness/latency?
- Is tracking required; if so, for what business question?

### ROI and heatmap

- Does heatmap mean density, occupancy, dwell, or movement frequency?
- Are ROIs polygons configured by users?
- Are multiple/versioned ROIs needed per camera?
- What time aggregation windows are required?

### Playback

- Streaming, download, or both?
- Exact versus keyframe-aligned boundaries?
- Playback-start p90 objective?
- Pre/post-event buffering requirements?
- Browser/container/codec compatibility requirements?

### Live viewing

- Expected simultaneous viewers and viewed streams per user?
- Target live p90 latency?
- Who runs and secures LiveKit?
- Which camera codecs can pass through versus transcode?

### Deployment and reliability

- Production Linux distribution and server hardware?
- One physical server or multiple?
- Storage devices, redundancy, filesystem, and available capacity?
- Appliance-style/offline operation?
- Required uptime, recovery time, and acceptable recording gaps?
- Behavior under disk failure and power loss?
- Confirmed subsystem priority?

### Security

- User identity provider and roles?
- Camera-level authorization requirements?
- TLS termination location?
- Credential storage/rotation mechanism?
- Audit retention requirements?

## Engineering questions to resolve through spikes/ADRs

- GStreamer versus another RTSP/media ingestion implementation for the production boundary.
- Container choice by codec and playback compatibility.
- Timestamp/timebase normalization and discontinuity policy.
- Segment finalization ordering and crash reconciliation.
- Media subscriber API, buffer ownership, and overload semantics.
- Event ordering/delivery/overflow semantics.
- LiveKit publication adapter and codec conversion requirements.
- Bounded behavior during PostgreSQL outage.
- Dynamic configuration application boundaries.

## Decision record rule

Material, hard-to-reverse decisions require an ADR containing context, options, trade-offs, decision, consequences, and reversal criteria. Do not turn an unanswered requirement into an architectural fact merely to keep coding.

