# Detection Inference Specification

Scope: replacing `DeterministicFakeInference` with a real ONNX person detector
behind the existing `PersonInference` trait.

Governing decision: `docs/adr/015-onnx-person-inference.md`.

Contract IDs are `DI.C*`; verification requirements are `DI.R*`.

## 1. Purpose

Detection is currently a wired seam with a fixture behind it. The pipeline —
sampling, decode supply, overload policy, bounded memory, event publication and
failure isolation — is implemented and proven by Phase 6. The model is not.

This specification defines the adapter that makes detection real, the contracts
it must honour, and the exact boundary between what the work proves and what it
leaves unproven.

## 2. Decision statement

In-process ONNX Runtime through a pinned `ort`, behind `PersonInference`, with a
second family-scoped seam for output decoding. YOLOX-Tiny is a reversible
documented default, not a confirmed requirement. Weights are a deployment
artifact identified by SHA-256. Startup is fail-closed. The production claim is
derived from the live session, never from the build.

## 3. Verified current state

Verified 2026-10-02 against `main` at `c88aed3`.

- `crates/detector` is 1,590 lines across `decoder.rs`, `detector.rs`,
  `inference.rs`, `sampler.rs` and `lib.rs`. It depends on
  `surveillance-common`, `surveillance-events`, `surveillance-media` and
  `tokio`. It has no machine-learning dependency, and `ort` is absent from the
  workspace.
- `PersonInference` is the substitution boundary. `infer` is synchronous and
  takes `&mut self`. `is_production_model()` defaults to `false` so an
  implementation that does not answer is presumed not to be production.
- `DeterministicFakeInference` is the only implementation. It emits one person
  when the first RGB byte is odd.
- `compiled_backend()` returns an `InferenceDescriptor` built by constructing
  the fake. It is a build fact with no runtime input. `apps/server` asserts the
  reported descriptor matches what the pipeline builds, and `/api` serves
  `is_production_model: false` with `results_are_fixture: true`.
- `detector.rs` already runs inference on `tokio::task::spawn_blocking`, clones
  the frame (`DecodedFrame::pixels` is an `Arc<[u8]>`), moves the inference
  object across the thread boundary and back, counts
  `inference_duration_micros`, and maps a join failure to
  `DetectionStopReason::TerminalInference`. A `SlowInference` test covers
  shutdown landing mid-inference.
- `DecodedFrame` carries `camera_id`, `connection_epoch`, `media_timestamp`,
  `observed_at`, `width`, `height`, `stride`, `format` and
  `pixels: Arc<[u8]>`.
- The detector's decoder invokes FFmpeg with `-pix_fmt rgb24` and **no scale
  filter and no `-hwaccel`**, so frames arrive at full source resolution via
  software decode, one child process per camera. At 1080p that is 6.2 MB per
  frame; at 1440p, 11 MB.
- `MAX_DETECTIONS_PER_EVENT` is 64.
- No configuration key names a model file. `model_path`, `weights` and `.onnx`
  appear nowhere in `config/` or `apps/server`.
- No model artifact exists in the repository or on the development machine.

### 3.1 Slice 0 probe findings

Established 2026-10-02 by running the model, not by reading documentation.

- **`ort` cannot be built on the development machine.** `ort-sys` 2.0.0-rc.13
  reports `no prebuilt binaries available for target x86_64-apple-darwin`; its
  Apple support is an xcframework for arm64. Microsoft's ONNX Runtime releases
  1.28 through 1.30 ship `osx-arm64` only, and PyPI wheels for macOS x86_64 stop
  at 1.23.2. `ort` rc.13 targets ONNX Runtime 1.28, so no working version pair
  exists for this platform.
- **The development machine and CI are different architectures.** CI runs
  `macos-26-arm64` and resolves `1.92.0-aarch64-apple-darwin`; the development
  machine is `x86_64-apple-darwin`. The toolchain pin fixes the compiler
  version, not the target triple.
