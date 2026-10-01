# FFmpeg Webcam Source and MediaMTX Retirement Specification

Status: proposed; implementation not started.

Date: 2026-09-04

Execution plan: `docs/ffmpeg-webcam-source/PLAN.md`.

Governing requirements and decisions:

- `AGENTS.md`
- `analysis.md`
- `docs/adr/001-media-pipeline.md`
- `docs/adr/004-rtsp-client-adapter.md`
- `docs/adr/006-multi-camera-supervision.md`
- `docs/adr/011-native-encoded-livekit-publication.md`
- `docs/adr/012-viewer-gated-transcode-publication.md`
- `docs/adr/013-remove-mediamtx-code.md` (must be amended or superseded before
  implementation because its deletion order conflicts with this specification)
- `docs/direct-camera-rtsp/SPEC.md`

## 1. Purpose

Remove MediaMTX as the local webcam-to-RTSP bridge without making LiveKit the
canonical ingestion or recording owner and without losing the deterministic
integration coverage that MediaMTX currently provides.

The first deployable slice adds a managed FFmpeg webcam source to the existing
Rust camera pipeline. FFmpeg handles operating-system device capture and H.264
encoding. Rust retains ownership of lifecycle, bounded distribution, recording,
detection, health, recovery, and LiveKit publication.

This specification deliberately separates two concerns:

1. **Source neutrality now:** direct RTSP cameras, FFmpeg-backed webcams, and
   deterministic file-backed sources can enter one camera-owned lifecycle.
2. **Codec neutrality later:** H.265 or other codecs are introduced only with a
   real vertical slice and consumer support. This work remains H.264-specific.

## 2. Decision statement

Adopt this target topology:

```text
Physical H.264 camera ----> RTSP H.264 source -----+
                                                    |
USB/built-in webcam ------> FFmpeg H.264 source ---+--> H.264 source owner
                                                    |          |
Deterministic media ------> file H.264 source -----+          v
                                                   bounded distributor
                                                   |       |       |
                                                   v       v       v
                                               recorder detector LiveKit
```

LiveKit remains a downstream WebRTC delivery service. LiveKit Egress may be
used later as an optional export or secondary recording capability, but it does
not own canonical preservation, camera lifecycle, or detection input.

MediaMTX is removed only after replacement tests prove the behaviors currently
covered by its fixtures. It is not retained as a production hop or fallback.

## 3. Verified current state

The following statements are repository or test facts, not target-design
assumptions:

- `OneCameraConfig` contains a concrete `RtspSourceConfig`.
- `OneCameraRuntime` accepts `&RtspSourceConfig` and calls the H.264 RTSP
  supervisor directly.
- Retina owns one authenticated RTSP/TCP session and emits complete bounded
  `H264AccessUnit` values.
- `MediaDistributor<H264AccessUnit>` has independent bounded recorder, live,
  and detection queues.
- Recorder overload rejects new media and visibly stops only the affected
  camera rather than silently losing preservation data.
- Live and detection lanes prefer freshness and preserve decodable-run
  semantics under overload.
- Recording persists original H.264 in segmented fragmented MP4 and derives
  metadata from the resulting file.
- Detection and live publication each own separate FFmpeg decoding boundaries;
  they do not open another camera connection.
- ADR 012's default LiveKit path decodes to I420 and lets libwebrtc encode only
  while a viewer is present or within the configured idle grace.
- The encoded LiveKit path exists but is not usable with the current
  `webrtc-sys` encoder-selection behavior; it is a reversal path, not the
  default.
- On 2026-09-04, the committed `make run` path successfully captured the
  built-in macOS webcam at 1280x720, 15 FPS, H.264; connected through MediaMTX
  and Retina; recorded fully decodable 30-second segments; and published frames
  to a subscribed LiveKit viewer.
- That run also exposed a launcher shutdown defect: Ctrl-C caused the outer
  Make process to return failure and its two-second blanket child-kill deadline
  left the active recording as a `.partial.mp4` after the server logged that
  shutdown had begun.

The successful run is the behavioral baseline. It does not prove Linux,
Windows, representative camera, target-hardware, or capacity support.

## 4. Requirements status and open inputs

### 4.1 Accepted requirements

- One ingestion owner per `CameraId`.
- Recording, detection, and LiveKit attach to the same encoded stream.
- Every queue and parser allocation is bounded and observable.
- Recording outranks live viewing and detection.
- LiveKit failure cannot stop recording or camera connectivity.
- No raw frames or encoded video travel on the domain event bus.
- Device selection and capture parameters are configuration, not hardcoded
  production values.
