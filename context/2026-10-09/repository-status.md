# Repository Status — 2026-10-09

Development launcher verification on Apple Silicon macOS; base HEAD `015bc4e`.
Changes remain uncommitted in Makefile, README, local-webcam-stack.sh,
onnx-defaults.sh, and test_onnx_defaults.py.

## Verified

- `make run` defaults to real ONNX inference, caches checksum-verified
  YOLOX-Tiny and ONNX Runtime 1.28.3 under `.local/onnx`, and invokes the
  browser opener after the proxy is ready. Explicit detection/browser opt-outs
  and custom model/runtime overrides remain available.
- First-use model and runtime downloads succeeded; runtime loaded through ctypes.
- `cargo build -p surveillance-server --features onnx-inference` passed.
- Full webcam stack reached ONLINE with PostgreSQL, LiveKit and proxy.
- Authenticated dashboard returned 200. `/api/system` reported `yolox_tiny`
  and `results_are_fixture: false`.
- Camera health showed connected, zero reconnects, 62 completed segments,
  zero failed segments, 9,247 inference passes and 10 detection queue drops.
  Metrics reported 7,459 produced person detections. These are execution
  counters, not accuracy evidence.
- 18 Python regression tests and 32 dashboard tests passed. Shell syntax and
  `git diff --check` passed. Full workspace gate was not rerun for shell/docs changes.

## Bugs found and corrected

- macOS Bash 3.2 with nounset refused the empty development-profile array.
  Conditional array expansion fixes both profile and feature argument lists;
  a regression runs the launcher's actual build command with empty/nonempty arrays.
- This webcam rejected 15 FPS at 1280x720; direct 30 FPS capture succeeded.
  The development defaults now use 30 FPS; explicit VIDEO_FPS overrides remain.

Installed Homebrew FFmpeg 9.0.2, PostgreSQL 17.11 and LiveKit 1.13.9 with
approval to perform the smoke run. No Homebrew background services were enabled.
The verified stack was left running for the user.

## Limits

Browser opener returned without a warning; rendered overlays and actual
viewer decoding were not inspected in this verification. No new production
accuracy or camera capacity claim follows from this one-webcam run.
