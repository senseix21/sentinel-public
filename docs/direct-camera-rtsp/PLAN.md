# Direct-Camera RTSP Qualification Plan

Status: locally implementable qualification tooling complete; execution against
physical cameras and target hardware is blocked on the external inputs in Stage
0.

Date: 2026-09-01

Specification: `docs/direct-camera-rtsp/SPEC.md`.

## Delivery rule

```text
Inventory -> Baseline -> Direct ingest -> Record -> Recover
          -> Live/detect isolation -> Soak -> Scale -> Decide
```

Complete tasks in order within a stage. A checkbox changes to `[x]` only when its
named evidence exists. Use `[!]` for an external or decision blocker and continue
with independent work.

Implemented locally:

- [x] Direct-camera harness with an explicit physical-camera acknowledgement,
  credential-free endpoint validation, an explicit direct topology, owned release-
  server lifetime, readiness/connect timeout, and private `0700`-equivalent
  evidence creation through `umask 077`.
- [x] Periodic Prometheus and CPU/RSS/child/file-descriptor sampling.
- [x] Server-owned established-RTSP/TCP evidence, graceful shutdown, full
  `ffprobe` metadata capture, and decode validation for every durable segment.
- [x] Camera-scoped metric report/gate with regression tests; another camera's
  high counters cannot make the qualified camera pass.
- [x] Evidence template and Phase 9 roadmap entry.
- [x] Local validation on 2026-09-01: `make check`, the authenticated Retina
  fixture, the 10-camera/two-camera isolation fixture, and the direct-report
  regression suite passed. This is baseline evidence, not physical-camera
  evidence.
- [!] Physical-camera, disruption, target-host, product-objective, 10-camera,
  and 20-camera stages require the external inputs named below.

## Stage 0 — Confirm authority and inputs

Goal: make the experiment safe, reproducible, and interpretable before connecting
to a camera.

- [ ] Name the camera owner/operator and obtain permission for reboot, network
  interruption, credential rejection, and credential rotation tests.
- [ ] Record vendor, model, firmware, selected stream, expected codec/profile,
  resolution, FPS, GOP, bitrate mode, audio, and ONVIF/PTZ requirements.
- [ ] Confirm the target host and network path, including camera VLAN/firewall,
  RTSP port, DNS behavior, NTP/time synchronization, and whether NAT is involved.
- [ ] Confirm soak duration, viewer concurrency, storage budget, allowable test
  gap, and objective thresholds. Record unresolved product decisions explicitly.
- [ ] Allocate camera-scoped test credentials through the secret store. Do not
  embed them in an RTSP URL, shell command, document, or packet capture.
- [ ] Estimate worst-case storage and network use from configured peak bitrate;
  verify free-space safety margin before recording.
- [ ] Create the dated evidence document from the package defined by SPEC section
  10 and record commit/tool versions before the run.

Gate: all disruptive actions are authorized and the workload can be stated
without relying on generic camera specifications.

## Stage 1 — Establish unchanged baselines

Goal: distinguish camera incompatibility from a pre-existing repository failure.

- [x] Record `git status --short`; preserve unrelated user changes.
- [x] Run focused unit suites for endpoint validation, RTSP mapping, lifecycle,
  media distribution, recorder, LiveKit owner, and supervisor contracts.
- [!] The former external RTSP reconnect and multi-camera fixtures were removed
  by ADR 013. Select and implement a replacement before claiming a reproducible
  external RTSP baseline.
- [x] Capture baseline metric names and camera-health schema; do not invent
  evidence queries after the experiment.
- [x] Record failures exactly. Fix only a reproduced defect with a regression
  test; do not mix unrelated refactoring into qualification.

Suggested focused commands, adjusted to the actual test names at execution time:

```bash
cargo test -p surveillance-camera --all-features
cargo test -p surveillance-media --all-features
cargo test -p surveillance-recorder --all-features
cargo test -p surveillance-livekit --all-features
cargo test -p surveillance-server --all-features
```

The direct physical-camera run is deliberately separate:

```bash
QUALIFICATION_ACK=direct-physical-camera \
QUALIFICATION_SECONDS=180 \
make direct-camera-qualification
```

Supply the required `SURVEILLANCE_CAMERA_*`, `SURVEILLANCE_RTSP_*`, and video
dimension variables through the operator's secret-aware process environment.
Do not paste their values into this document or a committed shell wrapper.

Gate: deterministic ingest and failure contracts pass before the physical camera
is blamed for a failure.

## Stage 2 — One-camera direct-ingest bring-up

Goal: prove DC.R1 through DC.R3 over the direct camera path.

