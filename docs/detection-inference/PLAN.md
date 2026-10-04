# Detection Inference Implementation Plan

Specification: `docs/detection-inference/SPEC.md`.
Decision: `docs/adr/015-onnx-person-inference.md`.

## 1. Delivery rules

- Each slice ends with the repository gate green and the tree in a shippable
  state. No slice leaves detection half-substituted.
- The riskiest arithmetic lands before the runtime dependency. Slices 1 and 2
  add no dependency on `ort`, so the letterbox inverse and the output decoder
  are proven while they are still pure functions.
- A slice that cannot meet its exit gate stops and reports what is unverified.
  It does not borrow a later slice's evidence.
- No checkbox is ticked without the command that proves it.
- Nothing in any slice may claim accuracy or capacity. `DI.C13` applies to
  commit messages as much as to documents.
- `make check` is the gate. Individual steps may be run while iterating, but a
  slice is not done until the whole gate passes.

## 2. Dependency graph

```text
Slice 0  probe: does ort build and run here
    |
    |  (independent of 0 — pure functions, no ort)
Slice 1  letterbox arithmetic + ModelOutputDecoder seam + YOLOX decoder
    |
Slice 2  configuration surface and validation
    |
Slice 3  model artifact, session, warm-up, fail-closed startup   <- needs 0
    |
Slice 4  runtime-sourced production descriptor
    |
Slice 5  decode scaling to model input geometry
    |
Slice 6  motion gate with cooldown and heartbeat
    |
Slice 7  real model end to end, observability
    |
Slice 8  validation, measurement, snapshot, roadmap
```

Slices 1 and 2 may proceed while Slice 0 is unresolved. Slice 3 may not.

## Slice 0 — Resolve the runtime unknown — COMPLETE, 2026-10-02

Outcome: **`ort` cannot be built on the development machine**, and the model
questions were answered by other means. Findings are recorded in specification
section 3.1.

- [x] Build a throwaway crate outside the workspace depending on
  `ort = "=2.0.0-rc.13"`. Build failed:
  `no prebuilt binaries available for target x86_64-apple-darwin`.
- [x] Establish why. `ort`'s Apple support is an arm64 xcframework; Microsoft
  ships `osx-arm64` only for 1.28-1.30; PyPI macOS x86_64 wheels stop at
  1.23.2; `ort` rc.13 targets 1.28. No working version pair exists here.
- [x] Load `yolox_nano.onnx` (3,659,407 bytes, SHA-256 `c789161e...`, YOLOX
  release `0.1.1rc0`, Apache-2.0) through `onnxruntime` 1.23.2 instead.
- [x] Extract one frame from a durable recording with FFmpeg.
- [x] Record shapes and timings: input `images` `[1,3,416,416]`, output
  `output` `[1,3549,85]`; 14.7 ms p50 at default threads, 35.7 ms p50 at one
  intra-op and one inter-op thread, x86_64 CPU.
- [x] Determine the convention from the file: the export is **raw**, so grid
  and stride reconstruction belongs to the decoder; input is unnormalized
  0-255 RGB. Confirmed by producing a correct detection at score 0.815.

Exit gate, assessed:

- Runtime version and provider recorded. **Met.**
- Output shape recorded and preprocessing convention established from the
  file. **Met.**
- The contingency fired: `ort` cannot resolve a prebuilt runtime for this
  platform, so **slice 3 is blocked** pending a development platform that can
  build it. Slices 1 and 2 are unaffected and proceed.

Additional finding outside the slice's scope, recorded because it affects the
gate: CI runs `aarch64-apple-darwin` while development is `x86_64-apple-darwin`.
Adding `ort` to the workspace today would produce a build that passes CI and
fails on the developer's machine.

Nothing from the probe was kept.

## Slice 1 — Letterbox arithmetic and the decoder seam — COMPLETE, 2026-10-02

No dependency on `ort`. Everything here is a pure function over numbers.

- [x] Add a `Letterbox` type: constructed from source and target dimensions,
  exposing scale, pad offsets, the forward mapping and the inverse.
- [x] Reject degenerate dimensions in the constructor rather than dividing by
  zero later.