- `yolox_nano.onnx` (3,659,407 bytes, SHA-256
  `c789161ed43c8269fcd4e67c67eeeb4e80c622da2eb296a20bc6007bd18a0b7d`) declares
  input `images` `[1, 3, 416, 416]` float32 and output `output`
  `[1, 3549, 85]` float32. 3549 is 52^2 + 26^2 + 13^2, the three stride levels
  concatenated.
- **The export is raw.** `decode_in_inference` was not applied: the first four
  columns ranged -2.07 to 3.48, which is neither pixel nor normalized space.
  Objectness and class columns are already sigmoid-activated.
- Preprocessing confirmed by producing a correct detection: raw 0-255 values,
  **no** division by 255, RGB channel order, HWC to CHW, 114-grey letterbox
  padding. A wrong convention yields nonsense rather than a degraded result.
- Measured on x86_64 CPU with `onnxruntime` 1.23.2: 14.7 ms p50 at default
  thread counts, **35.7 ms p50 with intra-op and inter-op threads set to 1**.
  These figures do not transfer to the production target.
- `get_available_providers()` reported `CoreMLExecutionProvider` on a machine
  where it is not usable for this model, confirming `DI.C6`.
- YOLOX-Nano at 416 detected a person filling the frame at score 0.815 and did
  not detect a second, smaller person in the background. One frame is not an
  accuracy measurement; it is consistent with the known weakness of this size
  and input resolution on small subjects.

### 3.2 Slice 3 build findings

Established 2026-10-03 by building the backend in a Linux container, not by
reading documentation. None of these is visible on the development host, where
`ort` cannot build at all.

- **ONNX Runtime and libwebrtc cannot be statically linked together.** Both
  vendor protobuf and Abseil; `crates/config` depends on both
  `surveillance-detector` and `surveillance-livekit`, so every binary above it
  collides on `google::protobuf::internal::WireFormatLite::SkipField` and the
  Abseil surface. `ort` is built with `load-dynamic` as a result (ADR 015).
- **The runtime is a deployment artifact.** With `load-dynamic`, `ort` neither
  fetches nor links ONNX Runtime. `ORT_DYLIB_PATH` names it; the development
  image installs 1.28.3, the API level `ort` 2.0.0-rc.13 targets.
- **`ort` panics when the library will not load.** It calls
  `.expect("Failed to load ONNX Runtime dylib")` and again on the
  `OrtGetApiBase` lookup, so a missing runtime aborts the process rather than
  producing the classified failure `DI.C7` requires. This is a known breach of
  that contract, mitigated by shipping the library with the image rather than
  discovering it, and it is not fixable from this side of the boundary.
- `default-features = false` on `ort` silently disables `download-binaries`.
  The library crate still compiles, because an rlib does not link; every binary
  then fails with `undefined symbol: OrtGetApiBase`.
- ONNX Runtime's prebuilt binaries need **glibc 2.38 or newer**
  (`__isoc23_strtol`); Debian bookworm ships 2.36. The image is trixie, and
  carries `g++` because ONNX Runtime is C++.
- A bare `--features onnx-inference` is ambiguous in this virtual workspace and
  resolves into a build whose members cannot find their own dependencies. The
  gate uses `package/feature` syntax.
- The `ort` build fetches from a CDN and failed once with a DNS error before
  succeeding on retry: the second non-hermetic build dependency, after
  `webrtc-sys`.

### 3.3 Slice 7 real-model findings

Established 2026-10-03 by running `DI.R8` in the container against the official
`yolox_tiny.onnx` from the YOLOX 0.1.1rc0 release (20,219,662 bytes, SHA-256
`427cc366…cc70b0f7`, Apache-2.0), on a photograph of a street containing four
people: two unobstructed, one cropped by the right edge, one cropped by the left
edge to an arm and a leg.

- **The adapter detects.** Three detections at a 30% score threshold, each
  landing on one of the three people that are more than a limb, each box inside
  the frame, confidences 0.886, 0.876 and 0.862. The person cropped to an arm
  was not found. This is the first evidence that any of this detects anything.