- Child processes use `tokio::process`, have one owner, drain all pipes, accept
  cancellation, and are joined.
- The implementation must work toward Linux and Windows webcam support without
  embedding macOS-specific behavior in the pipeline contract.

### 4.2 Reversible development defaults

These values reproduce the known macOS development workload and are not product
requirements:

- H.264 video only.
- 1280x720 output.
- 15 FPS.
- 1.5 Mbps target bitrate.
- one-second GOP at 15 FPS.
- no B-frames for the first adapter slice.
- no audio.
- FFmpeg executable resolved as `ffmpeg` unless explicitly configured.
- existing recorder, live, detection, and event queue capacities.

Every resolved value must be visible in redacted startup diagnostics or
configuration inspection. A default must not be reported as a qualified
platform capability.

### 4.3 Open decision inputs

- Supported Linux distributions and webcam drivers.
- Supported Windows versions and whether DirectShow is sufficient or a Media
  Foundation-specific adapter is required.
- Device hot-plug semantics expected by the product.
- Whether multiple consumers may select different webcam formats.
- Required audio behavior.
- Whether webcam hardware encoding must be used when available.
- Approved webcam startup and recovery objectives.
- Exact p90 live first-picture and reconnect objectives.
- Target camera/viewer workload and CPU budget.

These inputs block production support claims, not the reversible development
adapter.

## 5. Scope

### 5.1 In scope

- A source-neutral H.264 session/lifecycle boundary.
- Existing direct Retina RTSP behavior behind that boundary.
- A managed FFmpeg webcam source for macOS, Linux, and Windows command
  construction.
- A deterministic non-hardware source using the same FFmpeg-output parser.
- Explicit framing, timestamp, discontinuity, epoch, and keyframe contracts.
- Bounded process I/O and parser memory.
- Typed startup, runtime, exit, and shutdown failures.
- Per-source health and bounded-cardinality metrics.
- Recording, detection, LiveKit, reconnect, multi-camera, browser, and shutdown
  regression coverage replacing the removed MediaMTX workflows.
- Final removal of MediaMTX executable/configuration/documentation dependencies.

### 5.2 Out of scope

- Making LiveKit Ingress the webcam or camera owner.
- Replacing canonical recording with LiveKit Egress.
- Routing video through RTMP, SRT, WHIP, or RTSP merely to preserve the old
  topology.
- H.265, VP8, VP9, AV1, audio, ONVIF, or PTZ implementation.
- A generic raw-and-encoded `MediaPayload` queue.
- Native V4L2, Media Foundation, or AVFoundation bindings in Rust.
- Forking or patching libwebrtc.
- Camera-count or operating-system support claims without measured evidence.
- Refactoring unrelated API, storage, authentication, or dashboard code.

## 6. System boundaries

### 6.1 Process and trust boundary

```text
Operating system / device boundary
    |
    | device identifier + capture permissions
    v
FFmpeg child process
    |
    | bounded local byte stream; no network listener
    v
Rust source session
    |
    | validated H.264 access units
    v
Rust camera/media domain
    |
    +--> local recording/storage
    +--> local detection/event domain
    `--> LiveKit SDK --> LiveKit server --> authorized browser
