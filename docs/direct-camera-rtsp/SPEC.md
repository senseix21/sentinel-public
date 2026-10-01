# Direct-Camera RTSP Production Qualification Specification

Status: qualification tooling implemented; no production-camera evidence has
been collected.

Date: 2026-09-01

Governing decisions:

- `docs/adr/001-media-pipeline.md`
- `docs/adr/004-rtsp-client-adapter.md`
- `docs/adr/006-multi-camera-supervision.md`
- `docs/adr/012-viewer-gated-transcode-publication.md`

Execution plan: `docs/direct-camera-rtsp/PLAN.md`.

Implemented harness: `scripts/phase9/direct-camera-qualification.sh`. It refuses
credential-bearing endpoints, starts the
release server against one physical endpoint, samples health/metrics/resources,
checks the server-owned RTSP/TCP peer, gracefully joins the server, probes and
fully decodes durable segments, and applies the deterministic metric gate in
`scripts/phase9/direct-camera-report.py`. This is qualification machinery, not
camera evidence.

### Local verification recorded on 2026-09-01

- `make check` passed, including formatting, Clippy with warnings denied, all
  non-ignored workspace tests, the release build, dependency policy, and audit.
- The authenticated Retina RTSP fixture passed its five camera-contract tests
  and three fragmented-MP4 recording tests after a reproduced publisher-process
  shutdown defect was fixed with bounded TERM/KILL cleanup.
- The synthetic multi-camera fixture passed the 10-camera supervisor and
  two-camera failure-isolation scenarios.
- The direct-camera report gate passed its success, camera-isolation, and
  failure regression tests.

These results establish a locally reproducible baseline for the qualification
tooling. They do not satisfy any physical-camera, network-disruption, sustained
soak, 10-camera, 20-camera, or target-hardware acceptance criterion below.

## 1. Purpose

Qualify the intended production topology in which each physical IP camera is
the RTSP server and the Rust surveillance server connects to it directly.
MediaMTX is not a production hop in this topology.

This work does not introduce a new media architecture. It supplies the physical-
camera evidence still required by ADR 004 and the Phase 0, Phase 3, and Phase 9
gates. Synthetic MediaMTX results prove controlled RTSP mechanics; they do not
prove interoperability with a camera model or production capacity.

## 2. Target topology

```text
Camera VLAN
    |
    | one authenticated RTSP/TCP session per camera
    v
Camera ingestion owner (Retina)
    |
    | complete, bounded H.264 access units
    v
Bounded media distributor
    |--------------------|----------------------|
    v                    v                      v
Recorder             Detector          LiveKit publisher
original H.264       sampled decode     viewer-gated decode/re-encode
    |                    |                      |
    v                    v                      v
fMP4 + metadata      domain events        LiveKit SFU -> WebRTC browser
```

The former proxy-backed deterministic fixtures and macOS webcam bridge were
removed by ADR 013. Replacement external RTSP test infrastructure is an open
qualification requirement.

## 3. Decision statement

For production cameras that satisfy this specification, configure the camera's
RTSP endpoint directly in `OneCameraSettings`. Credentials remain separate from
the endpoint. The server owns connection establishment, authentication, stream
selection, reconnect, epochs, timestamps, health, and teardown.

No recorder, detector, LiveKit adapter, API handler, or diagnostic tool may open
an independent camera session. LiveKit remains a downstream consumer; a camera
is not pointed at LiveKit Ingress and LiveKit does not become the RTSP owner.

## 4. Known facts, assumptions, and open inputs

### Known facts

- The accepted implementation uses Retina 0.4.19 and RTSP interleaved TCP.
- The implemented MVP ingest contract accepts H.264 video and assembles complete,
  bounded access units.
- Recording consumes the original encoded H.264 stream when compatible.
- Live publication uses the same distributor subscription as other consumers and
  adds no upstream camera connection.
- The default live adapter decodes and re-encodes only while a remote participant
  is present or within the bounded idle grace.
- The former authenticated, reconnect, multi-camera, and load fixtures were
  removed; their historical results are not current executable coverage.

### Reversible qualification defaults

These are workload defaults, not confirmed product requirements:

- H.264 High Profile, 4 MP, 15 FPS, approximately 4 Mbps.
- Two-second GOP.
- RTSP over interleaved TCP.
- Video only; audio and PTZ are excluded.
- 60-second segments for the accepted 2026 qualification profile. The current
  H.264 MVP constructor defaults to 30 seconds, so a run must record which value
  was resolved and must not describe the constructor default as the accepted
  workload.