- **The letterbox inverse is right on real output, not only on constructed
  points.** The source is 810x1080, so the content occupies 312x416 of the
  model's square with 52 columns of padding each side and none above or below.
  A box reaching x=1.000 of the source therefore proves the inverse removes the
  horizontal padding and rescales; a wrong inverse puts boxes consistently
  beside the people rather than on them, which is exactly what a confidence
  number cannot tell you.
- **The released artifact is a raw export.** The decoder's grid and stride
  reconstruction accepted its output shape, which it would not have done had the
  export carried decoding inside it.
- **Preprocessing belongs to an artifact, not to a family.** YOLOX 0.1.1rc0
  preprocessing flips to RGB, divides by 255 and applies ImageNet mean and
  standard deviation; from 0.2.0 it does none of that and feeds OpenCV's BGR
  unchanged. This artifact is published under the earlier tag and behaves as the
  later convention: unnormalized input produces the confident, correctly placed
  boxes above.
- **Channel order was measured, not assumed.** The control run with the channels
  reversed scores at or below BGR on every box (0.886/0.862/0.844 against
  0.886/0.875/0.861) with geometry unchanged to three decimal places. The
  adapter feeds BGR as a result. The margin is small, as it should be for a
  class identified by shape — which is why the wrong order would never have
  announced itself, and why a comment in the adapter claiming this had been
  established by running the artifact, written before any artifact had been run,
  was worth more than the line it occupied.
- **A passing `DI.R8` was not proof of detection.** Its per-box assertions hold
  vacuously over an empty set, so a model that found nothing passed. It now
  requires a stated minimum.
- Load took 1.73 s and inference 133 ms, on one CPU thread, in a container, in
  an **unoptimized debug build** with unoptimized preprocessing. The number is
  recorded because it was observed. It is not a capacity figure, it is not from
  the target hardware, and nothing may be divided by it.

What this does not establish remains the larger part: accuracy on anything but
this one frame, behaviour on the 114-grey padding this pipeline centres where
YOLOX's own preprocessing puts it bottom-right, and every item in section 4.3.

## 4. Requirements status and open inputs

### 4.1 Accepted requirements

- Detection is an independently degradable consumer. A failing detector must not
  stop recording or live viewing.
- Results are camera-scoped, epoch-scoped and timestamped.
- Sampling rate, confidence threshold and queue policy are configuration.
- No media payload reaches the domain event bus.
- Weights are a deployment artifact with a published SHA-256.

### 4.2 Reversible development defaults

Each of these is a documented default adopted to avoid blocking on an open
decision gate. None is a confirmed requirement, and each names what would
change it.

| Default | Value | Changed by |
|---|---|---|
| Model family | YOLOX | an accuracy objective YOLOX misses, or confirmed Intel hardware |
| Model size and input | YOLOX-Tiny at 416x416 | measurement on confirmed hardware showing headroom for 640 |
| Score threshold | 0.25 | the confidence semantics gate |
| NMS IoU | 0.45 | measured false-merge behaviour |
| Execution provider | CPU | confirmed accelerator |
| Intra-op / inter-op threads | 1 / 1 | measured per-camera cost on target |
| Gate cooldown | 5 s | observed vanish behaviour for stationary people |
| Gate heartbeat | 10 s | measured gate miss rate |

### 4.3 Open decision inputs

These remain open and must not be implied as settled by any code or document
produced under this specification.

- Server hardware, accelerator, operating system and deployment model.
- Acceptable detection FPS and confidence semantics.
- Whether tracking is required.
- The labeled dataset and accuracy criteria for evaluation.
- Cameras per unit and the supported camera count.

## 5. Scope

### 5.1 In scope

- An `OnnxPersonInference` implementation of `PersonInference`.
- A family-scoped `ModelOutputDecoder` with one YOLOX implementation.
- Model artifact loading, SHA-256 provenance, and publication on detector state.
- Execution-provider registration and warm-up validation.
- Fail-closed startup when detection is enabled.
- Runtime-sourced production reporting, replacing the build-fact descriptor.
- Motion gating with cooldown and heartbeat.
- Scaling to model input geometry in the detector's decode invocation.
- Configuration surface with declared apply-time semantics.