```

The FFmpeg command line must not contain camera passwords, LiveKit tokens, or
other secrets. A local webcam identifier is operational metadata and must not be
used as a metric label.

No local listener is required for webcam transport. If a chosen framing option
requires a socket, it must be loopback-only, randomly allocated or exclusively
created, owned by one camera session, and justified by the framing spike.

### 6.2 Crate ownership

- `crates/camera` owns source configuration contracts, source sessions,
  lifecycle, reconnect, epochs, and FFmpeg child ownership.
- `crates/media` owns H.264 access-unit representation, parser-independent media
  timestamps, decodable-run semantics, and bounded distribution.
- `crates/recorder` remains H.264/fMP4 preservation owner.
- `crates/detector` remains sampled decode/inference owner.
- `crates/livekit` remains WebRTC publication owner.
- `crates/config` validates source-specific settings transactionally and
  calculates the complete per-camera memory budget.
- `apps/server` selects concrete source configuration and composes the existing
  owners. It does not parse FFmpeg output or manage device processes directly.

No new crate is created for the first vertical slice. A separate capture crate
is permitted only if `crates/camera` gains a measured or organizationally real
second owner that cannot remain cohesive.

## 7. Core contracts

### FW.C1 — Singular source ownership

For one active `CameraId`, exactly one `SourceOwner` holds exactly one active
source session. RTSP uses one Retina session; webcam uses one FFmpeg child.
Recorder, detection, and LiveKit cannot start sources.

An attempted second owner for the same camera must be prevented by the existing
camera-supervisor map and tested at the command boundary.

### FW.C2 — Source configuration is exhaustive

Validated camera configuration identifies one source kind:

```rust
pub enum H264SourceConfig {
    Rtsp(RtspSourceConfig),
    FfmpegWebcam(FfmpegWebcamConfig),
    FileFixture(FileH264SourceConfig),
}
```

`FileFixture` is test/development-only and cannot be accepted from production
API configuration unless a later decision explicitly authorizes it.

The enum must not accept irrelevant combinations such as RTSP credentials on a
webcam source or an AVFoundation device on Linux. Validation consumes the
unvalidated settings and returns no partial runnable configuration.

### FW.C3 — One active-session contract

An active source yields a bounded stream of events:

```rust
pub enum H264SourceEvent {
    AccessUnit(Arc<H264AccessUnit>),
    Discontinuity(SourceDiscontinuity),
    EndOfStream,
}
```

The exact async interface may use enum dispatch or a narrow trait. It must make
these properties explicit:

- only a successfully started session can yield media;
- the session is exclusively mutable while being polled;
- shutdown consumes the session or otherwise prevents further reads;
- all child tasks/processes are joined before shutdown completes;
- errors retain a source and typed category without leaking command arguments
  or credentials.

Prefer an enum over boxed dynamic dispatch while the supported source set is
small and closed. Introduce a trait only when tests or a real separately built
adapter require substitution.

### FW.C4 — H.264 access-unit invariant

Every distributed item remains an `H264AccessUnit` with:

- at least one VCL NAL;
- immutable reference-counted EBSP NAL storage;
- a non-zero source epoch before publication;
- a non-zero timestamp timebase;
- validated byte and NAL-count limits;
- explicit IDR/keyframe classification;
- parameter-set visibility sufficient for `starts_decodable_run()`.

The webcam adapter must not create a weaker parallel packet type. If FFmpeg
emits Annex-B, its start codes are removed before constructing `H264NalUnit`.

### FW.C5 — Encoded and decoded planes remain separate

The canonical distributor remains encoded and H.264-specific for this slice.
Raw frames exist only behind decoder owners.

```text
H264AccessUnit distributor
    +--> recorder
    +--> detection decoder --> bounded decoded frame --> inference
    `--> live decoder ------> bounded I420 frame -----> libwebrtc
```

No queue carries an enum mixing compressed access units with megabyte-scale raw
frames. Each decoded-frame boundary documents capacity, maximum bytes, and drop
policy independently.

### FW.C6 — Lifecycle is transport-neutral

The source owner, not the RTSP or FFmpeg adapter, owns the common lifecycle:

```text
Disconnected
    -> Starting
    -> Connected
    -> Reconnecting
    -> Starting
    -> ...
    -> Terminal | Stopped
```

Legal transitions and existing camera-health meanings remain stable. RTSP
`connect` and FFmpeg `spawn/readiness` are both `Starting`; a running child that
has not produced a valid decodable unit is not yet proven media-ready.

Each successful session start receives a fresh non-zero epoch. Media from a
previous epoch cannot be distributed after the new epoch begins.

### FW.C7 — Failure classification

Source failures are classified, not string-matched by the application:

- **Retryable:** device temporarily busy, child exits after having streamed,
  temporary read failure, transient RTSP/network failure.
- **Terminal configuration/capability:** executable missing, unsupported
  platform/backend, invalid device syntax, unsupported output codec, invalid
  dimensions, parser bounds too small, authentication rejection.
- **Operator-actionable runtime:** permission denied, device removed, repeated
  startup without media, encoder unavailable.
- **Requested:** cancellation or camera removal.

Operator-actionable failures may be retryable only when the retry policy and
diagnostics prevent a tight loop. The classification must be covered by tests
for each platform-independent FFmpeg exit/parser case.

### FW.C8 — FFmpeg child ownership

`FfmpegWebcamSession` owns:

- one `tokio::process::Child`;
- piped stdout;
- continuously drained, bounded-diagnostic stderr;
- any parser buffer;
- readiness state;
- termination and join state.