- One camera for functional qualification, then 10 cameras on target hardware.

### Inputs required before execution can close

- Camera vendor, model, firmware, and stream path for each representative class.
- Measured resolution, FPS, codec/profile/level, GOP, and average/peak bitrate.
- Camera authentication mode and credential-rotation procedure.
- Target host OS, CPU/GPU, RAM, storage, filesystem, and network topology.
- Approved soak duration, viewer concurrency, and reliability/latency objectives.
- Whether audio, H.265, ONVIF, PTZ, variable resolution, or a substream is required.

Unknown values must remain unknown in the evidence report; generic datasheet
values may not substitute for SDP, packet, file, and runtime measurements.

## 5. Scope

### In scope

- Direct authenticated RTSP/TCP connection from Rust to representative cameras.
- H.264 SDP/capability validation and access-unit assembly.
- Recording, metadata, detection, LiveKit publication, API health, and metrics
  from that singular ingest.
- Camera outage, restart, network interruption, credential rejection/rotation,
  LiveKit outage, database outage, and graceful/forced server shutdown.
- One-camera correctness qualification followed by the Phase 9 realistic-camera
  workload.
- Documentation of camera-specific behavior and supported profiles.

### Out of scope

- Selecting and implementing a replacement external RTSP fixture.
- Adding LiveKit Ingress, RTMP, WHIP, SRT, or another RTSP gateway.
- H.265, audio, ONVIF, or PTZ implementation without a separately accepted
  vertical slice.
- Claiming support for a camera family based on one model or firmware version.
- Claiming support for 10, 20, or more cameras from architectural reasoning.
- Changing the external HTTP/WebRTC protocols.

## 6. Component and ownership contracts

### DC.C1 — Singular upstream ownership

For each running `CameraId`, exactly one camera ingestion owner may hold an
authenticated RTSP session. Recorder, detector, and LiveKit attach only to the
bounded distributor.

Evidence must include camera-side session/client observations when the camera
exposes them, plus process network/file-descriptor observations. Absence of a
second connection must not be inferred only from source code.

### DC.C2 — Credential separation and redaction

The RTSP endpoint contains no username, password, query, or fragment. Credentials
come from the existing secret boundary. Test output, HTTP responses, logs,
metrics, process arguments, evidence files, and command history contain no secret.

Invalid credentials are terminal until configuration changes. A temporarily
offline camera is retryable. If a vendor maps both conditions to an ambiguous
status, the evidence report records the observed classification and operator
diagnostic rather than silently changing global retry policy.

### DC.C3 — Validated media capability

Before `PLAY`, the source selects the configured H.264 video stream and validates
dimensions and configured frame/access-unit limits. Runtime packets must agree
with the negotiated capability. No fabricated codec, resolution, frame rate, or
timestamp is accepted.

Unsupported codec/profile behavior is an explicit terminal capability result for
the MVP, not an implicit transcode or fallback through MediaMTX.

### DC.C4 — Epoch and timestamp integrity

Every successful reconnect creates a fresh session epoch. RTP timestamp reset,
wrap, discontinuity, missing packets, SPS/PPS change, and mid-GOP start are
observable and cannot be joined into a false continuous media timeline.

Recording metadata derives from durable segment contents and wall-clock mapping,
not source PTS alone.

### DC.C5 — Bounded distribution

Every consumer retains its documented capacity, byte limit, overload policy, and
metrics:

- recorder: preservation-oriented; inability to admit media is visible and may
  stop/degrade only that camera;
- detector: freshness-oriented sampled queue;
- live: freshness-oriented eviction and decodable-run recovery;
- lifecycle/domain events: bounded per-class policy; no video payloads.

Measured representative access-unit sizes must fit validated bounds with stated
headroom. Bounds may be raised only from evidence and operator memory budgets.

### DC.C6 — Recording independence

LiveKit, detector, API, and database failures do not immediately stop encoded
media preservation. A LiveKit outage makes live viewing unavailable while
recording continues. Database unavailability follows the existing bounded file-
first reconciliation contract.

### DC.C7 — Owned lifetime and cancellation

The camera session, recorder, detector decoder, LiveKit decoder, and all spawned
tasks/processes have one owner, a cancellation signal, and a join/error policy.
Stop/remove/shutdown leaves no duplicate reader, orphaned FFmpeg child, open
partial without lifecycle evidence, or reconnect loop after cancellation.

### DC.C8 — Camera-local failure isolation

