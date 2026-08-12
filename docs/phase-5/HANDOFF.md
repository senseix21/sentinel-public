# Phase 5 Handoff — Independent LiveKit Publication

Status: ready to implement after the Phase 4 local gate

Baseline date: 2026-08-12

## Mission

Publish each supported camera's live H.264 stream through self-hosted LiveKit
without opening another camera connection and without coupling recording,
indexed playback, reconciliation, retention, or the API to LiveKit availability.

Phase 5 adds one latency-oriented consumer to the existing encoded media fan-out.
It must not move archival responsibilities into LiveKit, put media on the domain
event bus, expose camera or LiveKit secrets to clients, or introduce pixel
decode/re-encode unless an explicit compatibility decision and measured resource
budget require it.

## Read first

Read these sources in order before changing publication behavior:

1. `AGENTS.md`
2. `context/README.md` and its latest snapshot
3. `ROADMAP.md`, Phase 5
4. `docs/requirements.md`
5. `docs/adr/001-media-pipeline.md`
6. `docs/adr/003-configuration.md`
7. `docs/phase-0/livekit-publication.md`
8. `docs/phase-0/livekit-rust-api.md`
9. `docs/phase-0/rtp-h264-contract.md`
10. `docs/phase-0/timestamp-contract.md`
11. `docs/phase-0/media-distribution-contract.md`
12. `crates/media/src/packet.rs`
13. `crates/media/src/distribution.rs`
14. `crates/camera/src/supervisor.rs`
15. `apps/server/src/one_camera.rs`
16. `apps/server/src/pipeline.rs`
17. `apps/server/src/camera_supervisor.rs`
18. `apps/server/src/health_http.rs`
19. `crates/livekit/`

Read `analysis.md` only when resolving an architectural ambiguity. It is the
target proposal, not evidence of the current implementation.

## Verified starting point

The current workspace provides:

- one owned RTSP/TCP connection per running camera;
- complete, timestamped, bounded H.264 access units after RFC 6184 depayloading;
- independent recorder-preserving and live-freshness queues in
  `MediaDistributor`;
- explicit queue capacity, depth, accepted, rejected, and stale-drop metrics;
- independently supervised per-camera pipelines and joined process shutdown;
- camera-scoped health/API state and bounded-cardinality metrics;
- durable fragmented-MP4 recording, PostgreSQL indexing, reconciliation,
  multi-segment playback, and retention isolated from optional consumers;
- a Phase 0 local LiveKit spike showing that single-slice H.264 can be published
  and subscribed without pixel transcoding;
- a physical-webcam spike sustaining approximately 15 FPS after correcting
  access-unit pacing, with 10,464 forwarded packets and zero LiveKit drops;
- evidence that treating H.264 NAL units as frames breaks multi-slice pacing;
- evidence that attaching the live consumer late causes source-side slow-reader
  drops, establishing downstream-ready-before-subscribe ordering.

The Phase 4 local gates pass: formatting, warning-free workspace Clippy, 95
ordinary tests, doc tests, release build, PostgreSQL/outage/reconciliation/
playback/retention qualification, Phase 0 RTSP/crash recovery, and Phase 3
ten-camera/two-camera isolation.

Representative High Profile cameras, browser rendering, viewer concurrency,
glass-to-glass latency, production LiveKit deployment, and target hardware remain
external qualification gates.

## Current limitations that Phase 5 owns

- `crates/livekit` is a placeholder with no dependencies or implementation.
- The running pipeline drops the existing live subscription instead of owning a
  publisher consumer.
- No production LiveKit adapter, room/participant/track identity contract, or
  publication lifecycle exists.
- No publisher-specific reconnect, backoff, keyframe, parameter-set, pacing, or
  shutdown policy exists.
- No LiveKit configuration, secret boundary, token issuer, API resource, health,
  readiness, or metrics exists.
- No deterministic repository-owned LiveKit server fixture exists.
- The Phase 0 `lk` CLI/Unix-socket bridge is experiment infrastructure, not a
  production adapter.
