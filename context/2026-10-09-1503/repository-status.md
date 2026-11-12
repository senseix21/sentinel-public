# Repository status — 2026-10-09 15:03 +06

Verified: face-only ONNX inference is implemented and plain `make run` is running
with the real pinned model. Default score is 70%; the dashboard opens through
the development login proxy. The face overlay uses real face geometry, not
shrunk person boxes, and does not label/count identities. No commit was made;
the working tree includes earlier launcher, token and local-live fixes.

Verified: workspace all-feature tests 582 passed / 37 ignored, all-feature
Clippy and formatting passed, UI 41 and development Python 21 tests passed.
Release passed with `CARGO_PROFILE_RELEASE_STRIP=none`; ordinary stripped release
hits the macOS SQLx macro linker error. Audit tools are missing. The complete
`make check` gate is not green. Detailed reproducible evidence and limits:
[VERIFICATION.md](../../docs/face-detection/VERIFICATION.md).

Verified: two Chromium tabs decode webcam video and receive real face packets
using plain launcher defaults; corrupt-model startup leaves recording and live
working. macOS automatic media-IP selection now uses the host route/interface.
Firefox playback remains unverified; the user confirmed its loopback preference
was enabled. That confirmation did not establish working playback.
