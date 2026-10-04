# Repository Status — 2026-10-04 20:21 +06

## Verified facts

- Branch: `feature/detection-inference`, from `main` at `c88aed3` (pull request
  #10 merged 2026-10-02).
- Detection inference slices 0 through 8 of `docs/detection-inference/PLAN.md`
  are implemented, and `DI.R8` and `DI.R10` have both been run.
- A real inference backend exists: ONNX Runtime through `ort`, behind the
  optional `onnx-inference` feature, with a YOLOX output decoder. A default
  build still runs `DeterministicFakeInference`.
- The production claim is taken from the live backend. `compiled_backend()` is
  gone; `GET /api/system` aggregates the descriptors the running pipelines
  reported. No detector running reports fixture, not production; any fixture
  among them reports the whole deployment as fixture.
- The detector's decoder scales to the model's input geometry with 114-grey
  padding at the origin, so a 1080p camera feeding a 416x416 model holds roughly
  a fifth of the bytes it used to.
- A motion gate admits on motion, within a cooldown of a recent detection, or on
  a heartbeat, and resets on a connection epoch change.

## Evidence package

The gate, 2026-10-04:

- `make verify` on the development host: **536 passed, 0 failed**.
- `make container-verify-onnx`, Linux with `ort` live: **543 passed, 0 failed**.
- `make audit`: **exit 0**. Duplicate-crate and unencountered-license warnings
  only; no advisory and no licence refusal.

The artifact:

- `yolox_tiny.onnx`, 20,219,662 bytes, SHA-256
  `427cc366d34e27ff7a03e2899b5e3671425c262ea2291f88bb942bc1cc70b0f7`,
  Apache-2.0, from the YOLOX 0.1.1rc0 release:
  `https://github.com/Megvii-BaseDetection/YOLOX/releases/download/0.1.1rc0/yolox_tiny.onnx`.
  Not committed, and not ours to commit.
- `ort` is pinned at `=2.0.0-rc.13`. It resolves ONNX Runtime 1.28.3, which the
  development image installs at `/opt/onnxruntime` and names through
  `ORT_DYLIB_PATH`.
- Execution provider: CPU, configured and proven by a warm-up inference rather
  than by querying availability, which `DI.C6` forbids treating as evidence.

`DI.R8`, one frame end to end (`make container-verify-model`):

- A street photograph, 810x1080, holding four people: two unobstructed, one
  cropped by the right edge, one cropped by the left edge to an arm and a leg.
- Three detections at 0.878, 0.876 and 0.854, each on one of the three people
  that are more than a limb, each box inside the frame. The limb was not found.
- Load 1.73 s, inference 133 ms. **One CPU thread, in a container, in an
  unoptimized debug build with unoptimized preprocessing, on a development host
  that is not the target.** Preprocess and output-decode time are counted
  separately by the detector but were not broken out in this run; the figure
  above is whole-inference wall time. Nothing may be divided by it.

`DI.R10`, sixty frames with ground truth (`make container-verify-accuracy`):

- COCO val2017, person category, crowd-flagged images excluded, 179 people.
- **Recall 73.7% (132/179), precision 89.7% (132/147)** at IoU 0.5 and a 30%
  score threshold. By subject size: large 95.3%, medium 70.2%, small 36.9%.
- Roughly one person in four is not reported. This is a public benchmark of
  general photographs and not this product's cameras.

## Platform and build facts

Established by running things, not by reading about them.

- `ort` publishes no prebuilt runtime for `x86_64-apple-darwin`, the development
  host. Continuous integration runs `aarch64-apple-darwin`, a third
  architecture.
- **ONNX Runtime and libwebrtc cannot be statically linked into one binary.**
  Both vendor protobuf and Abseil, and `crates/config` depends on both
  `surveillance-detector` and `surveillance-livekit`, so every binary above it
  collided. `ort` runs with `load-dynamic`; the runtime is a deployment artifact.
- **`ort` panics when that library will not load**, through
  `.expect("Failed to load ONNX Runtime dylib")`. A recorded breach of the
  specification's own fail-closed contract that cannot be closed from this side.
- The prebuilt runtime needs glibc 2.38 or newer; the image is Debian trixie
  with `g++`, because ONNX Runtime is C++.
- **Preprocessing belongs to an artifact, not to a model family.** YOLOX 0.1.1rc0
  preprocessing flips to RGB, divides by 255 and applies ImageNet mean and
  standard deviation; from 0.2.0 it does none of that and feeds OpenCV's BGR
  unchanged. The released file is published under the earlier tag and behaves as
  the later convention. Both channel order and padding placement were settled by
  measurement, not by reading.
- **macOS does not refuse to execute a file held open for writing; Linux does.**
  Measured, and it is why one test is Linux-only.

## Still unverified

- Detection accuracy on anything that represents this product. Phase 6's
  remaining item needs an agreed labeled dataset and criteria; neither exists.
- Capacity. No measurement on target hardware, which is itself unconfirmed since
  2026-08-14.
- The gate's thresholds are reversible defaults. No duty cycle has been measured
  on real footage.
- A real LiveKit room re-publishing a track after the publication loop recovers.
  The recovery itself is proven; `connect` and `disconnect` are faked because a
  `Session` holds a connected `Room` and no server runs in the gate.
- Phase 8's thirteen alert rules, which have never been executed because
  `promtool` is not installed here.
- Linux V4L2 and Windows DirectShow capture, and every open input in
  specification section 4.3: server hardware and accelerator, acceptable
  detection FPS and confidence semantics, whether tracking is required, the
  labeled dataset and accuracy criteria, and cameras per unit.