- [x] Add the `ModelOutputDecoder` trait: tensor plus shape plus `Letterbox` in,
  `DetectionSet` out. No frame, no session, no runtime.
- [x] Implement the YOLOX decoder against the raw export established in slice
  0: reconstruct the grid for strides 8, 16 and 32; apply
  `xy = (raw + grid) * stride` and `wh = exp(raw) * stride`; do not re-activate
  the already-sigmoid objectness and class columns; verify the reconstructed
  row count matches the tensor and return `MalformedOutput` when it does not.
- [x] Then: class filter to person, score threshold, NMS, inverse letterbox,
  clamp, truncate to `MAX_DETECTIONS_PER_EVENT`.
- [x] `DI.R1` property test for the inverse round-trip.
- [x] `DI.R2` tests against hand-built tensors, including the unexpected-shape
  path yielding `MalformedOutput` and a row count that does not match any
  stride decomposition.

Exit gate, assessed:

- `DI.R1` and `DI.R2` pass: 18 new tests, 7 for the letterbox and 11 for the
  decoder. **Met.**
- The decoder compiles and is tested with no model file present and no ONNX
  Runtime linked. `crates/detector` still depends only on `surveillance-common`,
  `surveillance-events`, `surveillance-media` and `tokio`. **Met.**
- `make verify` green: 526 tests passed, 0 failed, across 42 targets, up from
  508. Clippy clean over all targets and features. **Met.**

Two findings from writing the tests, both recorded because they are the kind of
thing that would otherwise be rediscovered later:

- The grid reconstruction was validated against the measured artifact: a
  416x416 input produces exactly 3549 rows, matching what the slice 0 probe
  read out of the real model.
- A first version of the truncation test placed its candidates at grid row 0,
  which maps entirely into the 91-pixel top padding for a 1280x720 source. The
  decoder correctly discarded every one of them. The test was wrong, not the
  code, and it now places candidates inside the image band.

## Slice 2 — Configuration surface — COMPLETE, 2026-10-02

- [x] Add the detection inference settings to `crates/config` with validating
  constructors: `model_path`, `model_sha256`, `input_width`, `input_height`,
  `execution_providers`, `intra_op_threads`, `inter_op_threads`,
  `score_threshold`, `nms_iou`, `motion_threshold`, `gate_cooldown`,
  `gate_heartbeat`.
- [x] Encode the apply-time semantics of `DI.C12`: restart-scoped settings are
  validated as a group and reported pending restart rather than partially
  applied.
- [x] Document every key in `config/example.toml` with its apply-time, and
  state that no model ships in the repository.
- [x] Reject a `model_path` that is empty, relative-escaping, or a symlink at
  validation time.

Exit gate, assessed:

- Settings round-trip. The twelve keys resolve through the compiled defaults and
  the file layer, and the shipped `config/default.toml` is held to the compiled
  defaults by an existing test. **Met.**
- An invalid value is rejected by name: a percentage above 100 yields
  `AssemblyError::OutOfRange` carrying the `SettingId`. **Met.**
- Apply-time is carried by the settings domain itself rather than by prose: the
  `settings!` table classifies every row and will not parse an unclassified one,
  so the restart-scoped and immediate split is enforced by the type system.
  **Met.**
- `make verify` green: 531 tests passed, 0 failed, up from 526. **Met.**

Three decisions the existing code forced, each departing from the written
specification and each recorded there:

- The settings value model has **no float**. Thresholds are whole percentages,
  following the storage watermarks, which also keeps them exact through TOML,
  the database and JSON and makes a NaN threshold unrepresentable.
- `ApplicationBoundary` documents that reporting `Immediate` for a value its
  owner captures at construction is the failure to avoid. The decoder was
  therefore made stateless, taking `DecodeThresholds` per call, so the
  classification is true rather than convenient.
- `Path` refuses the empty string, and `camera.detection.enabled` defaults to
  true. Giving `model_path` an empty default would have made the shipped
  configuration refuse to start once slice 3 lands. It has **no default** and
  joins `WITHOUT_DEFAULTS` instead: an unset path is how a deployment says it
  has not configured a production model.

## Slice 3 — Model artifact, session, warm-up, fail-closed startup — COMPLETE, 2026-10-03