- [ ] Ensure no RTSP proxy is part of the run and document the process and
  network topology.
- [ ] Add the camera using a credential-free `rtsp://host:port/path`, separately
  supplied credentials, actual dimensions, and live/detection initially disabled.
- [ ] Observe `Connecting -> Connected`; record DESCRIBE/SETUP/PLAY outcome,
  selected stream, authentication mode, and time to first media without secrets.
- [ ] Confirm RTSP interleaved TCP and exactly one upstream session using both
  server-side and, where available, camera-side observations.
- [ ] Compare configured dimensions with SDP and first access units. Treat a
  mismatch or unsupported codec as a result, not as permission to loosen checks.
- [ ] Measure access-unit rate/size, keyframe interval, SPS/PPS placement,
  multi-slice behavior, bitrate, and timestamp progression over a bounded window.
- [ ] Stop/remove/re-add the camera and prove teardown completes before a new
  session is admitted.

Gate: one direct session produces complete bounded H.264 access units with
truthful capability and lifecycle observations.

## Stage 3 — Recording truth and segment continuity

Goal: prove DC.R4 before optional consumers can obscure the result.

- [ ] Enable recording only and run at least three segment intervals as a smoke
  test, followed by the agreed one-camera soak.
- [ ] Probe every smoke-test segment and a documented sample/all segments from
  the soak for codec, profile, dimensions, frame rate, start/end, duration, size,
  and decode errors.
- [ ] Compare file evidence with PostgreSQL metadata and overlapping-time queries.
- [ ] Reconcile accepted/received/rejected access units, completed/failed
  segments, partials, and exact time gaps.
- [ ] Record recorder queue maximum depth, byte budget, lag, filesystem latency,
  write throughput, disk use, and process resources.
- [ ] If representative frames exceed current bounds, calculate a new bound and
  whole-fleet memory consequence before changing configuration or code.

Gate: every finalized segment examined is decodable and metadata is truthful;
there are no unexplained gaps, partials, drops, or backpressure events.

## Stage 4 — Failure, recovery, and credential behavior

Goal: prove DC.R5, DC.R6, and the camera-specific rows of the failure matrix.

Run one injection at a time with an unaffected control camera when available:

- [ ] Disconnect and restore the camera network path.
- [ ] Reboot/power-cycle the camera under operator authority.
- [ ] Stop/restore the camera RTSP service if the device exposes that operation.
- [ ] Execute bounded repeated flapping; record every attempt, backoff, epoch,
  gap, task/process count, and recovery time.
- [ ] Configure invalid credentials and verify terminal, redacted behavior without
  a retry storm.
- [ ] Rotate credentials through the authorized configuration path and verify
  recovery at the documented application boundary.
- [ ] Exercise graceful server shutdown/restart.
- [ ] Exercise forced server exit separately, inspect partial evidence, restart,
  and verify bounded reconciliation.
- [ ] Confirm a recovered stream begins on a decodable run and that timestamps
  across epochs do not fabricate continuity.

Gate: each expected recovery or terminal outcome is deterministic, observable,
secret-free, camera-local, and leaves no duplicate session or owned-resource leak.

## Stage 5 — Optional-consumer isolation

Goal: prove recording remains the priority while detection and live viewing use
the same ingest.

### Detection

- [ ] Enable deterministic inference at the configured sample FPS.
- [ ] Reconcile sampled, stale/dropped, decoded, inferred, and event counts.
- [ ] Inject decoder/inference failure or timeout using the existing controlled
  seam and prove recording continues.
- [ ] Record clearly that this is pipeline evidence, not model-accuracy evidence.

### LiveKit

- [ ] Enable publication and connect one authorized browser viewer.
- [ ] Prove the browser reports advancing video with nonzero dimensions.
- [ ] Reconcile source access units, live queue receipt/eviction/stale counts,
  adapter drops/restarts, and published frames over one timestamped interval.
- [ ] Measure join-to-first-picture with the source GOP recorded beside it.
- [ ] Remove the viewer, wait through idle grace, and prove the decoder/track
  stops while ingest and recording continue.
- [ ] Stop/restart LiveKit and prove recording/detection continue and publication
  recovers without a second camera connection.
- [ ] Do not claim glass-to-glass latency while the adapter reports
  `TimestampAuthority::AdapterClock`.

Gate: optional consumers degrade independently and no experiment changes the
singular camera connection count.

## Stage 6 — One-camera sustained qualification

Goal: close DC.R1 through DC.R10 for one representative camera profile.

- [ ] Run the approved duration with recording and the approved detection/live
  workload enabled.