- The official LiveKit Rust SDK documents ordinary track publication, but its
  direct pre-encoded video ingest work remains unreleased upstream pull request
  #1048 as of 2026-08-12. Do not depend on that branch without an ADR accepting
  the maintenance and upgrade cost.

## Non-negotiable invariants

1. A running camera retains exactly one upstream RTSP connection.
2. LiveKit consumes the existing complete `H264AccessUnit`; it never reconnects
   to the camera or reparses an independent RTSP session.
3. LiveKit failure, slowness, restart, or credential rejection cannot stop or
   backpressure recording, indexed playback, or unrelated cameras.
4. The live queue is bounded and freshness-oriented. Stale media may be dropped;
   latency may not grow without bound.
5. Every dropped, rejected, discontinuous, or unpublishable access unit is
   counted by a bounded category.
6. A NAL unit or slice is not treated as a frame. Publication is paced from
   complete access-unit timestamps and connection epochs.
7. Pixel decoding/re-encoding is forbidden in the H.264 pass-through path.
   Annex-B conversion, parameter-set insertion, and RTP packetization are allowed
   media adaptation and must remain explicit.
8. Publisher readiness precedes attachment to the media subscription. Startup
   loss is deliberate and observable rather than a source-side backlog.
9. Every publisher task, child process if selected, subscription, room session,
   and retry timer has one owner, cancellation signal, and join policy.
10. Camera and LiveKit API credentials never reach dashboard clients, logs,
    metrics, panic messages, URLs, or debug output.
11. Browser clients receive short-lived, camera/room-scoped LiveKit access, never
    the API secret or camera credentials.
12. Track/participant identities are deterministic, validated, camera-scoped,
    and cannot collide after reconnect.
13. Reconnect cannot leave duplicate participants, tracks, tasks, or child
    processes owned by the surveillance server.
14. Codec/profile/caps changes and connection-epoch discontinuities are explicit
    publication state transitions, not silently mixed RTP timelines.
15. LiveKit is real-time distribution only. It does not become recording storage,
    playback assembly, metadata persistence, or a domain-event transport.

## Target ownership

```text
CameraPipeline
├── Camera ingest owner
├── Recorder consumer                  preservation-oriented
└── LivePublicationOwner               latency-oriented
    ├── bounded live subscription
    ├── LiveKit room/participant session
    ├── one camera-scoped video track
    ├── access-unit/RTP adaptation
    ├── reconnect/backoff/cancellation
    └── fixed-cardinality health + metrics

ServerRuntime
├── CameraSupervisor
├── Camera/API/metrics adapter
├── LiveKit token issuer               control plane only
└── joined process shutdown
```

The `livekit` crate owns protocol/SDK adaptation and publication lifecycle. The
`media` crate owns encoded representations and distribution semantics. The
camera pipeline owns composition and task lifetime. The API validates requests
and asks a token issuer for scoped access; it does not own publisher state or
retain secrets in response models.

## Contracts to design before implementation

The following shapes describe required behavior, not mandatory Rust syntax.
Write focused contract tests before selecting or wrapping a concrete adapter.

### Adapter decision gate

Before adding a production dependency, compare at least:

1. a released official Rust SDK path that accepts pre-encoded access units, if
   one becomes available;
2. a maintained in-process WebRTC/LiveKit adapter with explicit ownership;
3. an owned external adapter process using a documented local IPC protocol;
4. an isolated transcode path only if representative H.264 cannot pass through.

Record in a new ADR:

- exact dependency/version/commit and license;
- whether it accepts complete encoded access units or raw frames;
- copies and allocations per access unit;
- RTP timestamp, SPS/PPS, keyframe request, codec profile, and reconnect support;
- queue-full and cancellation behavior;
- native/system dependencies and supported deployment platforms;
- child-process supervision and IPC bounds if out of process;
- security and secret injection boundary;
- upgrade/reversal criteria.

An unreleased SDK branch is not accepted by convenience. The `lk` CLI socket
spike may be reused only as a test oracle unless its production lifecycle,
security, compatibility, and maintenance costs are explicitly accepted by ADR.