### 5.2 Out of scope

- Tracking, re-identification, occupancy, dwell time and heatmaps.
- ROI administration and rule evaluation.
- Accuracy evaluation against a labeled dataset, which has no dataset and no
  criteria.
- Capacity qualification and any supported-camera-count claim.
- TensorRT engine building, JetPack pinning and on-device engine caching.
- Camera-side gating.
- Quantization and person-only model retraining.

## 6. System boundaries

### 6.1 Process and trust boundary

The ONNX Runtime library executes in-process. Model weights are untrusted input
in the sense that a malformed or substituted file must produce a classified
startup failure rather than a panic or a silent degradation; they are trusted in
the sense that an operator who can write the configured path can choose what the
system detects, which is a deployment property recorded in ADR 015 rather than a
control this specification adds.

Frame pixels never leave the process. No frame content, and no path from the
configured model location, appears in logs, metrics, events or API responses.

### 6.2 Crate ownership

| Crate | Gains |
|---|---|
| `crates/detector` | `OnnxPersonInference`, `ModelOutputDecoder`, the YOLOX decoder, letterbox arithmetic, gate state |
| `crates/config` | the detection inference settings and their validation |
| `crates/events` | the model hash field on detector state |
| `apps/server` | wiring, fail-closed startup validation, runtime-sourced descriptor |

`ort` is a dependency of `crates/detector` only.

## 7. Core contracts

### DI.C1 — The substitution boundary is unchanged

`PersonInference` keeps its current shape. `infer` stays synchronous and
`&mut self`, so the existing `spawn_blocking` ownership transfer in
`detector.rs` continues to work without modification. No detection code outside
the adapter learns that inference became real.

`is_production_model()` keeps its `false` default. An implementation that does
not answer stays presumed non-production.

### DI.C2 — Output decoding is family-scoped and frame-free

A `ModelOutputDecoder` receives the output tensor, its shape, and the letterbox
parameters used to produce the input. It returns a `DetectionSet`. It receives
no `DecodedFrame`, no session and no configuration beyond thresholds, so it is
testable against a hand-built tensor with no model present.

Exactly one implementation ships: YOLOX. Adding a family is a new
implementation, not a modification of an existing one.

The YOLOX implementation must reconstruct the anchor grid itself, because the
published export is raw. For each stride in 8, 16, 32 over an input of side `S`,
the level contributes `(S / stride)^2` rows in that order, and the decode is:

```text
xy = (raw_xy + grid) * stride
wh = exp(raw_wh) * stride
score = objectness * class_score
```

Objectness and class columns are already sigmoid-activated and must not be
activated again. The decoder verifies that its reconstructed row count equals
the row count of the tensor it was given, and returns `MalformedOutput` when it
does not, because a mismatch means the model is not the family this decoder
understands.

### DI.C3 — The frame contract

The adapter reads `DecodedFrame` as packed RGB24 and indexes rows through
`stride`. It must not assume `stride == width * 3`. A frame whose `format` is
not RGB24, or whose `stride` is smaller than `width * 3`, is a
`MalformedOutput`-class failure of configuration rather than a cause for
best-effort interpretation.

The detector's decode invocation scales to the configured model input geometry
with aspect ratio preserved and centred padding, filling the pad with 114-grey.
The scale factor and pad offsets are therefore derivable from source and target
dimensions alone.

Pixel values are passed to the model unnormalized, as 0-255 floats in RGB
channel order, converted from HWC to CHW. This is the YOLOX convention and was
confirmed against the artifact in Slice 0; it is a property of the model family
and belongs to whichever `ModelOutputDecoder` family is selected.

### DI.C4 — The letterbox inverse is exact

For any source dimensions, target dimensions, and any box in target pixel space,
mapping back to normalized source coordinates must be the exact inverse of the
forward transform within floating-point tolerance, and must clamp to `[0, 1]`
rather than emitting out-of-range coordinates.