Unblocked by making `ort` optional behind `onnx-inference` and supplying a
Linux container as the platform that can build it. The development host still
builds, tests and ships the whole workspace with the fixture backend.

- [x] Add `ort` pinned exactly to `crates/detector`.
- [x] Implement `OnnxPersonInference`: resolve path, confirm regular
  non-symlink file, read, SHA-256, compare optional pin.
- [x] Construct the session with providers registered in configured order, each
  with `.error_on_failure()`, and explicit thread counts.
- [x] Run the warm-up inference on a synthetic frame before reporting ready.
- [x] Allocate the input tensor once per session, not per frame.
- [x] Implement preprocessing: row-wise read honouring `stride`, letterbox,
  normalize per the Slice 0 finding, HWC to CHW.
- [x] Wire fail-closed startup in `apps/server`: every `DI.C7` condition
  refuses start with a classified, operator-actionable error naming the setting.
- [x] Publish the model SHA-256 on detector state.
- [x] `DI.R4`, `DI.R5`, `DI.R6` tests.

Exit gate, assessed:

- `DI.R4` through `DI.R6` pass: five artifact and session tests under the
  feature, two selection tests in both configurations. **Met.**
- A configured model that cannot be loaded refuses to start, and a configured
  model in a build without the backend refuses with a message naming the
  setting and the remedy. A build with **no** model configured keeps the fixture
  backend and reports it, which Slice 2 established is the shipped state rather
  than a failure. **Met, with that clarification.**
- No `unwrap`, `expect` or `panic!` on any path reachable from a malformed model
  file **in this code**. Not met beneath it: `ort` panics when the ONNX Runtime
  library itself will not load. Recorded in ADR 015 and specification section
  3.2 as a breach that cannot be closed from this side.
- `make verify` green on the host (531 tests) and `make container-verify-onnx`
  green in the container with the backend live. **Met.**

Findings that changed the design, recorded in specification section 3.2:

- ONNX Runtime and libwebrtc cannot be statically linked together; `ort` runs
  with `load-dynamic`, which is also what an on-device Jetson runtime needs.
- The runtime became a deployment artifact with a pinned version in the image.
- `default-features = false` silently disabled binary acquisition, and an rlib
  does not link, so the failure only appeared on every binary later.
- The prebuilt runtime needs glibc 2.38 or newer, so the image moved from
  bookworm to trixie and gained `g++`.

## Slice 4 — The production claim becomes runtime-sourced

Implemented. `DetectionHealthSnapshot` carries the live backend's descriptor,
taken from the object the owner was handed, and `GET /api/system` aggregates it
from the camera registry instead of asking what the build contains.
`compiled_backend()` is removed: its answer was a build fact, and a build can
carry the ONNX backend and run with no weights.

The aggregate under-claims on purpose. No detector running reports neither
production nor fixture, because there are no results to characterise. Any
fixture backend among the running detectors reports the whole deployment as
fixture, because one stream of fabricated detections is enough to mislead
whoever reads them.


- [ ] Derive `InferenceDescriptor` from the live inference object.
- [ ] Replace `compiled_backend()`'s build-fact meaning with the runtime
  invariant of `DI.C8`.
- [ ] Replace the existing guard test with `DI.R7` rather than deleting it,
  and state in the commit what invariant each assertion now carries.
- [ ] Confirm `/api` reports `is_production_model` and `results_are_fixture`
  from the live object.

Exit gate:

- `DI.R7` passes.
- A fixture build still reports `results_are_fixture: true`.
- An ONNX build with no weights does not report production — and, per Slice 3,
  does not start at all when detection is enabled.
- `make check` green.

## Slice 5 — Scale at decode

Implemented. The detector's FFmpeg invocation scales to the model's input
geometry with centred 114-grey padding, so frames arrive at roughly a fifth of
their former size at 1080p and the adapter copies rather than resizes.
`-hwaccel` is a per-camera setting, defaulting to software decode.

One consequence needed handling rather than assuming: with frames arriving
already padded, a letterbox derived from the frame would map detections into
padded space. The adapter now builds it once from the camera's configured
geometry, which is the only remaining record of what the camera actually sees.


