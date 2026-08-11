# Repository Status — 2026-08-11 05:37 +06

- Branch `main`; HEAD `9ec5175`; pre-existing dirty worktree preserved.
- Workspace MSRV is now Rust 1.88 because pinned Retina 0.4.19 requires it.
- `camera::rtsp` owns an isolated authenticated H.264 RTSP/TCP playing session;
  Retina types do not cross the adapter boundary.
- The adapter validates redacted source configuration, selects H.264 video,
  exposes probed dimensions/codec, maps bounded complete access units and
  discontinuities, classifies visible RTSP status failures, and provides
  bounded teardown.
- Fifteen camera unit tests and two ignored fixture tests pass. The fixture tests
  are executed by `scripts/phase0/retina-rtsp-fixture.sh` and passed against
  MediaMTX 1.20.0 plus FFmpeg on this Mac.
- No representative physical camera or reconnect supervisor has been qualified;
  this is not a production camera-support claim.

