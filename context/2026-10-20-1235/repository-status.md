# Repository status — 2026-10-20

README now contains detailed startup, shutdown, recording checks, deterministic,
all-feature/ONNX, PostgreSQL/configuration, security, Chromium and file-backed
RTSP/recovery instructions, with actual local verification results.

One ignored playback test previously failed its own invalid locator/zero-size
metadata before reaching FFmpeg. It now supplies nonempty corrupt bytes through
a valid .mp4 locator, exercising the intended pre-response media refusal.
Phase 4 fixture passed: 13 tests plus SQL checks. Phase 7 passed: 28 plus SQL.

Validation: format and all-target/all-feature Clippy passed. Default make verify
passed with CARGO_PROFILE_RELEASE_STRIP=none (580 Rust, 34 ignored; UI 46;
Python 24). All-feature tests with cached ORT_DYLIB_PATH: 587 passed, 39 ignored.
Two real-model presence tests passed separately. All-feature release passed
with stripping disabled; plain make verify reproduced the SQLx LINKEDIT failure.

One Apple Silicon macOS H.264 RTSP source, 1280x720 at 20 FPS, combined real
YOLOX/YuNet inference, PostgreSQL, LiveKit and Chromium ran for about eight
minutes. Initial two-tab decoding and marked front/back/empty scene checks
passed. Authenticated HTTP reads, indexed search, downloaded clip decode and
unauthenticated 401 passed. MediaMTX showed exactly one surveillance reader.

Intentional source outage preserved liveness, degraded readiness, then recovered
ingest into epoch 2 and resumed recording. BUT browser decode timed out after
recovery. Live frames stuck at 754, published access units at 775; detection
inferred stuck at 3017. Live media rejection and detection drop counters grew,
while health still claimed Running/Publishing. This is a confirmed local
recovery failure, not a claim about its root cause. Zero recorder queue
rejections and zero segment failures observed; all 17 finalized MP4s decoded.
Shutdown finalized the last segment, but launcher server join took about 33
seconds before other services stopped. Its 30-second termination fallback means
this is not evidence of prompt graceful server exit. Historical sustained
recorder-backpressure failure remains unresolved.

cargo deny check fetched current data outside the sandbox and failed advisory
policy (7 vulnerability findings: 4 rustls, 3 zip). Bans, licenses and sources
passed. cargo-audit is absent; make audit and Phase 8 security fixture stop at
that prerequisite. Full make check is not green.

Raw evidence: target/verification-2026-10-20/ (ignored local output).
Physical webcam, Firefox, production accuracy and target-hardware scale remain
unverified. No production code was changed; no commit was made.