- [ ] Add the scale and pad filter to the detector's FFmpeg invocation, derived
  from configured input geometry.
- [ ] Recompute the per-camera memory budget from scaled dimensions, including
  the model input tensor and the gate's retained frame.
- [ ] Make hardware decode selectable by configuration rather than hardcoded,
  defaulting to software so the development platform is unchanged.
- [ ] Record the measured before and after frame size and the budget change.

Exit gate:

- Decoded frames arrive at model input geometry.
- The memory budget check accounts for every retained buffer.
- `DI.R9` isolation parity still passes.
- `make check` green.

## Slice 6 — Motion gate

Implemented. The gate decides after sampling and before inference, admitting on
motion, within the cooldown of a recent detection, or on the heartbeat. State is
per camera and resets on a connection epoch change. Its timing also resets when
the sampler detects backward media time or a timebase discontinuity.

Frame comparison samples every 64th byte: enough to separate a still scene from
a moving one, where comparing whole frames would cost more than the inference it
avoids.


- [ ] Implement per-camera gate state: retained reference frame, cooldown
  deadline, heartbeat deadline, reset on `connection_epoch` change.
- [ ] Implement the three-way decision of `DI.C9`.
- [x] Export `surveillance_detector_gate_skipped_total` and
  `surveillance_detector_gate_heartbeat_total` with bounded per-camera labels.
- [ ] `DI.R3` state-machine tests.

Exit gate:

- `DI.R3` passes, including the cooldown and heartbeat paths and the epoch
  reset.
- Counters are fixed-cardinality.
- A stationary person within cooldown continues to be reported.
- `make check` green.

## Slice 7 — Real model end to end

Implemented. `gate_skipped`, `gate_heartbeat`, `preprocess_duration_micros` and
`output_decode_duration_micros` join the detection counters; preprocessing and
inference are counted apart because they scale with different things and one
number would hide which a change affected.

`crates/detector/tests/onnx_real_model.rs` carries DI.R8, ignored by default and
gated on the feature, taking the model and frame from the environment as the
other external-artifact harnesses do. It asserts the backend reports production,
that every box lands inside the frame, which is what catches a wrong inverse
letterbox on real output, and that the count meets a stated minimum, without
which a model detecting nothing passes. `make container-verify-model` runs it.
It is not evidence of accuracy.


- [x] Add `DI.R8`, ignored by default, taking the model path from the
  environment as the other external-artifact tests do.
- [x] Add the preprocess and decode duration counters.
- [x] Run `DI.R8` once with a real model and a frame; record the detections
  produced and the durations measured. Run 2026-10-03 on `yolox_tiny.onnx`
  from the YOLOX 0.1.1rc0 release against a street photograph holding four
  people: three detections at 0.886, 0.876 and 0.862, each on one of the three
  people that are more than a limb, each box inside the frame; load 1.73 s,
  inference 133 ms, one CPU thread, container, debug build. The run also found
  that the adapter was feeding the channels in the wrong order and that the
  test could pass on a model detecting nothing. Specification section 3.3.

Exit gate:

- `DI.R8` has been run and its output recorded. Met: specification section 3.3,
  reproducible with `make container-verify-model`.
- Durations are labelled as development-platform x86_64 CPU figures that do not
  transfer to the production target.
- `make check` green with `DI.R8` ignored.

## Slice 7a — Ground truth across a set

Added after `DI.R8` passed, because a passing `DI.R8` cannot see the class of
defect that was actually present.

`scripts/detection/build-eval-set.py` builds an evaluation set from COCO
val2017 person annotations, excluding crowd-flagged images because COCO marks
those as one unbounded region and a detector that correctly finds six people
inside one would be charged five false positives. `DI.R10`
(`crates/detector/tests/onnx_accuracy.rs`) measures recall, precision and
recall by subject size at a stated overlap, and requires both minimum
thresholds as inputs so it cannot pass by reporting.

- [x] Generate a set with real ground truth and measure the shipped pipeline:
  71.5% recall, 87.1% precision over 60 frames and 179 people at IoU 0.5.
