# FFmpeg Webcam Source and MediaMTX Retirement Implementation Plan

Status: proposed; no implementation slice is complete.

Date: 2026-09-04

Contract: `docs/ffmpeg-webcam-source/SPEC.md`.

This plan is ordered by risk and dependency. Each slice is intended to leave a
runnable, testable workspace. MediaMTX deletion is deliberately last because the
current committed path is working and supplies the behavioral baseline.

## 1. Delivery rules

For every slice:

1. Read the complete touched implementation and applicable ADRs.
2. Record known facts, assumptions, and unresolved questions before changing a
   public contract.
3. Reproduce the behavior or failure with the smallest reliable test.
4. Add or update the contract/test before implementation where practical.
5. Keep every channel, parser buffer, diagnostic tail, and process lifetime
   bounded.
6. Preserve unrelated dirty-worktree changes.
7. Run focused formatting, lint, and tests during development.
8. Update current documentation when a command or behavior actually changes.
9. Do not claim OS, camera-count, or latency support without the specified
   evidence.
10. Run the complete workspace and external validation gate only at the final
    locally implementable slice.

Checkbox legend follows `ROADMAP.md`:

- `[ ]` not started;
- `[~]` implemented but incompletely qualified;
- `[x]` complete with required evidence;
- `[!]` blocked by an external input or decision.

## 2. Dependency graph

```text
Slice 0: reconcile decisions/baseline
    |
    +--> Slice 1: framing spike
    |        |
    |        `--> Slice 4: FFmpeg deterministic source
    |
    `--> Slice 2: source-neutral contract
             |
             `--> Slice 3: RTSP migration with no behavior change
                       |
                       +--> Slice 4
                       |      |
                       |      `--> Slice 5: physical webcam
                       |                |
                       |                `--> Slice 6: runtime/config/API
                       |                          |
                       |                          `--> Slice 7: parity fixtures
                       |                                    |
                       |                                    `--> Slice 8: shutdown
                       |                                              |
                       `----------------------------------------------+--> Slice 9: remove MediaMTX
                                                                                 |
                                                                                 `--> Slice 10: full validation/handoff
