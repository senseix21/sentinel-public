# Repository Status — 2026-10-03 22:37 +06

## Verified facts

- Branch: `feature/detection-inference`, from `main` at `c88aed3` (pull request
  #10 merged 2026-10-02).
- Detection inference slices 0 through 8 of `docs/detection-inference/PLAN.md`
  are implemented. Verification figures are in this snapshot's closing section
  and were produced by the final pass, not by the slices individually.
- A real inference backend exists: ONNX Runtime through `ort`, behind the
  optional `onnx-inference` feature, with a YOLOX output decoder. A default
  build still runs `DeterministicFakeInference`.
- The production claim is taken from the live backend. `compiled_backend()` is
  gone; `GET /api/system` aggregates the descriptors the running pipelines
  reported. No detector running reports neither production nor fixture; any
  fixture among them reports the whole deployment as fixture.
- The detector's decoder scales to the model's input geometry with centred
  114-grey padding, so a 1080p camera feeding a 416x416 model holds roughly a
  fifth of the bytes it used to.
- A motion gate admits on motion, within a cooldown of a recent detection, or on
  a heartbeat, and resets on a connection epoch change.

## Platform and build facts

Established by running things, not by reading about them.

- `ort` publishes no prebuilt runtime for `x86_64-apple-darwin`, the development
  host. Microsoft ships ONNX Runtime for `osx-arm64` only in 1.28 through 1.30,
  and PyPI's macOS x86_64 wheels stop at 1.23.2. Continuous integration runs
  `aarch64-apple-darwin`, a third architecture.
- **ONNX Runtime and libwebrtc cannot be statically linked into one binary.**
  Both vendor protobuf and Abseil, and `crates/config` depends on both
  `surveillance-detector` and `surveillance-livekit`, so every binary above it
  collided. `ort` runs with `load-dynamic`; the runtime is a deployment artifact
  named by `ORT_DYLIB_PATH`, and the development image installs 1.28.3.
- **`ort` panics when that library will not load**, through
  `.expect("Failed to load ONNX Runtime dylib")`. A missing runtime aborts the
  process rather than refusing cleanly. This is a recorded breach of the
  specification's own fail-closed contract and cannot be closed from this side.
- The prebuilt runtime needs glibc 2.38 or newer; the image is Debian trixie
  with `g++`, because ONNX Runtime is C++.
- `ort` fetches from a CDN during the build and failed once with a DNS error
  before succeeding on retry: the second non-hermetic build dependency, after
  `webrtc-sys`.

## Still unverified

- Detection accuracy. Phase 6's remaining item needs a labeled dataset and
  agreed criteria; neither exists. A loaded model says nothing about what it
  detects.
- Capacity. No measurement on target hardware, which is itself unconfirmed since
  2026-08-14.
- `DI.R8`, the real model end to end, is written and ignored by default. Whether
  it has been executed is recorded in the closing section.
- The gate's thresholds are reversible defaults, not tuned values. No duty cycle
  has been measured on real footage.
- Linux V4L2 and Windows DirectShow capture, and every open input in
  specification section 4.3.