This is the highest-risk arithmetic in the design. A wrong inverse produces
boxes that look plausible and are consistently displaced, which no type system
catches and no fixture without ground truth reveals.

### DI.C5 — Model artifact and provenance

The adapter resolves a configured path, confirms it is a regular non-symlink
file, reads it, and computes SHA-256 over its bytes. When a hash pin is
configured and does not match, startup fails. The computed hash is published on
the detector-state event and is the `model_version` component of the reported
descriptor.

The repository contains no model artifact. The path is deployment
configuration.

### DI.C6 — Execution providers are proven by inference, not by query

Providers are registered in configured order, each with `.error_on_failure()`.
After session construction the adapter runs one warm-up inference on a
synthetic frame of the configured input geometry.

`get_available_providers()` is a compile-time list and will claim providers that
have no kernel image for the device, so availability is never treated as
evidence. Only the warm-up result is.

### DI.C7 — Startup is fail-closed

**Known breach, recorded rather than hidden:** when the ONNX Runtime shared
library itself cannot be loaded, `ort` panics inside its own initialisation
before any code here runs. That path aborts rather than refusing. Everything
below concerns failures this code can observe.

With detection enabled, each of the following refuses server start with a
classified, operator-actionable error: the configured path missing, unreadable,
a symlink, or not a regular file; a hash pin mismatch; session construction
failure; provider registration failure; warm-up failure.

There is no fallback to the fixture and no silent disabling of detection. A
build that cannot detect must not start as one that claims it can.

### DI.C8 — The production claim is runtime-sourced

`InferenceDescriptor` is derived from the live inference object: model name from
configuration, model version from the SHA-256, and `is_production` true only
when a session is loaded and its warm-up has passed.

`compiled_backend()`'s current meaning — a pure build fact — does not survive
this change and must be replaced rather than reinterpreted. The replacement
invariant is that the descriptor the API serves is derived from the object the
pipeline actually constructed, and that a build carrying the ONNX backend with
no weights reports `is_production_model: false`.

### DI.C9 — Gating has a floor and a ceiling

The gate decides whether to run inference on a sampled frame. It runs inference
when inter-frame difference exceeds a configured threshold, **or** the last
detection is within the configured cooldown, **or** the configured heartbeat
interval has elapsed since the last inference.

The cooldown exists because a person who stops moving would otherwise
disappear. The heartbeat exists to bound the damage from a mistuned threshold.
Gate state is per camera and resets on `connection_epoch` change.

Gating reduces inference only. Decode is unaffected, because H.264
inter-frames cannot be decoded in isolation.

### DI.C10 — Resources are bounded and charged

Input tensor allocation is a function of configured input geometry and is
allocated once per session, not per frame. ONNX Runtime intra-op and inter-op
thread counts are explicit configuration. Decoded output is truncated to
`MAX_DETECTIONS_PER_EVENT` before any `DetectionSet` is constructed.

The per-camera memory budget check must account for the model input tensor and
the gate's retained reference frame.

### DI.C11 — Failures are classified, not retried blindly

`InferenceFailureKind` is reused unchanged.

| Condition | Kind |
|---|---|
| Session run error | `Retryable` |
| Output shape or layout not what the decoder expects | `MalformedOutput` |
| Execution provider no longer usable | `Unavailable` |
| Allocation failure | `ResourceExhausted` |
| Warm-up or run exceeding a configured bound | `Timeout` |

Classification is structural. It is never derived from the text of a runtime
error message.

### DI.C12 — Configuration application semantics

Every setting declares when it takes effect.

| Setting | Applies |
|---|---|
| `model_path` | after restart |
| `model_sha256` | after restart |
| `input_width`, `input_height` | after restart |
| `execution_providers` | after restart |
| `intra_op_threads`, `inter_op_threads` | after restart |
| `score_threshold` | immediately, read per inference |
| `nms_iou` | immediately, read per inference |
| `motion_threshold` | immediately |
| `gate_cooldown` | immediately |
| `gate_heartbeat` | immediately |