```

The framing spike and source-contract design may be developed in parallel only
if their provisional assumptions are not committed as public API. The contract
freezes only after the spike decision is recorded.

## 3. Baseline replacement matrix

Before modifying code, populate and maintain this matrix in the implementation
work. A deletion is permitted only when the replacement column is executable or
the loss has a separately accepted decision.

| Existing MediaMTX-dependent asset | Behavior it proves | Replacement |
|---|---|---|
| `scripts/dev/local-stack.sh` | webcam, database, LiveKit, proxy, dashboard, recording | new local stack using `FfmpegWebcamSource` |
| `scripts/phase0/retina-rtsp-fixture.sh` | auth, RTSP/TCP, reconnect, saturation, recording | retain direct RTSP fixture capability or replace with owned RTSP test peer; webcam source alone is insufficient |
| `crates/camera/tests/retina_mediamtx.rs` | external Retina contract | replacement external RTSP peer test |
| `scripts/phase0/rtsp-reconnect-fixture.sh` | source outage/recovery | deterministic source kill/restart plus separate RTSP reconnect fixture |
| `scripts/phase3/multi-camera-fixture.sh` | 10-camera mechanics and two-camera isolation | deterministic multi-source fixture |
| `scripts/phase5/livekit-fixture.sh` | real LiveKit subscriber publication | webcam/deterministic source LiveKit fixture |
| `scripts/phase9/authenticated-ui-fixture.sh` | full authenticated browser stack | new source-backed browser fixture |
| `scripts/phase9/livekit-load-fixture.sh` | concurrent viewer/load mechanics | new deterministic source load fixture |
| MediaMTX YAML files | loopback RTSP path/auth configuration | delete after no executable consumer remains |
| prototype `src/mediamtx.rs` | uncompiled legacy process manager | delete independently after confirming no historical build target uses it |

The replacement need not use one tool for every row. In particular, removing
MediaMTX from webcam capture does not eliminate the need for an external RTSP
peer when testing Retina protocol behavior.

## Slice 0 — Reconcile decisions and preserve the baseline

Status: `[ ]`

Purpose: resolve documentation conflict and prevent irreversible loss of proven
behavior before implementation begins.

Tasks:

- [ ] Record the current branch, base commit, and complete dirty-worktree status.
- [ ] Review each modified/deleted file and identify ownership; do not restore or
  overwrite user changes without resolving overlap.
- [ ] Amend or supersede ADR 013. Preserve its goal of no MediaMTX dependency,
  but change the order to replacement-before-removal.
- [ ] Keep `context/2026-09-01-1049/` immutable. Record its “retain fixtures”
  statement as historical context rather than rewriting it.
- [ ] Add the replacement matrix above to the accepted ADR or link this plan.
- [ ] Capture the 2026-09-04 `make run` results in a new dated evidence/context
  document, including the shutdown defect and the fact that the run used the
  committed baseline in an isolated worktree.
- [ ] Re-run the smallest baseline fixtures needed to establish reproducibility
  if the base commit changes before implementation.
- [ ] Define the exact authority for deleting generated test artifacts versus
  repository fixtures.

Acceptance:

- [ ] One accepted ADR states the target, alternatives, consequences, migration
  order, and reversal criteria.
- [ ] No current document simultaneously claims that the same fixture is both
  retained and removed.
- [ ] The dirty worktree is preserved or its changes are explicitly incorporated.
- [ ] Baseline commands and results are reproducible from a named commit.

## Slice 1 — Resolve the FFmpeg-to-Rust framing unknown

Status: `[ ]`

Purpose: choose a bounded media transport from evidence before exposing a public
source API.

Tasks:

- [ ] Define candidate FFmpeg commands for generated H.264 with fixed dimensions,
  FPS, GOP, repeatable SPS/PPS behavior, and no B-frames.
- [ ] Compare raw Annex-B H.264 with at least one timestamp-carrying framed or
  container format.
- [ ] For each candidate, document access-unit boundary, PTS/timebase, EOS,
  corruption, and restart semantics.
- [ ] Build the smallest throwaway parser/spike outside production paths or
  behind a private experimental module.
- [ ] Feed fragmented reads, coalesced reads, a truncated stream, oversized
  declarations, timestamp resets, and corrupt bytes.
- [ ] Measure parser buffer high-water mark, source CPU/RSS, output bitrate/FPS,
  timestamp drift, and finalized recording compatibility over a sustained run.
- [ ] Confirm whether FFmpeg can emit parameter sets with every IDR under the
  chosen encoder/profile settings.
- [ ] Record the chosen format in a new ADR or an amendment to the source ADR.
- [ ] Delete throwaway code that is not the chosen foundation.

Decision gate:

- [ ] Complete access-unit boundaries are proven.
- [ ] Timestamp authority is explicit and sufficient for recorder semantics.
- [ ] Parser memory has a configured hard maximum.
- [ ] Restart and discontinuity behavior maps cleanly onto source epochs.
- [ ] The choice introduces no network listener unless its benefit is measured
  and ownership is documented.

If no candidate meets the gate, stop production implementation and evaluate a
native capture boundary or separately owned local capture service. Do not
silently derive timestamps from arrival time.

## Slice 2 — Introduce source-neutral H.264 contracts

Status: `[ ]`

Purpose: represent source choice and active-session behavior without changing
the RTSP runtime.

Likely files:

- `crates/camera/src/source.rs` (new)
- `crates/camera/src/lib.rs`
- `crates/config/src/settings.rs`
- focused tests in `crates/camera` and `crates/config`

Tasks:

- [ ] Define `H264SourceConfig` as an exhaustive enum with `Rtsp` and private or
  not-yet-runnable planned variants as appropriate. Do not expose a variant
  before its validation is real.
- [ ] Define transport-neutral source events and typed error categories.
- [ ] Decide enum dispatch versus a narrow session trait. Prefer enum dispatch
  unless a real test/construction boundary requires dynamic substitution.
- [ ] Define readiness and shutdown contracts in rustdoc.
- [ ] Preserve `H264AccessUnit` as the encoded contract.
- [ ] Move common source lifecycle names out of RTSP-specific terminology only
  where the semantics are genuinely shared.
- [ ] Add tests proving invalid source-specific field combinations cannot create
  a runnable config.
- [ ] Add compile-time/exhaustive matches so a future source kind forces each
  construction boundary to make an explicit decision.
- [ ] Calculate proposed parser/process diagnostic memory with checked arithmetic
  in the pipeline budget, initially disabled for RTSP.

Acceptance:

- [ ] No `MediaPayload` raw/encoded union is introduced.
- [ ] RTSP credentials cannot exist on a webcam config.
- [ ] Source errors preserve causes and expose bounded classifications.
- [ ] No process, socket, queue, or runnable unused variant is added.
- [ ] Focused camera/config tests and Clippy pass.

## Slice 3 — Move RTSP behind the common lifecycle with no behavior change

Status: `[ ]`

Purpose: prove that abstraction does not weaken the existing production path.

Likely files:

- `crates/camera/src/rtsp.rs`
- `crates/camera/src/supervisor.rs`
- `apps/server/src/one_camera.rs`
- `apps/server/src/pipeline.rs`
- RTSP unit/integration tests

Tasks:

- [ ] Adapt `RtspSourceConfig` and the Retina session to the common source
  construction/session boundary.
- [ ] Move reconnect, stable-session reset, epoch stamping, and shutdown
  coordination into the source-neutral owner.
- [ ] Keep RTSP authentication and capability validation inside the RTSP adapter.
- [ ] Preserve all connection states, stop reasons, health fields, counters, and
  logs unless a separately reviewed migration is necessary.
- [ ] Prove recorder-full behavior remains camera-local and loss-intolerant.
- [ ] Prove RTSP teardown completes before a new session starts.
- [ ] Run the existing RTSP unit and external fixture tests from the preserved
  baseline.

Acceptance:

- [ ] Direct RTSP produces byte/metadata-equivalent recordings for the same
  fixture workload.
- [ ] Authentication rejection remains terminal and secret-free.
- [ ] Reconnect produces a fresh epoch and stable-session reset behavior remains.
- [ ] Queue outcomes and metrics retain their meanings.
- [ ] No downstream crate knows whether the source is RTSP.
- [ ] No second upstream connection is introduced.

## Slice 4 — Implement deterministic managed FFmpeg source

Status: `[ ]`

Purpose: implement child ownership and parsing without depending on physical
hardware or permissions.

Likely files:

- `crates/camera/src/ffmpeg.rs` or `crates/camera/src/ffmpeg_source/`
- selected parser module in `crates/camera` or `crates/media` according to
  ownership determined by Slice 1
- controlled helper fixtures under `scripts/` or crate tests
- `crates/config/src/settings.rs`

Tasks:

- [ ] Add validated executable, framing, startup, inactivity, shutdown, parser,
  and diagnostic-tail settings.
- [ ] Construct `tokio::process::Command` without a shell.
- [ ] Start one deterministic FFmpeg lavfi/file-backed child.
- [ ] Drain stdout and stderr concurrently under explicit bounds.
- [ ] Parse the selected framing into existing validated NAL/access-unit types.
- [ ] Declare connected only after a valid decodable-run start.
- [ ] Classify spawn, readiness, parse, I/O, child-exit, timeout, cancellation,
  and forced-kill outcomes.
- [ ] Ensure a replacement process cannot start until the old child and drain
  tasks are joined.
- [ ] Add source/session conformance tests.
- [ ] Add malformed input and bounded-memory tests.
- [ ] Add a deterministic three-segment recording fixture.

Acceptance:

- [ ] At least three configured-duration segments probe and fully decode.
- [ ] Observed dimensions/FPS/GOP/timestamps agree with configured generated
  media.
- [ ] Killing the child produces a visible transition, bounded backoff, fresh
  epoch, and recording recovery.
- [ ] Sustained stderr cannot deadlock media.
- [ ] Cancellation at every lifecycle stage leaves no child or task.
- [ ] Recorder rejection still stops only the affected source.

## Slice 5 — Add typed cross-platform webcam configuration

Status: `[ ]`

Purpose: reuse the deterministic owner/parser with real device capture.

Tasks:

- [ ] Add an exhaustive `FfmpegCaptureBackend` for AVFoundation, V4L2, and
  DirectShow under appropriate target validation.
- [ ] Add typed device selection values; no arbitrary argument vector enters
  through config or API.
- [ ] Build per-platform arguments in small pure functions with snapshot/unit
  tests covering spaces and special characters as data, not shell syntax.
- [ ] Validate dimensions, FPS, bitrate, pixel format, H.264 profile, GOP, and
  no-B-frame first-slice constraint.
- [ ] Add device-discovery commands as operator tooling, not automatic core
  behavior.
- [ ] Classify missing device, permission denial, busy device, unsupported mode,
  and encoder absence from controlled FFmpeg results.
- [ ] Ensure configuration/debug/error output redacts or bounds device details
  according to the accepted privacy policy.
- [ ] Run the physical macOS webcam smoke first because hardware is available.
- [ ] Prepare Linux and Windows smoke instructions without claiming execution.

Acceptance:

- [ ] macOS: capture starts, reaches connected, records three valid segments,
  reconnects after controlled child failure, and stops cleanly.
- [ ] Linux/Windows command construction and validation tests pass in available
  CI/build environments.
- [ ] Linux and Windows remain explicitly unverified until physical evidence is
  collected.
- [ ] Device index/name is not hardcoded in core runtime code.

## Slice 6 — Integrate source selection through runtime and configuration

Status: `[ ]`

Purpose: make a webcam a real camera pipeline input without changing downstream
consumers.

Likely files:

- `crates/config/src/settings.rs`
- persistent configuration schema/adapters
- `apps/server/src/pipeline.rs`
- `apps/server/src/one_camera.rs`
- camera API request/response types only if source administration is authorized
- metrics/health rendering

Tasks:

- [ ] Replace the concrete RTSP field in `OneCameraConfig` with validated
  `H264SourceConfig`.
- [ ] Compose the selected source in `CameraPipeline::start`.
- [ ] Include parser and process buffers in checked pipeline memory accounting.
- [ ] Define source-change application as validated pipeline replacement.
- [ ] Preserve credential loading exclusively for RTSP sources.
- [ ] Ensure configuration responses expose no credentials or arbitrary process
  arguments.
- [ ] Add bounded source-kind and failure-category health/metrics.
- [ ] Ensure existing API clients remain compatible or version the request
  shape deliberately; do not overload `rtsp_endpoint` to carry device syntax.
- [ ] Test transactional rejection of mixed/invalid source fields.

Acceptance:

- [ ] One server can run one RTSP source and one FFmpeg source concurrently.
- [ ] Stopping/replacing either leaves the other operational.
- [ ] Recorder, detector, LiveKit, storage, and event APIs receive the same media
  contracts as before.
- [ ] No secret or high-cardinality device value appears in metrics.
- [ ] Dynamic configuration behavior matches its documented boundary.

## Slice 7 — Restore end-to-end fixture parity

Status: `[ ]`

Purpose: replace every relevant behavior before deleting its MediaMTX harness.

Tasks:

- [ ] Create a one-command local stack using the FFmpeg webcam source directly.
- [ ] Preserve PostgreSQL, LiveKit, trust-boundary proxy, and dashboard startup
  checks from the old launcher.
- [ ] Replace deterministic webcam/source outage controls without conflating them
  with RTSP protocol reconnect coverage.
- [ ] Select and ADR a repository-owned external RTSP test peer for Retina auth,
  protocol, and reconnect tests, or explicitly retain the old fixture until one
  exists.
- [ ] Rebuild the two-camera failure-isolation fixture.
- [ ] Rebuild the synthetic 10-source supervisor workload.
- [ ] Rebuild the LiveKit real-subscriber fixture.
- [ ] Rebuild the authenticated browser fixture.
- [ ] Rebuild the LiveKit viewer/load harness with source-process resource costs
  reported separately.
- [ ] Add automated checks that the runtime does not start or connect to
  MediaMTX.
- [ ] Update the replacement matrix after each passing fixture.

Acceptance:

- [ ] Every retained capability has one executable command and named evidence.
- [ ] Browser video advances and rejects an all-zero/frozen-frame false positive.
- [ ] Recording remains complete during viewer join/leave and LiveKit outage.
- [ ] Multi-camera failure remains isolated.
- [ ] Synthetic load is described only as synthetic mechanics, not physical
  capacity.

## Slice 8 — Correct graceful shutdown and forced-exit behavior

Status: `[ ]`

Purpose: close the defect reproduced during the 2026-09-04 `make run` test.

Tasks:

- [ ] Add a failing regression fixture for Ctrl-C/SIGTERM during an active
  segment.
- [ ] Make the outer launcher signal the Rust server first.
- [ ] Give the server a documented deadline derived from source teardown,
  recorder drain/finalization, optional-consumer joins, and metadata flush.
- [ ] Within the camera pipeline, stop source production before dropping the
  distributor and draining the recorder.
- [ ] Join the FFmpeg child and its stdout/stderr tasks.
- [ ] Stop shared PostgreSQL, LiveKit, and proxy processes only after the server
  has joined or the deadline is exceeded.
- [ ] Return success for operator-requested Ctrl-C.
- [ ] Retain a separate forced-kill test that expects partial reconciliation
  evidence.
- [ ] Check all ports and owned PIDs after both paths.

Acceptance:

- [ ] Graceful stop leaves no unexplained partial file.
- [ ] The final active segment is either durably finalized with truthful metadata
  or explicitly classified according to the recorder contract.
- [ ] `make run` returns success after Ctrl-C.
- [ ] No child remains and every local port is reusable.
- [ ] SIGKILL behavior still preserves and reconciles partial evidence.

## Slice 9 — Remove MediaMTX

Status: `[ ]`

Purpose: delete the dependency only after the replacement matrix is green.

Entry gate:

- [ ] Slices 1 through 8 meet their local acceptance criteria.
- [ ] An accepted ADR authorizes deletion.
- [ ] Every MediaMTX-dependent file has a passing replacement or an explicitly
  accepted capability removal.
- [ ] Direct RTSP and FFmpeg webcam paths both pass focused regression gates.

Tasks:

- [ ] Delete MediaMTX configuration files.
- [ ] Delete the uncompiled prototype process manager after confirming it is not
  referenced.
- [ ] Remove MediaMTX Make variables/targets and prerequisite checks.
- [ ] Remove or replace scripts that start/control MediaMTX.
- [ ] Remove the executable dependency from current setup documentation,
  runbooks, and CI.
- [ ] Preserve historical evidence documents as historical records; do not edit
  them to imply they used the new topology.
- [ ] Search the repository for current executable/configuration references and
  classify every remaining historical mention.
- [ ] Prove no runtime process, endpoint, or test expects ports 8554/18554 unless
  a separately selected RTSP test peer intentionally owns one.

Acceptance:

- [ ] `rg` finds no MediaMTX reference in current commands, configuration,
  production documentation, or compiled code.
- [ ] Historical references are clearly dated evidence, not runnable guidance.
- [ ] `make run` starts the replacement local stack.
- [ ] Direct-camera qualification remains no-proxy.
- [ ] Full focused parity suite passes without a MediaMTX executable installed or
  available on `PATH`.

## Slice 10 — Full validation, measurement, and handoff

Status: `[ ]`

Purpose: establish the locally supportable claim and record remaining external
gates.

Tasks:

- [ ] Run all shell syntax and helper-language regression suites.
- [ ] Run all non-ignored workspace tests and applicable ignored integration
  fixtures.
- [ ] Run the complete Rust quality gate.
- [ ] Run `cargo deny check` and `cargo audit` without weakening policy.
- [ ] Run the local macOS physical-webcam stack through capture, recording,
  detection, LiveKit subscriber, authenticated UI, reconnect, and graceful
  shutdown.
- [ ] Measure source/server/LiveKit CPU and RSS at zero and one viewer, clearly
  separating FFmpeg capture cost from viewer-gated transcode cost.
- [ ] Record queue depths/drops, parser high-water mark, source FPS/bitrate,
  published frames, segment integrity, and shutdown duration.
- [ ] Write Linux and Windows operator smoke procedures.
- [ ] Execute those procedures when hardware/environments are available; until
  then mark them `[!]` rather than blocking locally independent completion.
- [ ] Update `ROADMAP.md` conservatively.
- [ ] Create a new context snapshot and update only the latest-snapshot index.

Required final commands:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --workspace --release
cargo deny check
cargo audit
```

