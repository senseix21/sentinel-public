# Repository Status — 2026-08-10 18:30 +06

## Identity

- Branch: `main`.
- HEAD: `9ec5175`.
- Worktree remains uncommitted and dirty with the workspace migration and planning/Phase 0 material. No user files were reverted.

## Material changes

- Saved `docs/phase-0/2026-mvp-baseline.md` as the product-owner-accepted initial qualification profile.
- Updated requirements and roadmap to reflect the accepted camera/media baseline.
- Upgraded local MediaMTX from 1.15.2 to 1.20.0 through Homebrew.
- Qualified synthetic H.264 and H.265 RTSP/TCP publication, consumption, and copy remux.
- Qualified the built-in webcam through AVFoundation -> yuv420p/H.264 -> MediaMTX -> RTSP/TCP -> copy remux.
- Updated the Phase 0 capability matrix and spike report with measured results.

MediaMTX generated `auto.crt` and `auto.key` in the workspace during the test. Both generated credential files were deleted immediately and are not recoverable from the worktree. Future MediaMTX spikes should use a temporary working directory.

## Results

- H.264 live synthetic: 640×360, 15 FPS, 12.000-second remux, clean decode.
- H.265 live synthetic: copy remux and decode passed; start offset was 1.8 seconds.
- H.265 no-B-frame control: exactly 12.000 seconds but retained 1.2-second start offset, so B-frame reordering is not the sole cause.
- Webcam: 1280×720, approximately 15 FPS, H.264 Constrained Baseline, yuv420p, 10.011-second remux, clean decode.
- Looping an already segmented H.264 file exposed a fixture timestamp/discontinuity failure: MediaMTX saw the track, but the downstream file was empty.

All MediaMTX and FFmpeg test processes were stopped after the experiments.