Restart-scoped settings are validated at startup and rejected as a group. A
change to one of them through the settings API is accepted into persistence and
reported as pending restart; it must not partially apply.

### DI.C13 — No claim beyond the evidence

No document, log line, metric name, API field or commit message produced under
this specification may state or imply a detection accuracy level, a supported
camera count, or a latency objective. Phase 6's accuracy gate and Phase 9's
capacity gate are unchanged by this work.

## 8. Observability

Added counters, all fixed-cardinality, none labelled by camera, recording,
model path or threshold value:

- `detection_inference_gate_skipped_total`
- `detection_inference_heartbeat_total`
- `detection_inference_preprocess_micros`
- `detection_inference_decode_micros`
- existing `inference_attempts`, `inferred`, `inference_failed` and
  `inference_duration_micros` are retained unchanged.

Detector state gains the model name, the model SHA-256, the resolved execution
provider, and whether the backend is production.

## 9. Failure matrix

| Failure | Detected by | Behaviour | Recording and live |
|---|---|---|---|
| Model file absent | startup validation | refuse start | n/a |
| Model hash mismatch | startup validation | refuse start | n/a |
| Provider registration fails | `.error_on_failure()` | refuse start | n/a |
| Warm-up produces no output | warm-up inference | refuse start | n/a |
| Session run fails mid-operation | `infer` | `Retryable`, backoff, camera continues | unaffected |
| Output layout unexpected | decoder | `MalformedOutput`, terminal for the detector | unaffected |
| Allocation failure | `infer` | `ResourceExhausted` | unaffected |
| Inference slower than sampling | existing overload policy | freshest frame preferred, drops counted | unaffected |
| Shutdown during inference | existing `spawn_blocking` join | bounded join, already proven | unaffected |

## 10. Testing and evidence requirements

### DI.R1 — Letterbox inverse round-trip

Property test over source dimensions, target dimensions and box coordinates:
forward then inverse returns the original within tolerance, output is clamped to
`[0, 1]`, and a degenerate source dimension is rejected rather than producing a
division by zero. No model, no ONNX Runtime, no camera.

### DI.R2 — Output decode against constructed tensors

Hand-built YOLOX output tensors assert: a single high-score person decodes to
the expected normalized box; a sub-threshold detection is dropped; two
overlapping boxes collapse under NMS; a non-person class is filtered; more than
`MAX_DETECTIONS_PER_EVENT` results are truncated; a tensor of unexpected shape
yields `MalformedOutput` rather than a panic.

### DI.R3 — Gate state machine

Unit tests assert: motion above threshold infers; motion below threshold and
outside cooldown and heartbeat skips; a frame within cooldown of a detection
infers despite no motion; heartbeat fires after the configured interval of
skips; an epoch change resets state.

### DI.R4 — Artifact and provenance

Tests assert: the computed SHA-256 matches a known fixture file; a symlinked or
non-regular path is refused; a configured pin mismatch is refused; the hash
appears on detector state.

### DI.R5 — Provider and warm-up failure

Tests assert that a session which constructs but produces no warm-up output
yields a classified startup failure, and that availability reported by the
runtime is not sufficient for readiness.

### DI.R6 — Fail-closed startup

Tests assert that detection enabled with no `model_path`, with a missing file,
and with a mismatched hash each refuse server start, and that none of those
paths yields a running server reporting `is_production_model: true`.

### DI.R7 — The honesty descriptor

Tests assert that the descriptor served by the API is derived from the live
inference object; that a fixture backend reports `is_production_model: false`
with `results_are_fixture: true`; and that a build carrying the ONNX backend
with no weights does not report production. The existing guard test is replaced
by this requirement, not deleted.

### DI.R8 — Real model through the full adapter

Ignored by default, matching the repository convention for tests needing
external artifacts. With a model file and a frame present, the frame produces a
decodable `DetectionSet`, the reported descriptor is production, every box lies
inside the frame, and the count meets a stated minimum — without which the test
passes on a model that detects nothing, because its per-box assertions hold
vacuously over an empty set. Inference duration is recorded.