It must:

- invoke FFmpeg without a shell;
- pass each argument separately;
- set stdin to null unless an explicit control protocol is adopted;
- prevent inherited stdout/stderr handles;
- drain stderr concurrently so diagnostics cannot block media;
- retain only a bounded tail or structured classification of stderr;
- never detach a drain or waiter task;
- send graceful termination where portable, wait a configured deadline, then
  force termination and wait again;
- prove the PID is gone before a replacement session starts.

Dropping the Rust owner must not be the normal shutdown strategy. A defensive
`Drop` may start no async cleanup and cannot substitute for `shutdown().await`.

### FW.C9 — Framing and timestamp decision gate

The transport between FFmpeg and Rust is not selected by this specification.
The implementation plan begins with a measured spike comparing at least raw
Annex-B H.264 and one timestamp-carrying framed/container option.

The chosen format must prove:

- complete access-unit boundaries;
- PTS and timebase semantics;
- behavior with `-bf 0` and the explicit consequence if B-frames are later
  enabled;
- SPS/PPS repetition at decodable-run starts;
- deterministic EOS and truncated-input behavior;
- bounded parser memory under malicious or corrupt length fields;
- recovery after child restart without joining timelines across epochs;
- no material corruption or pacing drift over a sustained run.

If raw Annex-B is chosen, the ADR must state how timing is derived and why that
authority is sufficient for recording. Arrival-time guesses may not masquerade
as source media timestamps.

### FW.C10 — Readiness

A webcam session becomes `Connected` only after it has produced a validated
decodable-run start compatible with configured limits. Process existence alone
is not readiness.

Startup has independent bounded deadlines for:

- child spawn;
- first diagnostic/capability result where applicable;
- first valid access unit;
- first decodable-run start.

Timeouts identify the stage in logs and metrics without logging the complete
command or device-private data.

### FW.C11 — Backpressure and memory

The FFmpeg stdout reader cannot block indefinitely while the recorder is full.
The established preservation policy remains: recorder queue saturation is a
visible camera-local stop/degradation, not permission to drop recording media.

The configured pipeline memory bound adds, with checked arithmetic:

- maximum parser buffer;
- maximum in-progress access unit;
- bounded stderr tail;
- existing encoded consumer queues;
- recorder fragment buffers;
- enabled decoded-frame buffers.

Kernel pipe capacity may be observed but is not treated as the application's
only memory bound or queue metric.

### FW.C12 — Timestamp and wall-clock mapping

The source adapter supplies media timestamps; the lifecycle owner supplies the
epoch; recorder `MediaWallClock` retains wall-clock mapping ownership.

Required behavior:

- timestamps are monotonic within the supported no-B-frame first slice;
- resets and regressions produce a discontinuity or session failure;
- each restart begins a new epoch even if FFmpeg restarts timestamps at zero;
- wall-clock segment metadata derives from recorded media and the existing
  clock contract;
- LiveKit transcode continues to declare `AdapterClock`, so no glass-to-glass
  latency claim follows.

### FW.C13 — Consumer independence

Adding webcam input changes no downstream ownership rule:

- LiveKit unavailable: recording and detection continue.
- Detector unavailable: recording and live continue.
- Database unavailable: files continue within the existing reconciliation
  bound.
- API unavailable: already-started media pipelines continue.
- Webcam source failure: only that camera reconnects or terminates.
- RTSP source behavior: unchanged and covered by regression tests.

### FW.C14 — Graceful shutdown ordering

Shutdown occurs in dependency order:

```text
refuse mutations
    -> stop source production
    -> join RTSP session or FFmpeg child
    -> drop distributor producer
    -> drain recorder queue
    -> finalize or classify active segment
    -> join optional consumers
    -> flush bounded lifecycle metadata/events
    -> remove pipeline from supervisor
    -> stop shared development infrastructure
```

The outer launcher must signal the Rust server first and allow the configured
pipeline shutdown deadline. It must not send TERM to every child simultaneously
and then impose an unrelated two-second kill deadline.

Normal Ctrl-C must return success. Forced shutdown is tested separately and may
leave a partial only as explicit reconciliation evidence.

### FW.C15 — Configuration application semantics

Source-kind changes are transactional and apply by replacing the camera
pipeline after validation. They do not mutate a live session in place.

Suggested application boundaries:

| Setting | Application boundary |
|---|---|
| source kind/device/backend | pipeline restart |
| width/height/FPS/pixel format | pipeline restart |
| encoder/profile/bitrate/GOP | pipeline restart |
| parser and access-unit bounds | pipeline restart |
| reconnect policy | next pipeline start unless existing config machinery proves safe replacement |
| recorder segment duration | next segment under existing recorder contract |
| LiveKit viewer settings | existing documented boundary |

An invalid multi-field update changes nothing and emits one audited rejection.

### FW.C16 — Security and privacy

- Webcam devices are local privileged resources; service packaging must grant
  the minimum OS permissions required.
- The API never accepts an arbitrary FFmpeg executable plus arbitrary argument
  vector. It accepts typed capture settings from which trusted code builds
  arguments.
- Executable paths and device identifiers are validated and never interpolated
  through a shell.
- Logs do not contain secrets or signed LiveKit tokens.
- Source endpoints, device names, paths, and error strings are not metric labels.
- File fixtures cannot escape their configured fixture root.
- A future non-loopback capture agent requires a new authenticated transport
  decision; it is not implied by this local adapter.

## 8. Platform adapters

### 8.1 macOS

- FFmpeg input backend: AVFoundation.
- Device is discovered and selected explicitly; index `0` remains only a
  development CLI default, never a core default.
- Camera permission denial is operator-actionable and secret-free.
- Continuity Camera warnings do not by themselves fail built-in-camera capture.

### 8.2 Linux

- First FFmpeg input backend: V4L2.
- Device is represented as a validated path under the deployment's device
  policy; `/dev/video0` is not hardcoded in core configuration.
- Packaging must document service-user group/ACL requirements.
- Format negotiation and hardware encoder availability are measured on target
  hardware before support is claimed.

### 8.3 Windows

- First FFmpeg input backend: DirectShow because it is broadly available in
  FFmpeg builds; this is a reversible development choice, not a rejection of
  Media Foundation.
- Device selection uses an exact validated device name or stable identifier
  supported by the selected FFmpeg build.
- Argument construction must handle spaces without shell quoting because the
  process API receives separate arguments.
- Media Foundation becomes a separate adapter only if DirectShow limitations
  are reproduced and acceptance criteria are defined.

## 9. Observability

Existing camera and queue metrics remain authoritative. Proposed new metrics
must be added only with tests proving their presence and bounded labels.

Required observations per camera/source kind:

- configured source kind as a bounded enum value in health or one-hot metric;
- source spawn/connect attempts and successes;
- startup-stage failures by bounded category;
- session restarts and epochs;
- bytes read and access units produced;
- parser malformed, oversized, truncated, and timestamp-discontinuity counts;
- FFmpeg exits by bounded classification;
- graceful and forced child terminations;
- current parser-buffer usage/capacity if cheaply observable;
- existing queue accepted/received/rejected/dropped/depth/capacity;
- segment completion/failure;
- detector and LiveKit health.

Do not place camera URLs, device identifiers, executable paths, PIDs, raw exit
messages, or exception text in labels. Logs may carry PID and redacted device
identity as fields when operationally necessary.

Health must distinguish:

- process alive;
- source starting/reconnecting/terminal;
- camera/media ready;
- recorder degraded;
- optional detection/live degradation.

## 10. Failure matrix

| Failure | Source state | Recording | Detection | LiveKit | Other cameras | Required action |
|---|---|---|---|---|---|---|
| FFmpeg executable absent | terminal | unavailable | unavailable | unavailable | continue | fix installation/config |
| Camera permission denied | operator-actionable terminal or bounded retry per platform evidence | unavailable | unavailable | unavailable | continue | grant permission |
| Device busy at startup | reconnecting with bounded backoff | unavailable | unavailable | unavailable | continue | release device |
| Device unplugged | reconnecting; fresh epoch after return | gap/classified partial only | unavailable | unavailable | continue | reconnect device |
| FFmpeg exits unexpectedly | reconnecting or terminal by exit class | gap only for camera | unavailable | unavailable | continue | inspect bounded diagnostic |
| FFmpeg stdout stalls | bounded timeout and restart | visible gap/failure | unavailable | unavailable | continue | restart source |
| Malformed/oversized output | typed source failure; no unbounded allocation | no corrupt finalized file | unavailable | unavailable | continue | inspect encoder/parser |
| Recorder queue full | camera-local stop/degrade | visible preservation failure | no priority inversion | no priority inversion | continue | resolve storage/throughput |
| Detector failure | connected | continue | degraded | continue | continue | restart/inspect detector |
| LiveKit failure | connected | continue | continue | degraded | continue | restart LiveKit path |
| Database unavailable | connected | file-first preservation within reconciliation bound | continue | continue | continue | restore database |
| Graceful server stop | stopped after joins | active segment finalized or explicitly classified | joined | unpublished/joined | all stop intentionally | none |
| Forced server kill | process absent | partial retained as reconciliation evidence | stopped | stopped | stopped | restart/reconcile |

