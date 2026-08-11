# Repository Status — 2026-08-10 21:35 +06

- Branch: `main`; HEAD: `9ec5175`.
- Worktree remains intentionally dirty with the uncommitted workspace migration and Phase 0 evidence.
- Physical webcam path published H.264 track `TR_VCHqUJnMK7vA4E`; a second LiveKit participant subscribed.
- AVFoundation source was 1280x720 UYVY 4:2:2 at approximately 15 FPS; source adapter converted to `yuv420p` and encoded H.264 Constrained Baseline.
- The RTSP-to-LiveKit leg used stream copy, but processed only about 2.8 FPS and 8.74 media seconds in roughly 36 wall seconds.
- MediaMTX discarded frames for a slow reader and terminated that RTSP session on I/O timeout.
- All owned processes stopped and the temporary Unix socket was removed.

This is compatibility evidence plus negative throughput evidence. Browser rendering and latency were not established.