`make container-verify-model` runs it. Executed 2026-10-03; section 3.3 carries
the result. This test is evidence that the adapter works end to end. It is not
evidence of accuracy: one frame is an anecdote.

### DI.R9 — Isolation parity

The existing Phase 6 tests pass unchanged: detector saturation remains bounded
and prefers recent frames, a crashed or unavailable detector does not stop
recording or live viewing, and shutdown during blocking inference remains
bounded and joined.

### DI.R10 — Full repository gate

`make check` passes: shell syntax, both Python suites, the Node UI tests,
`cargo fmt --all --check`, Clippy over all targets and features with warnings
denied, all workspace tests, the release build, `cargo deny check` and
`cargo audit`.

## 11. Evidence package

Recorded in a dated `context/` snapshot at completion:

- `make check` output with test counts.
- The `ort` version pinned and the ONNX Runtime version it resolved.
- The model file name, its SHA-256, and its source URL.
- The resolved execution provider from the warm-up.
- Measured preprocess, inference and decode durations on the development
  machine, labelled explicitly as x86_64 CPU figures that do not transfer to
  the production target.
- The `DI.R8` output: the fixture frame used and the detections produced.
- A restatement of what remains unproven: accuracy, capacity, and every item in
  section 4.3.

## 12. Entry gate

- ADR 015 accepted.
- A model artifact obtainable and its licence recorded.
- A development platform on which `ort` resolves a prebuilt ONNX Runtime.
  Slice 0 established that `x86_64-apple-darwin` is not such a platform. This
  gate is open, and it gates slice 3 onward only.

## 13. Exit gate

- `DI.C1` through `DI.C13` hold.
- `DI.R1` through `DI.R7`, `DI.R9` and `DI.R10` pass in CI.
- `DI.R8` has been run once with its output recorded. Met: section 3.3.
- `/api` reports production only for a build with loaded, warmed weights.
- `ROADMAP.md` Phase 6 reflects the new state, with the accuracy item still
  open.
- No document or interface claims accuracy or capacity.

## 14. Risks and mitigations

| Risk | Mitigation |
|---|---|
| `ort` has no stable release and a churning API | exact pin; `PersonInference` as the recorded reversal seam; upgrade runs the gate |
| The 2021-vintage YOLOX ONNX export may not load under ONNX Runtime 1.28 | Slice 0 probe answers this before any production code is written |
| YOLOX preprocessing convention is documented inconsistently | determined from the actual file in Slice 0, not from documentation |
| A wrong letterbox inverse is invisible without ground truth | `DI.R1` property test plus `DI.R8` visual evidence |
| Default ONNX Runtime thread pools oversubscribe a small target | explicit configuration defaulting to 1 |
| Coupling decode geometry to model input blocks a future full-resolution consumer | recorded in ADR 015 consequences; a second decode is the escape |
| Shipping a real model invites accuracy claims | `DI.C13`; the runtime-sourced descriptor; Phase 6 item stays open |
| A missing ONNX Runtime library panics inside `ort` rather than refusing | the library ships with the image and its path is fixed, not discovered; recorded in ADR 015 as a breach of `DI.C7` that cannot be closed from this side |
| ONNX Runtime and libwebrtc collide when statically linked | `load-dynamic`, which is also what an on-device Jetson runtime requires |
| `ort` has no prebuilt for the development machine's `x86_64-apple-darwin` | adding `ort` to the workspace is gated on a development platform that can build it; slices 1 and 2 need no runtime |
| CI is `aarch64-apple-darwin` while development is `x86_64-apple-darwin`, so `ort` would build in CI and fail locally | recorded here before `ort` is added; the gate must fail for a developer the same way it fails in CI |

## 15. Reversal criteria

Removing this adapter returns the system to a fixture backend by configuration
alone: detection disabled, or a build without the ONNX backend. The pipeline,
the event contracts and the API fields are unchanged by its presence or absence,
which is the property that makes the runtime and model choices reversible.