### Publication lifecycle

At minimum distinguish:

```text
Disabled -> Starting -> Connecting -> Publishing
                         |              |
                         v              v
                      Backoff <------ Degraded
                         |
                         `----------> Terminal

Any owned state -> Stopping -> Stopped
```

Define legal transitions, retryable versus terminal failures, stable-session
backoff reset, cancellation during every state, and what happens when the camera
pipeline reconnects or changes media capabilities.

`Publishing` must imply ownership of an active room session and track. A
disconnected publisher cannot remain represented as publishing.

### Media input and overload

Define:

- queue capacity in access units and a measured byte ceiling;
- stale-drop policy and whether the newest keyframe/parameter sets are protected;
- maximum live lag before queued media is discarded;
- behavior while disconnected or backing off;
- startup ordering between publisher readiness and subscription attachment;
- channel close and camera epoch transition behavior;
- SPS/PPS cache bounds and invalidation;
- response to parameter-set changes, missing keyframes, or unsupported profiles;
- whether a LiveKit keyframe request can be propagated to an RTSP/ONVIF source or
  only causes local wait-for-next-keyframe behavior.

The producer must never await LiveKit network progress. Dropping live media is an
explicit lower-priority policy; recorder saturation retains its existing
camera-local preservation behavior.

### Timestamp and packetization

Specify how `MediaTimestamp` maps to the LiveKit/WebRTC RTP clock:

- H.264 uses a 90 kHz RTP timestamp domain;
- preserve monotonic ordering within one camera connection epoch;
- reset publication state or create a fresh RTP/track epoch on discontinuity;
- define rounding and overflow behavior for arbitrary validated timebases;
- never pace from executor arrival time alone;
- never assign one frame duration to every NAL unit;
- bound MTU packetization and reject oversized/malformed units explicitly.

Tests must cover wraparound, timestamp reset, nonzero initial timestamps,
multi-slice access units, SPS/PPS-only input, loss/discontinuity, and reconnect.

### Room, participant, and track identity

Use validated logical identities derived from `CameraId`, with an explicit
deployment namespace where needed. Define:

- one room per camera versus shared rooms;
- participant and track name stability across reconnect;
- duplicate-session takeover behavior;
- maximum lengths and allowed characters;
- camera rename/removal behavior;
- whether room creation is implicit or control-plane managed;
- cleanup of empty rooms and stale participants.

Do not expose internal database IDs, filesystem paths, RTSP URLs, or credentials
in room metadata.

### Authentication and browser access

Define a narrow token-issuer boundary with:

- LiveKit URL, API key, and API secret loaded through secret-safe configuration;
- short token lifetime and bounded clock skew;
- camera/room-scoped subscribe-only grants for viewers;
- publisher grants unavailable to browser clients;
- validated viewer identity supplied by the future authenticated application
  boundary, not arbitrary request JSON;
- no token logging, caching beyond expiry, or inclusion in metrics;
- stable external errors for unavailable LiveKit/control-plane dependencies.

Because the current loopback API has no user authentication/authorization, any
viewer-token endpoint must remain loopback/development-only or be blocked until
the Phase 8 security boundary is accepted. Do not present loopback as production
authorization.

## Work packages

### P5.1 — Refresh the LiveKit architecture decision

- Verify current official SDK and LiveKit server capabilities from primary
  sources and a pinned local spike.
- Implement the smallest adapter spike consuming the existing complete
  `H264AccessUnit`, not a second RTSP input.
- Compare in-process and owned-process choices against the adapter gate.
- Add an ADR selecting the initial implementation, native dependencies,
  security boundary, failure model, and reversal criteria.
- Name acceptance tests before production implementation.

Tests/spikes: single- and multi-slice H.264, parameter-set delivery, nonzero media
timestamp, epoch reset, queue-full feedback, cancellation, and subscriber decode.

Exit: one pinned adapter can publish repository-generated access units to a
subscriber without pixel transcode, and every copy/task/process cost is known.

### P5.2 — Add validated LiveKit domain and configuration contracts

- Implement secret-redacted endpoint/credential configuration.
- Define room, participant, track, publication state, and typed failure types.
- Add bounded reconnect/backoff, queue/lag, and shutdown settings with versioned
  defaults and clear restart/reconnect application boundaries.
- Validate H.264-only MVP capabilities and reject unsupported codec/profile
  combinations explicitly.

Tests: URL and secret validation, redacted `Debug`/errors, identity collisions,
state transitions, backoff bounds/reset, invalid queue/lag limits, unsupported
codec/profile, and transactional configuration validation.

Exit: invalid or secret-leaking publication configuration is unrepresentable at
the runtime boundary.

### P5.3 — Implement encoded H.264/RTP adaptation

- Consume complete timestamped access units.
- Convert framing only as required by the selected adapter.
- Cache and prepend SPS/PPS according to the accepted contract.
- Map timestamps deterministically to 90 kHz RTP time and handle epoch changes.
- Packetize within explicit payload/MTU bounds and classify malformed media.
- Keep payload ownership/copying visible and benchmark only if it is material.

Tests: single NAL, STAP-A-derived and FU-A-derived access units, multi-slice
frames, IDR with/without parameter sets, parameter-set change, timestamp
conversion/wraparound, discontinuity, oversized media, and allocation bounds.

Exit: the adapter input is browser-compatible RTP/H.264 under a tested contract,
with no pixel decode/re-encode.

### P5.4 — Implement one owned `LivePublicationOwner`

- Own connect, publish, event drain, retry/backoff, cancellation, unpublish, room
  disconnect, and join behavior.
- Become ready before taking the live subscription.
- Drop stale media while disconnected or slow; never block the distributor.
- Wait for a valid parameterized keyframe before beginning or resuming output.
- Treat credential/codec/configuration failures as terminal and network/server
  failures as bounded retryable states.
- Ensure all SDK callbacks or child processes remain under structured ownership.

Tests: requested shutdown in every state, publisher-first failure, camera input
close, queue saturation, slow transport, retry reset, terminal credentials,
parameter-set wait, and no detached tasks/processes.

Exit: one publication owner can repeatedly start, fail, recover, and stop with
constant memory and deterministic ownership.

### P5.5 — Attach publication to each camera pipeline

- Replace the currently dropped live subscription with the owned publisher.
- Preserve one camera connection and the mandatory recorder path.
- Start publisher readiness before attaching/feeding live media.
- Define camera-pipeline behavior when only LiveKit fails: recording remains
  running and the camera reports live publication degraded.
- Join publisher shutdown without delaying recorder finalization indefinitely.
- Allow publication-disabled cameras without constructing unused tasks.

Tests: one connection with recorder plus publisher, two cameras publishing
independently, LiveKit outage while segments continue, slow publisher drops only
live media, camera reconnect creates one fresh publication epoch, and graceful
shutdown joins all owners.

Exit: LiveKit is a real independent consumer, and killing it changes only live
publication state and live drop counters.

### P5.6 — Add per-camera publication health and observability

- Add credential-free publication state and bounded last-failure category to the
  camera health registry/API.
- Expose fixed-cardinality metrics for connection attempts, publishing state,
  reconnects, access units, bytes, keyframes, parameter-set waits, lag, queue
  depth/capacity, stale drops, packetization errors, and shutdown failures.
- Distinguish process liveness, recording readiness, and optional live-view
  degradation. A LiveKit outage must not make recording appear dead.
- Add structured state-transition logs with camera ID and fixed failure category.

Tests: snapshot lifecycle, bounded labels, secret redaction, stale failure reset,
metrics monotonicity, and API behavior while recording is healthy but live is
degraded.

Exit: an operator can diagnose connection, media, latency, and overload failure
without reading unbounded logs or exposing secrets.

### P5.7 — Add scoped viewer access

- Implement a narrow token issuer independent of media publication.
- Issue short-lived subscribe-only grants scoped to the selected camera room.
- Never issue publisher/admin grants to viewers.
- Keep the endpoint loopback/development-only until application authentication
  and authorization are accepted; label this limitation visibly.
- Return logical LiveKit connection data without camera credentials or server API
  secrets.

Tests: expiry/skew, room scope, subscribe-only grants, invalid camera, unavailable
issuer, cross-camera preparation, redacted response/logging, and public-bind
rejection without authentication.

Exit: a development browser can obtain least-privilege access without learning
any infrastructure or camera secret; production authorization remains explicit.

### P5.8 — Build deterministic LiveKit integration and outage fixtures

- Start a pinned self-hosted LiveKit server with temporary development
  credentials owned by the fixture.
- Feed deterministic single- and multi-slice H.264 through the actual Rust camera
  access-unit/distribution/publication path.
- Subscribe with an independently owned client and prove decodable video.
- Stop/restart LiveKit while recording continues and completed segment counts
  advance.
- Prove reconnection creates one participant/track and no duplicates.
- Deliberately slow or disconnect the subscriber/publisher boundary and prove
  bounded live drops without recorder rejection.
- Test SIGINT and forced publisher/server failure with complete process cleanup.

Exit: the complete single-ingest record-and-live path is reproducible locally,
including LiveKit outage isolation and recovery.

### P5.9 — Measure and close the local phase gate

- Measure access-unit ingress-to-LiveKit-server publication latency with a
  monotonic marker at p50/p90/p99.
- If a browser fixture is reproducible, measure glass-to-glass separately; do not
  substitute server-forwarding latency and call it glass-to-glass.
- Record CPU, memory, network, queue depth/drops, and reconnect time for one, two,
  and ten synthetic cameras.
- Qualify the agreed viewer concurrency when product input exists; otherwise mark
  it external and avoid a capacity claim.
- Update ADRs, roadmap, runbook/context snapshot, and failure matrix from measured
  evidence.

Exit: the locally supported codec/workload has reproducible publication,
isolation, recovery, and percentile measurements. Representative-camera/browser/
viewer-capacity claims remain blocked until their inputs exist.

## Required failure matrix

| Scenario | Recording result | Live result | Required behavior |
|---|---|---|---|
| Normal H.264 publication | continues | publishing | one track, decodable subscriber |
| LiveKit unavailable at startup | continues | backoff/degraded | bounded retry; no live backlog growth |
| LiveKit stops while publishing | continues | reconnecting | stale live media dropped; one recovery track |
| Invalid LiveKit credentials | continues | terminal | no infinite retry; secret-safe error |
| Slow publisher transport | continues | degraded | live-only stale drops; bounded lag |
| Live queue full | continues | freshest retained | drop counter advances; recorder unaffected |
| Camera disconnect/reconnect | recorder follows existing policy | fresh epoch | no timestamp mixing or duplicate track |
| Missing SPS/PPS | continues | waiting/degraded | no undecodable publication; bounded wait |
| Parameter-set/profile change | continues if recorder supports it | restart/reject by policy | explicit state and metric |
| Multi-slice access unit | continues | correctly paced | one presentation timestamp per access unit |
| Oversized/malformed access unit | existing camera policy | rejected/degraded | bounded error; no panic/allocation escape |
| Viewer disconnect | continues | publisher remains or idles by policy | no camera reconnect |
| Token issuer unavailable | continues | existing publication continues | viewer request fails safely |
| Publisher task/process crash | continues | degraded/restarted | owned join evidence; no orphan process |
| Process shutdown | finalizes recorder | unpublishes/disconnects | every owner joined within bounds |

## Observability requirements

Use validated camera IDs and fixed state/failure/operation labels only. Never
label by room token, participant token, track SID, RTSP/LiveKit URL, IP address,
viewer identity, codec string from remote input, or arbitrary error text.

Expose at least:

- publication enabled and state per camera;
- connection attempts, successes, reconnects, terminal failures, and backoff;
- live queue capacity/depth, accepted, received, stale drops, and close drops;
- access units/bytes/keyframes published and parameter-set waits;
- current and maximum observed live lag;
- timestamp discontinuities and packetization/media rejections by fixed category;
- SDK/IPC send failures, child exits, cancellation, and shutdown failures;
- active participant/track count owned by this process;
- token issue success/failure by fixed category, without token or viewer labels;
- recorder completion/failure continuity during LiveKit outage;
- measured latency histograms only with bounded buckets and explicit boundary
  names such as `publisher_ingress_to_server`, not an ambiguous `live_latency`.

## Security and deployment gates

Do not silently freeze:

- LiveKit Cloud versus self-hosted ownership and version;
- public URL, TLS termination, TURN/STUN topology, firewall/NAT behavior, and
  region placement;
- secret injection/rotation and token lifetime;
- one-room-per-camera versus shared-room topology;
- viewer authentication, authorization, audit, and revocation;
- exact H.264 profiles/levels and browser/device support;
- H.265 policy, fallback codec, and transcode permission;
- simulcast/adaptive streaming and target bitrate feedback;
- viewer concurrency and live-latency objective/workload;
- representative camera behavior and target CPU/GPU/network capacity.

Use reversible development defaults only when they are visibly labeled and
cannot masquerade as production authorization or capacity qualification.

## Validation commands

Run focused contract and fixture tests during each package. At the Phase 5 local
gate run:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --workspace --release
git diff --check
```