## 11. Testing and evidence requirements

### FW.R1 — Contract tests

Run the same source-session conformance suite against RTSP, deterministic FFmpeg,
and webcam-compatible fake/process fixtures where applicable:

- readiness requires valid media;
- complete access units and parameter sets;
- non-zero epoch stamping;
- timestamp monotonicity/discontinuity;
- end-of-stream classification;
- cancellation while starting, streaming, and reconnecting;
- no task or process remains after join.

### FW.R2 — Parser tests

Tests cover:

- split boundaries across reads;
- multiple units in one read;
- truncated final unit;
- zero/oversized lengths where applicable;
- maximum legal access unit and NAL count;
- missing VCL;
- SPS/PPS/IDR classification;
- timestamp reset and regression;
- arbitrary byte input through property tests when useful.

No malformed input may panic or grow memory beyond configured bounds.

### FW.R3 — Managed-process tests

Use controlled helper processes/scripts to prove:

- spawn success and failure;
- stderr is drained under sustained output;
- media output is not blocked by stderr;
- early exit preserves a bounded diagnostic category;
- graceful termination joins;
- unresponsive child is forcibly terminated after the configured deadline;
- cancellation during startup leaves no process;
- a replacement child starts only after the prior PID is gone.

### FW.R4 — Recording parity

For one deterministic source and one physical webcam:

- create at least three complete configured-duration segments;
- fully decode every finalized segment;
- verify codec, dimensions, FPS, duration, and size with `ffprobe`;
- reconcile database metadata with file observations;
- require zero recorder queue rejection and unexplained gaps.

### FW.R5 — Failure and recovery parity

Reproduce publisher/device loss and recovery:

- observe legal state transitions and bounded backoff;
- obtain a fresh epoch;
- resume on a decodable run;
- measure the exact recording gap;
- show unrelated camera pipelines continue;
- prove no duplicate FFmpeg child exists.

### FW.R6 — LiveKit parity

With no viewer beyond idle grace:

- room signaling may remain connected;
- no live decoder process or published track remains;
- published-frame count remains flat.

With one viewer:

- the viewer subscribes to the expected camera-scoped track;
- video dimensions are non-zero and frames advance;
- published frames increase;
- send failures and unexpected restarts remain zero;
- recording remains complete.

No glass-to-glass latency claim is made while timestamp authority is
`AdapterClock`.

### FW.R7 — Multi-camera isolation

Run at least two independent sources, fail one, and prove the other continues
recording and publishing. A synthetic 10-source workload may test supervisor and
bounded-resource mechanics but cannot establish physical-camera capacity.

### FW.R8 — Shutdown parity

Graceful Ctrl-C/SIGTERM must:

- return success from the launcher;
- stop source production before recorder drain;
- join the server within its configured deadline;
- leave no orphaned FFmpeg, LiveKit, PostgreSQL, proxy, or server process;
- leave no unexplained `.partial.mp4`;
- make all bound ports immediately reusable.

Run SIGKILL separately and prove the expected partial/reconciliation behavior.

### FW.R9 — Cross-platform qualification

Support is claimed separately for each tested tuple:

```text
OS/version + FFmpeg version/build flags + capture backend + device class
+ resolution + FPS + pixel format + encoder/profile
```

macOS success does not imply Linux or Windows success. CI may cover command
construction and controlled child behavior without physical devices; physical
smokes remain documented manual gates.

### FW.R10 — Full repository gate

