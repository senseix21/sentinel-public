# Repository Status — 2026-08-10 23:19 +06

- Branch `main`; HEAD `9ec5175`; pre-existing dirty worktree preserved.
- Camera now depends on stable `CameraId` from common.
- Added adapter-independent state-specific camera lifecycle types and
  credential-free operational snapshots.
- Twelve focused camera tests pass: five lifecycle and seven H.264 depayloader
  tests.
- Retina 0.4.19 is selected only for the next deterministic RTSP/TCP spike; it
  has not been added as a dependency and no camera support claim is made.
