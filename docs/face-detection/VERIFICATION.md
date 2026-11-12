# Face-only detection verification

Date: 2026-10-09. Host: Apple M5 Pro, arm64 macOS. Local-development
qualification only; no production accuracy or 10/20-camera capacity claim.

## Implemented contract

`make run` selects real YuNet face inference at 70%. Person mode remains an
explicit rollback. The existing detection owner retains its decoder, sampler,
bounded queue and blocking inference boundary; no camera connection is added.
Faces have separate event and wire types, no person tracking/coasting or identity
labels, a maximum of 64 results and a 16 KiB transport limit. Empty observations
clear boxes. Latest-packet overlays expire after a configurable three seconds.
Recording stores original media without drawing overlays.

Artifact: OpenCV Zoo YuNet 2023mar, fixed float32 BGR 1×3×640×640 input,
12 validated outputs, 8,400 candidates, NMS IoU 45%.
SHA-256: `8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4`.
License and source links: [ADR 016](../adr/016-face-only-live-detection.md).

## Automated checks

| Check | Observed result |
|---|---|
| `cargo fmt --all --check`, `git diff --check` | Passed |
| Workspace all-target/all-feature Clippy, `-D warnings` | Passed |
| Workspace all-feature tests | 582 passed, 0 failed, 37 ignored |
| Default-feature workspace Clippy | Passed |
| `CARGO_PROFILE_RELEASE_STRIP=none cargo build --workspace --all-features --release` | Passed |
| `node --test apps/server/ui/tests/*.test.js` | 41 passed |
| `python3 -m unittest discover -s scripts/dev -p 'test_*.py'` | 21 passed |
| Ignored FFmpeg decoder qualification | 2 passed |
| Real pinned face-model external test, one-face and two-face fixtures | Both passed |

The ordinary stripped macOS release build hit SQLx proc-macro loading's
`mis-aligned LINKEDIT string pool` error. The release override above is the
validated workaround; the workspace release profile was not weakened.
`make audit` could not run because `cargo-deny` and `cargo-audit` are absent.
Consequently the complete `make check` gate is not claimed green. Ignored tests
require their separately documented external services/artifacts.

Deterministic checks cover mode/configuration separation, SHA pinning, required
640 geometry, inclusive 70% cutoff, malformed/nonfinite outputs, NMS/capacity,
face event scoping, room filtering, packet size, integer precision, stale/reordered
packets, empty results, expiry, letterboxing/DPR and fixture suppression.

## Real model evidence

The public scikit-image v0.24.0 astronaut image (512×512) was letterboxed to raw
640×640 RGB. A manually marked face rectangle (175,60,100,120 pixels) overlapped
the model box at IoU ≥0.5. Confidence was 0.943; returned normalized bounds were
approximately (0.349,0.122,0.179,0.219). The torso was outside the face box.
Two copies side by side produced two boxes at approximately 0.938 confidence.
A blank frame produced zero boxes. These fixtures exercise geometry and adapter
behavior; two copies do not test unique identities or representative accuracy.

Thirty CPU iterations on the one-face fixture measured p50 42.0 ms, p90 52.8 ms,
p99 57.8 ms; two-face fixture p50 40.5 ms, p90 41.1 ms, p99 43.6 ms. Other local
build activity varied during qualification. These numbers measure inference
calls, not end-to-end latency or sustainable camera capacity. Model/session
memory, low-light, occlusion, pose and minimum-face-size qualification remain open.

External test: `cargo test -p surveillance-detector --features onnx-inference
--test onnx_face -- --ignored --nocapture`, with `ORT_DYLIB_PATH`,
`FACE_MODEL_PATH`, `FACE_MODEL_SHA256`, `FACE_FRAME_PATH`, `FACE_SOURCE_WIDTH`
and `FACE_SOURCE_HEIGHT` supplied. Optional `FACE_EXPECTED_BOX` validates manual
box overlap; `FACE_MINIMUM_DETECTIONS=2` selects the two-face expectation.

## Live and failure evidence

Plain `make run` was restored with the real model and defaults. The launcher
selected the host's macOS default-route interface instead of the tool/VPN UDP
socket address, retaining loopback-only signaling. The real webcam supplied
1280×720 H.264 at 30 FPS via the existing owned FFmpeg development adapter.

`SENTINEL_EXPECT_FACE=true make verify-live-browser` (externally supplied
Playwright) passed with two simultaneous authenticated Chromium tabs. Over
16 seconds, decoded frames advanced 1→478 and 1→473; both stayed connected,
readyState 4. The UI accepted real `face/v1` packets and showed “Faces detected”.
This checks the real token API, video decoding and face transport, without mocked
packets or token re-signing. No claim is made about webcam detection accuracy.

A correctly checksum-pinned but corrupt ONNX artifact separately put detection
in Terminal state while both Chromium tabs continued decoding over 16 seconds.
Recording completed six segments, face events remained zero and inference failure
was visible. The real model was restored after that test.

Actual Firefox playback remains unverified. The user confirmed enabling
`media.peerconnection.ice.loopback`; that setting alone did not resolve the
reported connection problem. The newer host-interface fix passed Chromium and
needs a real Firefox run. File-backed RTSP face end-to-end qualification and the
full manual turn-away/reconnect/recovery matrix remain open. A hard timeout for
hung foreign inference is not qualified by a startup-artifact failure test.

Local ignored logs: `.local/face-{tests,clippy,release,decoder,model-one,model-two,
browser-default,failure-browser}.log`. Public fixture files remain under `.local/`;
no personal face crops, recognition embeddings or persisted face identities were
introduced.