After each material slice run focused tests. Before MediaMTX deletion is called
complete, run:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --workspace --release
```

Also run `cargo deny`, `cargo audit`, applicable shell/Python/Node checks, and all
new ignored external fixtures. Unavailable operating systems or devices are
reported as unverified, never silently skipped in the completion statement.

## 12. Evidence package

Each manual or sustained qualification records:

1. commit and dirty-state fingerprint;
2. OS, architecture, FFmpeg version/configuration, and capture backend;
3. device class and redacted/stable identifier policy;
4. resolved capture and encoding configuration;
5. selected framing format and parser limits;
6. test interval and shutdown method;
7. source, queue, recorder, detector, and LiveKit metrics;
8. process CPU/RSS and child count;
9. segment probes and complete decode results;
10. failure/recovery timeline;
11. orphan/port/partial-file checks;
12. exact limitations and unsupported claims.

Credentials, tokens, private endpoints, unredacted process environments, and
large recordings are not committed.

## 13. Migration invariants

At every intermediate commit:

- the workspace compiles;
- direct RTSP remains runnable;
- exactly one source owns media for each camera;
- the recorder consumes the same `H264AccessUnit` contract;
- LiveKit remains downstream;
- every new queue and buffer has a bound;
- no MediaMTX file is deleted unless its behavior is already covered or the
  loss is explicitly accepted and recorded;
- historical evidence remains unchanged;
- current docs distinguish available commands from historical commands.

## 14. Entry gate

- This specification and its plan are reviewed.
- ADR 013 is amended or superseded to adopt replacement-before-removal order.
- Existing uncommitted deletions are classified file by file; no user work is
  overwritten.
- The committed MediaMTX `make run`, RTSP fixture, and multi-camera fixture are
  identified as baseline evidence.
- FFmpeg with libx264 is available for the first local spike.
- A small redistributable deterministic H.264 input or generated test source is
  selected.
- Parser memory and startup/reconnect defaults are proposed explicitly.

## 15. Exit gate

MediaMTX retirement is complete only when:

- FW.C1 through FW.C16 are implemented or explicitly narrowed by an accepted
  decision;
- FW.R1 through FW.R8 pass locally;
- cross-platform command construction is tested;
- physical webcam evidence exists for every claimed operating system;
- direct RTSP behavior remains qualified to the extent it was before this work;
- reconnect, multi-camera, LiveKit, browser, and load harness capability is
  restored without MediaMTX or explicitly removed by a separately accepted
  product decision;
- graceful shutdown no longer reproduces the 2026-09-04 partial-file/exit-status
  defect;
- production and current development documentation contain no required
  MediaMTX dependency;
- no running configuration references a MediaMTX endpoint;
- the full repository gate passes;
- `ROADMAP.md` and a new context snapshot report verified facts and remaining
  external gates.

Until then, the truthful status is:

> MediaMTX removal is in progress. Direct physical-camera RTSP remains the
> production topology, and the FFmpeg webcam source is not supported until its
> applicable acceptance evidence exists.

## 16. Risks and mitigations

| Risk | Probability | Impact | Mitigation |
|---|---:|---:|---|
| Pipe format loses timestamps or boundaries | medium | high | spike before contract freeze; retain RTSP baseline |
| FFmpeg child deadlocks on stderr/stdout | medium | high | concurrent drains, bounded tests, owned joins |
| Shutdown kills recorder before finalization | already observed | high | ordered shutdown and regression fixture |
| Source abstraction weakens H.264 invariants | medium | high | keep `H264AccessUnit`; conformance tests |
| Cross-platform device behavior diverges | high | medium | typed backend configs and per-OS evidence |
| Webcam reconnect loop spins on permissions | medium | medium | typed classification and bounded backoff |
| Early deletion loses integration coverage | already occurring | high | replacement matrix and deletion gate |
| LiveKit becomes accidental critical path | low if design held | high | keep downstream-only boundary and failure tests |
| Speculative codec abstraction expands scope | medium | medium | H.264-only slice; exhaustive later extension |
| FFmpeg CPU cost limits camera count | unknown | high | measure exact workload before native/hardware work |

## 17. Reversal criteria

Reconsider the FFmpeg child-process source if measured evidence shows any of:

- no available bounded framing option preserves required timestamps and access-
  unit boundaries;
- child/process failure behavior cannot meet recording preservation objectives;
- CPU or memory cost exceeds the confirmed target budget;
- required device controls cannot be expressed safely through typed settings;
- platform support requires incompatible semantics that make one contract
  misleading.

If reversed, evaluate native capture or a separately owned local capture service
against the same source-session contract. Reintroducing MediaMTX as a production
hop, adopting LiveKit Ingress as canonical ingestion, or adopting LiveKit Egress
as canonical recording each requires a new ADR with measured motivation,
failure ownership, operational cost, and a rollback path.

