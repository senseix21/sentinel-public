# Detection model selection — design paused pending hardware

Design was carried through sections 1-3 of 5 and is paused, not abandoned.
Section 4 (benchmark harness and sizing rule) and section 5 are unwritten. No
spec file exists yet and no code was written.

## Accepted decisions

- In-process ONNX Runtime via the `ort` crate, behind the existing
  `PersonInference` trait. Rejected an out-of-process sidecar (per-frame IPC on
  the hot path, second artifact per hardware target) and pure-Rust runtimes
  (hand-written pre/post-processing, weakest accelerator coverage).
- `ort` pinned to an exact version. It has **no stable release**: latest is
  `2.0.0-rc.13` (2026-07-28), `max_stable_version` is null, and every 1.x
  release is yanked. The API has churned across rc.4/rc.6/rc.10/rc.13.
  `PersonInference` is the documented reversal seam.
- Motion-gated inference with a cooldown window and a periodic heartbeat. The
  cooldown exists because a person who stops moving would otherwise vanish; the
  heartbeat bounds the damage from a mistuned gate.
- Model weights are a deployment artifact, not a repo file. The detector depends
  only on a readable file at a configured path, with its SHA-256 published on the
  detector-state event so version skew is detectable.

## Verified facts

- Motion gating saves inference, not decode. H.264 inter-frames cannot be
  decoded in isolation, so the decoder runs continuously.
- YOLO11 ONNX output is `float32[1,84,8400]`; boxes are centre-x/centre-y/width/
  height in pixels of the 640x640 input; scores are already sigmoid-activated;
  NMS is not included in the export.
- `MAX_DETECTIONS_PER_EVENT` is 64, so post-processing must truncate.
- `ort` prebuilt binaries cover CUDA/TensorRT for Windows and Linux x64 only.
  Jetson is aarch64 and requires a source build or a community aarch64 wheel,
  loaded via `ORT_STRATEGY=load-dynamic` and `ORT_DYLIB_PATH`.
- `libonnxruntime_providers_tensorrt.so` and `libonnxruntime_providers_shared.so`
  must sit alongside `libonnxruntime.so`, or registration fails while
  availability still reports true.
- `get_available_providers()` is a compile-time list and will claim providers
  that have no kernel image for the device. Only a real inference run proves an
  execution provider works, so startup validation must run a warm-up inference
  and register providers with `.error_on_failure()`.

## Working assumptions

- YOLO26 rather than YOLO11. It is the current Ultralytics generation, is
  AGPL-3.0 (acceptable per the licensing decision), is benchmarked on Jetson
  Orin with TensorRT, and is NMS-free, which removes hand-written NMS and one
  coordinate transform.
- NVIDIA Jetson is the production target; Linux is the development target.
  Dev and production differ in both CPU architecture and accelerator, which is
  what makes the execution-provider abstraction necessary rather than
  speculative.
- Deployment is L4T containers or systemd on-device. Weights ship as ONNX; the
  TensorRT engine is built on-device and cached as derived local state, never
  shipped and never shared between devices or JetPack versions. A runtime
  artifact-store fetch is ruled out.

## Open questions — blocked on Erik, who asked for time on 2026-08-14

1. Which Jetson module and RAM variant. Roughly a 7x compute range across the
   family, and NVDEC engine count differs, which caps simultaneous decode
   independently of GPU compute.
2. Whether hardware is already purchased or still open. If open, the benchmark
   harness becomes a purchasing input rather than a validation step.
3. Cameras per unit, and whether it is one unit per site.
4. Real camera resolution, frame rate, and codec. H.265 or higher resolution
   would materially change the decode budget.
5. JetPack version. The ONNX Runtime build is pinned to it and is not portable.
6. Storage per unit and retention period.
7. Site operator and connectivity, which determines software and model delivery.

No capacity or accuracy claim is supported until 1, 3, and 4 are answered and
measured. Published Orin latency figures are not evidence for this workload.