Failure or flapping of camera N cannot stop recording or publication for any
other camera. Retry backoff remains bounded and observable per camera.

### DC.C9 — Development-fixture separation

Production configuration and deployment documentation contain no MediaMTX
endpoint or dependency. Development and integration fixtures may continue to use
MediaMTX, but their metrics and resource costs must be reported separately from
the surveillance platform.

## 7. Functional requirements and acceptance evidence

### DC.R1 — Inventory and immutable evidence identity

Record camera vendor/model, firmware, stream selection, time zone, and a redacted
configuration fingerprint. Each evidence run records server commit, build mode,
dependency/tool versions, test interval in UTC, and operator.

Acceptance: the camera/media capability matrix has one measured row per supported
profile and no `TBD` field needed to interpret the run.

### DC.R2 — Direct connection and authentication

The server reaches the physical camera without MediaMTX or another media proxy,
authenticates using separately supplied credentials, negotiates RTSP/TCP, and
enters `Connected`.

Acceptance evidence:

- redacted server lifecycle logs;
- camera health API and connection-state metrics;
- network/process observation identifying the camera endpoint and one owned
  session;
- proof that no MediaMTX process is running or reachable in the test topology.

### DC.R3 — Truthful media qualification

Capture SDP-reported and observed codec/profile/level, dimensions, FPS, GOP,
parameter-set behavior, slice structure, average/peak bitrate, access-unit size
distribution, and RTP timestamp behavior.

Acceptance: actual completed segment probes agree with configured dimensions and
codec; FPS/bitrate/GOP claims cite measured intervals rather than camera labels.

### DC.R4 — Continuous recording

Run for the agreed one-camera soak period with recording enabled. Every expected
segment is either completed and decodable or has a typed, timestamped failure.

Acceptance:

- all finalized files decode cleanly;
- database metadata agrees with file codec, dimensions, duration, size, and
  camera/time range;
- segment gaps and partials are enumerated, not averaged away;
- recorder queue rejects/backpressure and unexplained recording failures are zero.

### DC.R5 — Reconnect and media recovery

Exercise at least: camera reboot, network interruption, RTSP service interruption
if supported, and repeated flapping. Recovery occurs without server restart or
duplicate camera sessions.

Acceptance:

- legal lifecycle transitions and fresh epochs are visible;
- backoff grows, remains bounded, and resets only after the stable-session window;
- recording resumes on a decodable run;
- gap start/end and affected segments are exact;
- unrelated cameras continue through the same interval.

### DC.R6 — Authentication rejection and rotation

Start with invalid credentials and demonstrate terminal, secret-free diagnosis.
Apply an authorized credential change and recover through the documented setting
application boundary without exposing either value.

Acceptance: no infinite retry storm, secret leakage, duplicate connection, or
unrelated-camera interruption.

### DC.R7 — LiveKit publication from the same ingest

With a browser subscribed, the camera publishes through the existing viewer-gated
adapter. With no viewer beyond idle grace, no live FFmpeg decoder or published
track remains.

Acceptance:

- browser renders advancing, nonzero-dimension video;
- source/access-unit/live-eviction/published-frame counters reconcile over the
  measurement window;
- recording remains complete during viewer join/leave;
- LiveKit stop/start affects live viewing only and recovery is observable;
- time-to-first-picture is reported with the camera GOP;
- no glass-to-glass media-latency claim is made while timestamp authority remains
  `AdapterClock`.

### DC.R8 — Detection independence

Enable deterministic inference at the configured sampling rate. Detection decoder
failure, timeout, or overload degrades detection only.

Acceptance: sampled/dropped/inferred/event counters reconcile and recording/live
continue. No production accuracy claim follows from fake inference.

### DC.R9 — Shutdown and restart

Exercise graceful shutdown and `SIGKILL` separately. Graceful shutdown joins all
owned work and consistently completes or classifies the active segment. Forced
exit preserves recoverable partial evidence; restart reconciliation handles it
within configured bounds.

### DC.R10 — Security and network boundary

Camera access is restricted to the surveillance host/network policy. Cameras are
not exposed to browsers or public networks by this change. The dashboard receives
camera-scoped LiveKit access, never RTSP credentials.

Acceptance: deployment diagram and firewall/ACL evidence name allowed source,
destination, protocol, and port without publishing secrets.

### DC.R11 — Ten-camera realistic workload

After every representative profile passes one-camera correctness, run the Phase 9
10-camera workload on confirmed target hardware. Do not replace unavailable
physical cameras with repeated synthetic streams and call it representative.

