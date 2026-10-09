# Face-only live detection — implementation plan

Date: 2026-10-09
Status: local implementation complete; external qualification remains open.
Contract: [SPEC.md](SPEC.md). Evidence: [VERIFICATION.md](VERIFICATION.md).
Checked tasks describe completed implementation; unchecked tasks retain their
original broader validation requirements.

## Slice 1 — Reproduce, qualify a model and decide the boundary

- [ ] Capture the current person path using deterministic file-backed RTSP input;
  record source geometry, connection count, queue bounds and recording continuity.
- [x] Inventory decoder preprocessing and model scratch/frame budgets. Identify
  the smallest owner dispatch seam without changing camera connectivity.
- [x] Compare face-specific ONNX candidates using their authoritative model
  documentation and redistributable artifacts. Verify license and checksum.
- [ ] Run the specification's labeled frame matrix at 70% on available CPU;
  report detections, misses, negatives, face size, memory and latency percentiles.
- [x] Write an accepted reversible ADR covering direct face inference versus person-crop
  inference, one mode versus simultaneous models, preprocessing detail, ownership,
  runtime bounds, artifact selection and reversal. Direct full-frame face inference
  is the proposed MVP; person detection must not gate whether a face is attempted.

Exit: a documented artifact contract and reversible selection; production criteria
remain explicitly open if unavailable. No acquisition of new camera connections.

## Slice 2 — Domain/configuration contracts before runtime wiring

- [x] Add validated face result/event types without renaming or repurposing person
  payloads. Cover max 64, finite normalized bounds and threshold edge cases.
- [x] Extend event kind/delivery classification, bus tests and all exhaustive
  consumers. Face events use bounded lossy detection delivery, not lifecycle queues.
- [x] Add explicit mode and face artifact/threshold settings to defaults, parsing,
  assembly, persistence, validation, environment overrides and audit classification.
  Preserve person mode behavior and explicit overrides; use 70% for face scores.
- [x] Add artifact consistency, restart semantics and frame-budget tests.

Exit: deterministic tests compile across all feature combinations; a face cannot
be represented as a person event and invalid configuration cannot start inference.

## Slice 3 — One-camera face inference vertical slice

- [x] Implement the artifact-specific ONNX adapter with verified preprocessing,
  bounded candidates, threshold filtering, NMS and source-coordinate inversion.
- [x] Reuse inference failure classification and provenance; keep fixture output
  distinguishable. Add model-descriptor reporting for mode and availability.
- [x] Wire face mode through the existing owned decoder/sample/inference pipeline.
  Preserve cancellation, bounded blocking execution and freshness preference.
- [x] Publish real face observations, including empty results; omit person tracking
  and coasting in face mode. Do not add recognizer/identity state.
- [ ] Test malformed output, absent/corrupt/wrong-shape artifacts, timeout, gate
  heartbeat, reconnect, model failure, overload and shutdown. Record connection
  count and segment continuity during detector failures.

Exit: one camera produces real, bounded face events while recording continues.

## Slice 4 — Camera-scoped transport and face-only UI

- [x] Add the `face/v1` packet/relay path with per-epoch sequence and a 16 KiB limit.
  Keep the existing person wire path intact. Test room-camera filtering and drops.
- [x] Extend the owned relay subscription/lifecycle; ensure shutdown joins all work.
- [x] Validate face packets before drawing: schema, size/count, finite geometry,
  confidence, camera, fixture, epoch and sequence. Keep one latest packet per tile.
- [x] Render face rectangles using the existing content-rectangle transform.
  Clear immediately on empty sets or lifecycle invalidation; test age expiry,
  reorder, old epochs, DOM reconciliation, resize, aspect ratios and DPR.
- [x] Label face-mode counts “Faces detected”; remove unique-person/session totals
  from that mode. Show face detector unavailability independently of live status.

Exit: deterministic UI tests show only face boxes; malformed/stale/person packets
cannot populate the face overlay or inflate person counts.

## Slice 5 — Default launcher, real browser proof and handoff

- [x] Once qualified, make face mode the `make run` default with checksum-pinned
  cached artifacts, no hardcoded credentials, detection opt-out and explicit
  person-mode compatibility. Document custom artifact and geometry overrides.
- [ ] Run file-backed RTSP -> face inference -> event bus -> LiveKit -> dashboard
  with full-body, multiple-face and no-face fixtures. Prove one upstream session,
  unaffected original recordings and two concurrent viewers.
- [ ] Verify actual Chromium and Firefox decoding/face overlays separately; record
  the existing Firefox/WebRTC problem as unverified if it still prevents a run.
- [ ] Perform a manual Mac webcam smoke: face-only bounds, turn-away clearing,
  full-body framing, 70% default, detector recovery and graceful stop. Physical
  webcam evidence supplements deterministic fixtures and is not a CI dependency.
- [ ] Run `cargo fmt --all --check`, workspace all-target/all-feature Clippy with
  `-D warnings`, workspace all-feature tests, workspace release build, relevant
  UI/shell/launcher tests and security checks required by `make check`.
  Provide ONNX runtime environment explicitly. Report external fixture skips and
  the current macOS SQLx release-linker failure if unresolved; never weaken gates.
- [x] Update README, API/capability docs, ROADMAP and a new immutable dated context
  snapshot with actual evidence, artifact pin, measured limits and remaining gates.

Exit: SPEC acceptance criteria have recorded evidence or explicit external
blockers. No production face accuracy or multi-camera capacity claim without its
required dataset and workload. Implementation completion requires the full local
slice, not merely successful model loading.

## Dependencies and rollback

Slices 2's deterministic contracts can progress while slice 1's hardware/accuracy
questions remain open. Runtime slices depend on a verified artifact contract;
launcher default changes depend on real end-to-end qualification. Keep person mode
as the rollback path with its original events and topic. Mode switches require
pipeline restart and clear all face overlay state. Face adapter failures report
unavailable, rather than automatically drawing person boxes.

## Remaining qualification

The pending combined validation tasks are partially exercised: corrupt-model
failure isolation, malformed output, threshold boundaries, two-face and blank
frames, shutdown, and two Chromium viewers passed. The representative labeled
pose/light/size matrix, file-backed RTSP qualification, actual Firefox playback,
and full manual turn-away/recovery matrix remain unverified. Formatting, Clippy,
workspace tests and release build with the documented macOS workaround passed;
security tooling is missing. These are qualification gaps, not implemented
production guarantees. See VERIFICATION.md for exact evidence.
