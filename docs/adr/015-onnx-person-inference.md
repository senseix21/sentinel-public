# ADR 015: In-Process ONNX Person Inference

Status: accepted; amended 2026-10-02 with the build and platform strategy

Date: 2026-10-02

## Context

`crates/detector` carries a complete detection pipeline: decode-once frame
supply, timestamp-based sampling, a freshness-preferring drop policy under
overload, bounded memory, normalized bounding-box and confidence types, the
domain event bus, and failure isolation proven by Phase 6. Inference already
runs on `tokio::task::spawn_blocking` with the inference object moved across the
thread boundary and its duration counted.

What does not exist is a model. The only implementation of `PersonInference` is
`DeterministicFakeInference`, which emits one person when the first RGB byte is
odd. `is_production_model()` defaults to `false`, and `/api` reports
`results_are_fixture: true`. Nothing in the configuration can even name a model
file.

`AGENTS.md` lists the detection model, hardware, sample FPS and confidence
behavior as an explicit decision gate, and requires either the confirmed
requirement or a reversible, explicitly documented default that does not
masquerade as one. The hardware half of that gate has been open since
2026-08-14. This decision supplies the reversible default for the rest so that
work is not blocked on it.

## Options considered

1. **In-process ONNX Runtime through the `ort` crate.** One artifact, no
   per-frame IPC, accelerator coverage through execution providers, and a model
   format every candidate exports to. Chosen.
2. **Out-of-process inference sidecar.** Rejected: puts IPC on the per-frame hot
   path and requires a second deployable artifact per hardware target, for an
   isolation benefit the existing `spawn_blocking` boundary and
   `TerminalInference` classification already provide.
3. **Pure-Rust runtimes (`tract`, `candle`).** Rejected: hand-written
   pre/post-processing for every model family and the weakest accelerator
   coverage of the options, which is the opposite of what a Jetson target needs.
4. **OpenVINO with a person-only model.** Attractive on paper:
   `person-detection-0200` is 0.786 GFLOPs, 1.8 MParams, person-only, with NMS
   inside the graph. Rejected because `model.yml` distributes framework `dldt`
   with only `.xml` and `.bin` in FP32/FP16/FP16-INT8 — no ONNX, and OpenVINO
   converts to IR rather than from it — and because the OpenVINO runtime has no
   CUDA or TensorRT path. Revisit only if the target turns out to be Intel.
5. **Platform-native vision APIs.** Apple's Vision framework ships a trained
   human detector on the development machine at no cost. Rejected as a dead end:
   macOS-only, so it cannot be the production path on aarch64 Linux, and it
   proves almost nothing the existing fixture does not.

## Decision

Implement `PersonInference` with in-process ONNX Runtime through `ort`, pinned
to an exact version. The crate has no stable release: `2.0.0-rc.13` is the
newest, `max_stable_version` is null, and every 1.x release is yanked
(verified 2026-10-02). `PersonInference` is the documented reversal seam.

Introduce a second, narrower seam beneath it. A `ModelOutputDecoder` converts
one model family's output tensor into normalized detections and nothing else:
class filtering, score threshold, non-maximum suppression where the family
needs it, the inverse letterbox, and truncation to
`MAX_DETECTIONS_PER_EVENT`. This is not speculative abstraction. The output
layout is the only part of the adapter that changes with the model, it is the
highest-risk code in the design, and isolating it is what lets the riskiest
arithmetic be tested against a hand-built tensor with no model, no ONNX Runtime
and no camera present.

Adopt **YOLOX-Tiny** as the reversible default model: Apache-2.0, a
pre-exported 20.2 MB ONNX asset, 32.8 COCO AP at 416x416 for 6.45 GFLOPs, a
documented TensorRT path, and direct `xywh` output rather than the
distributional regression that NanoDet-Plus requires. Apache-2.0 removes the
AGPL-3.0 obligation that the Ultralytics families carry, so no commercial
licence is needed and no copyleft attaches to a deployed system.

Model weights are a deployment artifact, never a repository file. The detector
reads a configured path, hashes it with SHA-256, compares against an optional
configured pin, and publishes the hash on the detector-state event so field
version skew is detectable.

Startup is fail-closed. Detection enabled with weights missing, unreadable,
hash-mismatched, or failing a warm-up inference refuses server start. There is
no silent fixture fallback and no silent disabling of detection.

`is_production_model` becomes a runtime fact rather than a build fact. A build
can carry the ONNX backend and still run with no weights, so the descriptor
reported by the API is derived from the live session and is true only when a
session is loaded and its warm-up inference has passed.

Execution providers are registered in configured order with
`.error_on_failure()`, and a warm-up inference runs before readiness is
reported. `get_available_providers()` is a compile-time list that will claim
providers with no kernel image for the device, so only a real inference proves
one works. ONNX Runtime intra-op and inter-op thread counts are explicit
configuration defaulting to 1, because ten cameras each holding a session with
machine-sized default pools would contend through one blocking pool.

## Amendment: the runtime is optional and its gate is containerised

The slice 0 probe established that `ort` publishes no prebuilt binary for
`x86_64-apple-darwin`, that Microsoft ships ONNX Runtime for `osx-arm64` only
in 1.28 through 1.30, and that PyPI's macOS x86_64 wheels stop at 1.23.2. The
development machine is `x86_64-apple-darwin`; continuous integration runs
`aarch64-apple-darwin`. An unconditional dependency would therefore produce a
tree that builds in one place and not the other, and would stop the documented
gate from running on the developer's own machine.