Acceptance: the workload and p50/p90/p99 measurements satisfy product-approved
objectives, or the exact limiting resource/failure is documented. A 20-camera
claim requires its own measured run.

## 8. Failure matrix

| Injection | Expected camera state | Recording | Detection | Live | Other cameras |
|---|---|---|---|---|---|
| Camera power/network loss | reconnecting with bounded backoff | gap only for that camera | unavailable | unavailable | continue |
| Invalid credentials | terminal/actionable | unavailable | unavailable | unavailable | continue |
| RTSP malformed/unsupported media | terminal or typed retryable result based on evidence | no fabricated output | unavailable | unavailable | continue |
| Recorder backpressure | camera-local degraded/stop per policy | visible failure | no priority inversion | no priority inversion | continue |
| Detector failure | connected | continue | degraded | continue | continue |
| LiveKit unavailable | connected | continue | continue | degraded/unavailable | continue |
| Database unavailable | connected | file preservation within reconciliation bound | continue | continue | continue |
| API listener failure | connected | continue | continue | existing sessions unaffected where applicable | continue |
| Graceful server shutdown | stopped after owned joins | finalized/classified | stopped/joined | unpublished/joined | all stop deliberately |
| Forced server exit | process absent | partial evidence retained | stopped | stopped | stopped |

## 9. Measurements

Every sustained run records, per camera where applicable:

- connection state, reconnect count, attempt/epoch, time to connect/recover;
- accepted bitrate/FPS, access-unit bytes and observed maxima;
- queue capacity/depth, received, evicted, stale, rejected, and lag;
- segment count, duration, size, failures, gaps, and write throughput;
- detector sampled/dropped/latency/inference/event counts;
- LiveKit viewers, published frames, drops, restarts, and first-picture time;
- process and subsystem CPU, RSS, child-process count, file descriptors;
- disk throughput/latency/capacity and network throughput/loss when measurable;
- p50/p90/p99 for objectives approved by the product owner.

Metrics must use existing bounded camera and consumer labels. Do not add camera
URLs, model strings, firmware strings, failure messages, or secrets as labels.

## 10. Evidence package

Each qualification produces a dated document under
`docs/direct-camera-rtsp/evidence/` containing:

1. scope and pass/fail conclusion;
2. exact workload and topology;
3. camera and server inventory;
4. redacted configuration and software versions;
5. commands or fixture entry point;
6. raw-artifact locations and retention policy;
7. metric tables and probe results;
8. injected failures and recovery timelines;
9. every unmet requirement and unexplained anomaly;
10. supported-profile statement narrowly derived from the evidence.

Large recordings, credentials, tokens, and unredacted packet captures are not
committed. If a small media fixture is retained, its license and test purpose are
documented.

## 11. Entry gate

- Representative camera and authorized credentials are available.
- Camera and host are on an approved isolated test network.
- Target profile and expected dimensions are known sufficiently to configure
  safe byte/count limits.
- Test duration, disruption permission, and affected-camera list are approved.
- Recording storage has sufficient measured free space plus safety margin.
- LiveKit and PostgreSQL dependencies required by the selected test stage are
  available; stages independent of them may proceed first.

## 12. Exit gate

This qualification is complete only when:

- DC.C1 through DC.C9 have evidence for every supported representative profile;
- DC.R1 through DC.R10 pass for one camera of every supported class;
- DC.R11 passes for any claimed 10-camera production workload;
- the camera/media matrix and roadmap reflect measured facts;
- open H.265/audio/PTZ/ONVIF requirements remain explicit rather than implied;
- production deployment contains no MediaMTX dependency;
- replacement external RTSP fixtures pass once selected and implemented;
- required workspace checks pass after any code or configuration change.

Until then, the correct statement is: direct-camera RTSP is the intended and
implemented topology, but production interoperability/capacity remains partially
or wholly unqualified.

## 13. Reversal criteria

Do not insert MediaMTX automatically when a camera fails. First classify the
failure against the stable camera contract.

Reconsider the Retina implementation behind that contract if a representative
camera cannot be supported safely, timestamp/loss/capability information is
insufficient, or maintenance risk exceeds the GStreamer boundary cost, as ADR
004 already permits.

A production RTSP proxy is a separate topology decision requiring an ADR. It must
name the measured incompatibility, singular connection owner, credentials,
backpressure, failure isolation, operational cost, capacity, and removal
criteria. It may not create a second upstream camera connection or make recording
depend on LiveKit availability.
