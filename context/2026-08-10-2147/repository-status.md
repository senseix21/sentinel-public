# Repository Status — 2026-08-10 21:47 +06

- Branch `main`; HEAD `9ec5175`; dirty worktree preserved.
- Deterministic single-slice H.264: 150 frames/10.00 media seconds in 9.97 wall seconds at 15 FPS.
- Deterministic LiveKit result: 4,701 packets forwarded, zero dropped.
- Webcam single-slice RTSP-to-LiveKit path sustained approximately 15 FPS after publisher attachment.
- Webcam LiveKit result: subscriber attached; 10,464 packets forwarded, zero dropped.
- Multi-slice pacing failure was reproduced at 11 slices/frame and about 1.4 output FPS.
- Pre-consumer startup caused RTSP slow-reader drops; steady state after attachment did not add warnings.
- All owned media/server processes and temporary sockets were cleaned up.