- [ ] Sample metrics at a fixed interval and retain enough raw output to
  recompute totals and percentiles.
- [ ] Record CPU, RSS, child processes, file descriptors, disk/network rates,
  queue high-water marks, drops, reconnects, segment gaps, and subsystem errors.
- [ ] Inspect logs for secret leakage, retry storms, silent consumer termination,
  and errors hidden by an overall healthy state.
- [ ] Validate the documented camera-offline, LiveKit-outage, database-outage,
  and disk-full operator signals where safe and in scope.
- [ ] Repeat for every materially different codec/profile/resolution/firmware
  class proposed as supported.
- [ ] Update `docs/phase-0/camera-media-matrix.md` only with measured values.

Gate: the evidence package gives a narrow pass/fail verdict per camera profile,
with every anomaly and limitation retained.

## Stage 7 — Ten-camera target-hardware qualification

Goal: execute DC.R11 without confusing simulation with production evidence.

- [ ] Define the exact mix of representative cameras and viewed cameras.
- [ ] Establish an unoptimized one-camera target-hardware baseline.
- [ ] Run 10 realistic cameras with continuous recording, configured detection,
  and approved viewer distribution for the approved duration.
- [ ] Inject one camera outage and one optional-subsystem outage while the other
  nine continue.
- [ ] Report per-camera and whole-host CPU/GPU/RSS, disk and network behavior,
  queue drops/lag, recording gaps, recoveries, and approved p50/p90/p99 metrics.
- [ ] Identify the first measured bottleneck. Correct topology/correctness before
  micro-optimizing and change only one variable per comparison run.
- [ ] Re-run after any retained optimization and keep before/after evidence.

Gate: any supported-10-camera claim names exact hardware, camera workload,
software revision, duration, objectives, and result.

## Stage 8 — Twenty-camera measurement and topology decision

Goal: measure rather than infer the next scale step.

- [ ] Repeat the documented workload at 20 cameras, or document the exact
  resource/behavior preventing the run.
- [ ] Confirm memory and all queues remain bounded under overload.
- [ ] Record degraded-mode behavior and whether recording priority holds.
- [ ] If a deployment extraction or protocol change is proposed, create an ADR
  tied to the measured bottleneck, operational cost, and reversal criteria.
- [ ] Do not introduce a production RTSP proxy, LiveKit Ingress, worker split,
  external event broker, or distributed storage merely because it might help.

Gate: the 20-camera outcome is measured; any topology change responds to that
measurement and preserves one upstream connection per camera.

## Stage 9 — Documentation and final validation

- [ ] Publish the dated evidence package with secrets and sensitive network data
  redacted.
- [ ] Update the camera/media matrix, requirements, relevant roadmap checkboxes,
  runbooks, and latest context snapshot from evidence.
- [ ] State which camera models/firmware/profiles passed, failed, or remain
  untested. Do not generalize to a vendor family.
- [ ] Confirm production deployment manifests/configuration do not depend on
  an RTSP proxy; replacement development fixtures must exercise the same direct
  ingestion contract without becoming a production hop.
- [ ] Review ownership, bounds, overload, cancellation, failure isolation,
  metadata truth, metrics cardinality, and secret handling against the SPEC.
- [ ] Run the complete required checks:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --workspace --release
make check
```

`make check` may repeat the individual commands; record the actual command set
and results rather than counting duplicated execution as extra evidence.

Final gate: SPEC section 12 is satisfied, or every unsatisfied item is explicitly
blocked/unverified with its required next input.

## Defect workflow during qualification

For any failure:

1. preserve the smallest secret-free reproduction and exact camera/firmware;
2. classify whether it belongs to camera behavior, Retina mapping, lifecycle,
   media bounds, recorder, detector, LiveKit, configuration, or operations;
3. add a deterministic regression test at the narrowest stable contract;
4. implement the smallest vertical fix without camera-vendor coupling leaking
   across crate boundaries;
5. run focused success/failure/recovery/shutdown checks;
6. repeat the physical-camera reproduction;
7. finish with the complete validation gate;
8. update an ADR only for a material or hard-to-reverse decision.

## Stop conditions

Stop the affected run, preserve evidence, and do not broaden scope when:

- storage reaches its configured safety threshold;
- credentials or signed tokens appear in output;
- another production camera/session is affected unexpectedly;
- recording preservation is threatened by an optional consumer;
- memory, queue depth, process count, or file descriptors grow without a bound;
- the required disruptive action lacks operator authority;
- the source is H.265/audio/another unsupported profile and supporting it would
  require a new vertical slice;
- passing would require inserting an unreviewed proxy or weakening validation.