`ort` is consequently an **optional dependency behind the `onnx-inference`
feature**, default off, on `crates/detector` and re-exported by `apps/server`.
The workspace builds, tests and ships on any platform with the fixture backend.
This makes `PersonInference` a substitution boundary in the build rather than
only in prose: the ONNX backend can be absent entirely and nothing else changes.

The gate splits to match. `make verify` runs the deterministic half on default
features and is what every platform and pull-request CI runs. `make verify-onnx`
runs Clippy and the tests with `onnx-inference` enabled and requires a platform
`ort` supports. `--all-features` leaves the gate, because it would enable the
optional backend everywhere and defeat the split; it was a no-op until now, as
no crate declared a feature.

A development container supplies the supported platform. It is Debian rather
than Alpine because ONNX Runtime's prebuilts are glibc builds, and `linux/amd64`
on an Intel development host is native rather than emulated. The container runs
the same `make` targets, so a container run is the same gate rather than a
subset of it. `make container-verify` and `make container-verify-onnx` are the
entry points.

The real-model evidence of slice 7 is produced in that container, because the
development machine cannot execute it at all.

## Amendment: the runtime is loaded, not linked

Building the backend established a constraint that changes how ONNX Runtime is
obtained, and it is not negotiable.

**ONNX Runtime and libwebrtc cannot be statically linked into the same binary.**
Both vendor their own protobuf and Abseil. `crates/config` depends on both
`surveillance-detector` and `surveillance-livekit`, so every binary above it —
`surveillance-database`, `apps/server`, and their test binaries — would link two
copies and fail with `duplicate symbol:
google::protobuf::internal::WireFormatLite::SkipField` and the whole Abseil
surface behind it.

`ort` is therefore built with `load-dynamic`: the runtime is opened at runtime
rather than linked, keeping its symbols in their own object. This converges with
the deployment plan rather than fighting it, because a Jetson build of ONNX
Runtime is produced on-device and supplied separately in any case.

Two consequences follow, and both are real.

The library becomes a deployment artifact alongside the model. `ORT_DYLIB_PATH`
names it; the development image installs ONNX Runtime 1.28.3, matching the API
level `ort` 2.0.0-rc.13 targets, and sets that variable.

**`ort` panics when the library cannot be loaded.** It calls
`.expect("Failed to load ONNX Runtime dylib")`, and again on the
`OrtGetApiBase` symbol lookup. A missing or unloadable runtime is therefore a
process abort rather than the classified startup failure this specification
requires everywhere else, and no amount of care on this side of the boundary
converts it into one. Treating the library as part of the image rather than as
something discovered at startup is the mitigation; it is a mitigation, not a
fix, and it is recorded here because a deployment that gets this wrong gets a
crash instead of a diagnosis.

## Consequences

- Detection becomes a real capability rather than a wired seam, and the API
  stops reporting `results_are_fixture` for a build that has weights.
- A dependency with no stable release sits on the detection path. The API has
  churned across rc.4, rc.6, rc.10 and rc.13, so an upgrade is a deliberate
  change that runs the gate, not a routine bump.
- The accuracy of detection remains unproven. Phase 6's remaining open item is
  evaluation on an agreed labeled dataset against agreed criteria, and neither
  exists. COCO AP is an 80-class mean measured on web photographs, not a
  statement about these cameras, mounting heights or lighting.
- No capacity claim follows from this decision. The inference arithmetic is
  modest — ten cameras at five sampled frames per second is fifty inferences
  per second, or 323 GFLOP/s at YOLOX-Tiny — but a supported camera count
  requires a documented workload and repeatable measurement on confirmed
  hardware.
- The motion gate sits after decode and therefore cannot reduce decode cost.
  H.264 inter-frames cannot be decoded in isolation, so the decoder runs
  continuously regardless of gate duty cycle. Moving the gate upstream of
  decode requires control of the camera itself; ESP32-P4 is a viable platform
  for that if the product ever owns its cameras, and is recorded here as an
  option rather than a plan.
- Development on `x86_64-apple-darwin` continues without the ONNX backend, and
  with it only through the container. An engineer who never enables the feature
  is unaffected by `ort`'s instability, which is the point of making it
  optional.
- Two gate commands exist where there was one, and a change that compiles under
  default features can still fail under `onnx-inference`. Only the containerised
  job proves the second, so a green `make verify` is no longer sufficient
  evidence for a change that touches the backend.
- The ONNX Runtime shared library must be present wherever the backend runs, and
  its absence is a panic rather than a refusal.
- The build is not hermetic in a second place. `ort` fetches from a CDN during
  the build and failed once with a DNS error before succeeding on retry, which
  is the same class of fragility already recorded for `webrtc-sys`.
- Scaling to model input geometry inside the detector's own FFmpeg invocation
  couples decode configuration to the model's input size. A future consumer
  needing full-resolution frames needs its own decode or a configuration
  change.

## Reversal criteria

Replace the runtime behind `PersonInference` if `ort` remains without a stable
release when a Jetson or other aarch64 target is confirmed and its prebuilt
binaries do not cover it, or if an upgrade breaks the API faster than it can be
absorbed. The trait boundary is unchanged by such a replacement.

Replace the model behind `ModelOutputDecoder` if the accuracy evaluation, once
its dataset and criteria exist, shows YOLOX-Tiny below the accepted objective,
or if measurement on confirmed hardware shows its cost unaffordable. Swapping
families is one new decoder implementation and a configuration change.

Reconsider this decision in full if the confirmed hardware is Intel rather than
NVIDIA, in which case OpenVINO with a person-only model becomes materially
cheaper than ONNX with a COCO model and the runtime choice should be re-argued.