Also run:

- the pinned repository-owned LiveKit server/publisher/subscriber fixture;
- single- and multi-slice full subscriber decode;
- LiveKit stop/restart while recording segments continue;
- slow-publication/live-queue saturation isolation;
- Phase 0 RTSP/crash fixture;
- Phase 3 multi-camera isolation fixture;
- Phase 4 PostgreSQL/outage/playback/retention fixture;
- owned process/task leak checks before and after reconnect/shutdown; and
- latency/resource measurements with the exact workload recorded.

Report unavailable representative cameras, browsers, viewer concurrency, target
hardware/network, TURN/TLS, and production LiveKit deployment separately. Do not
replace those gates with architecture claims or the synthetic fixture.

## Phase exit gate

Phase 5 is locally complete only when:

- the selected adapter is pinned and justified by ADR;
- the actual camera access-unit fan-out feeds recorder and LiveKit without a
  second upstream camera connection;
- supported H.264 input reaches a real subscriber and fully decodes without
  pixel transcode;
- multi-slice access units and timestamps are correctly paced;
- LiveKit startup, outage, slowness, credential failure, and restart do not stop
  recording or unrelated cameras;
- queues, lag, retries, tasks/processes, and memory are bounded and observable;
- reconnect/shutdown leaves no duplicate participant, track, task, or process;
- camera-scoped live health/API/metrics expose fixed credential-free state;
- viewer access is least-privilege and explicitly development-only until the
  production authorization boundary exists;
- percentile latency is measured for a named boundary and workload; and
- all workspace and Phase 0–4 regression gates still pass.

The production phase gate additionally requires representative High Profile
cameras, supported browsers/devices, agreed viewer concurrency and latency,
target host/network, and production LiveKit/TURN/TLS/security qualification.

## Explicit first implementation action

Create or refresh a LiveKit adapter ADR and a repository-owned spike that sends
the existing `H264AccessUnit` directly into a pinned adapter boundary. Start with
one deterministic multi-slice, parameterized keyframe followed by timestamped
delta access units; subscribe and fully decode the result. Instrument queue-full,
copy/allocation, cancellation, and task/process ownership. Do not attach the
adapter to `CameraPipeline`, add token endpoints, or add an unreleased SDK
dependency until this spike selects and justifies the production boundary.

Current official sources to recheck at implementation time:

- <https://github.com/livekit/rust-sdks>
- <https://github.com/livekit/rust-sdks/pull/1048>
- <https://github.com/livekit/livekit-cli>
- <https://docs.livekit.io/transport/self-hosting/local/>