- [x] Settle the padding placement by measurement rather than by argument.
  YOLOX pads at the origin; this pipeline centred. The same set with the
  padding moved gives 73.7% recall and 89.7% precision from the same 147
  boxes — four more people found, concentrated in small subjects, 30.4% to
  36.9%. `LetterboxAlignment` now drives the filter and the inverse from one
  value. Specification section 3.4.

Exit gate:

- A measurement exists across a set with ground truth, reproducible by someone
  who disputes it. Met: `make container-verify-accuracy`.
- The measurement is labelled as what it is. Met: it is a public general-purpose
  benchmark, not this product's cameras, and its thresholds are regression
  guards rather than accepted criteria. Phase 6's accuracy item stays open.

## Slice 8 — Validation, measurement and handoff

- [x] Full `make check` and `make audit`. 2026-10-04: `make verify` 536 passed,
  `make container-verify-onnx` 543 passed, `make audit` exit 0 with
  duplicate-crate and unencountered-licence warnings only.
- [x] Update `ROADMAP.md` Phase 6 from evidence, leaving the accuracy item
  open and using `[!]` where it is blocked on the dataset and criteria. It
  carries the measured figures and says why they do not close it.
- [x] Write a dated `context/` snapshot carrying the evidence package of
  specification section 11, and repoint `context/README.md`.
  `context/2026-10-04-2021/`.
- [x] Restate every open input from specification section 4.3 in that snapshot.

Exit gate:

- The specification's section 13 exit gate holds, item by item. Met.
- The snapshot names what was measured and what was assumed, separately. Met:
  the evidence package is one section and what remains unverified is another,
  and the durations carry the four facts that stop them being capacity figures.
- No artifact produced by this work claims accuracy or capacity. Met. The COCO
  measurement is stated as a regression guard on a public benchmark of general
  photographs, and Phase 6's accuracy item stays `[!]`.

## 3. Focused verification map

| Change | Proven by |
|---|---|
| Letterbox inverse | `DI.R1` property test |
| Output decoding | `DI.R2` constructed tensors |
| Gate decisions | `DI.R3` state machine |
| Provenance | `DI.R4` hash and path tests |
| Provider validity | `DI.R5` warm-up failure |
| Fail-closed startup | `DI.R6` refusal paths |
| Production claim | `DI.R7` descriptor derivation |
| Adapter end to end | `DI.R8`, ignored, run once |
| Isolation unchanged | `DI.R9` existing Phase 6 tests |
| Whole tree | `DI.R10` `make check` |

## 4. Review checklist before each milestone

- Does any new path `unwrap`, `expect` or `panic!` on model-derived input?
- Is every new queue, buffer or retained frame bounded and charged to the
  per-camera budget?
- Does any new metric carry a camera, recording, path or threshold as a label?
- Does any log line or event carry frame content or the model path?
- Has any claim about accuracy, capacity or latency entered a document, a
  metric name, an API field or a commit message?
- Was a test weakened or ignored to get the gate green?
- Does the API still report `results_are_fixture` truthfully for this build?

## 5. External blockers that do not stop local work

- A development platform that can build `ort`: the current machine cannot.
  This blocks slices 3 onward, not slices 1 and 2.
- Server hardware and accelerator: unconfirmed since 2026-08-14. The remaining
  slices all complete on CPU once a buildable platform exists.
- The labeled dataset and accuracy criteria: absent. No slice depends on them,
  and no slice may substitute for them.
- Cameras per unit and supported camera count: open. No slice claims either.
- A physical camera: not required. `DI.R8` takes any still letterboxed to the
  model's geometry; the run used a photograph, because what it proves is that
  the adapter and the inverse are right, and a recording would prove the same
  thing no better.

## 6. Completion statement template

> Detection inference slices 0 through N are complete. `make check` passed with
> <counts>. The adapter loads <model file> with SHA-256 <hash> and resolves the
> <provider> execution provider under ONNX Runtime <version>. Measured on
> x86_64 CPU: preprocess <a> ms, inference <b> ms, decode <c> ms; these figures
> do not transfer to the production target. `DI.R8` produced <detections> on
> <frame>. Detection accuracy remains unevaluated: Phase 6's
> accuracy item requires a labeled dataset and agreed criteria, neither of
> which exists. No capacity claim is made or implied.