Acceptance:

- [ ] All locally applicable gates pass.
- [ ] The evidence names exact OS/device/workload/tool versions.
- [ ] No 10-, 20-, Linux-, Windows-, or latency-support claim exceeds evidence.
- [ ] Known limitations, external blockers, and reversal criteria are current.
- [ ] An operator can start, diagnose, and stop the supported local stack from
  current documentation.

## 4. Focused verification map

| Changed concern | Minimum focused verification |
|---|---|
| source enum/config | camera + config unit tests; invalid combination tests |
| lifecycle generalization | state-transition, reconnect, cancellation tests |
| FFmpeg process owner | spawn/exit/stderr/timeout/TERM/KILL tests |
| media parser | boundary, corruption, bounds, timestamp tests |
| recording integration | three segments, `ffprobe`, complete decode |
| LiveKit integration | real subscriber, advancing frames, metrics reconciliation |
| detector integration | configured sampling and failure isolation |
| multi-camera | fail one source while another records |
| dynamic settings/API | transaction rejection and pipeline replacement tests |
| launcher shutdown | Ctrl-C success, no orphan/port/partial; separate SIGKILL |
| MediaMTX deletion | repository reference audit and no-binary-on-PATH fixture |

## 5. Review checklist before each milestone

- [ ] Does one component have one clear owner?
- [ ] Is every async task or child process cancelled and joined?
- [ ] Is every queue/buffer bounded in items and bytes where relevant?
- [ ] Does recorder preservation remain the highest media priority?
- [ ] Can LiveKit or detection failure affect source/recording?
- [ ] Are timestamps, discontinuities, epochs, and keyframes truthful?
- [ ] Does an error retain useful context without leaking secrets?
- [ ] Did the change introduce a second source connection or decode path?
- [ ] Are defaults labeled as reversible assumptions?
- [ ] Is every claimed behavior covered by executable evidence?
- [ ] Were unrelated dirty-worktree changes preserved?

## 6. External blockers that do not stop local work

- `[!]` Physical Linux webcam qualification.
- `[!]` Physical Windows webcam qualification.
- `[!]` Confirmed production Linux distribution and packaging permissions.
- `[!]` Product-approved webcam reconnect and first-picture objectives.
- `[!]` Representative 10/20-camera workload and target hardware.
- `[!]` Audio requirements.

Record these blockers and continue through independent deterministic, macOS,
contract, and integration work. Do not fabricate completion or broaden the
scope to resolve them.

## 7. Completion statement template

Use this form at handoff:

> MediaMTX has been removed from current production and development workflows.
> Direct H.264 RTSP cameras and [named, evidenced OS/backend webcam tuples] feed
> the same camera-owned bounded H.264 distributor. Recording remains canonical
> and independent of LiveKit; LiveKit remains viewer-gated WebRTC delivery.
> The following environments and capabilities remain unverified: [...]. The
> supported workload is limited to the measurements in [...].

Do not use that statement until Slice 9 and all locally applicable Slice 10
acceptance criteria pass.

