# Repository Status — 2026-08-11 11:35 +06

- Branch `main`; HEAD `9ec5175`; pre-existing dirty worktree preserved.
- The authenticated supervisor fixture now proves publisher stop/start recovery,
  a fresh connected epoch, resumed media, and orderly cancellation.
- Added a bounded pure-Rust H.264 fragmented-MP4 adapter using pinned muxide
  0.2.5 behind the recorder boundary.
- Added root-owned storage persistence: exclusive partial creation, async write,
  flush/file sync, atomic rename, directory sync, and post-rename FFprobe.
- Added epoch-aware media-to-wall-clock mapping and explicit `SystemTime` to UTC
  path conversion; reconnect/timebase/clock discontinuities are tested.
- The accelerated fixture creates three consecutive 30-second H.264 segments
  through the Rust RTSP and muxing paths. All are FFprobe-readable and fully
  FFmpeg-decodable with observed 640x360 H.264 metadata.
- All 55 default workspace tests pass, plus five camera fixture tests and one
  three-segment recording fixture. Formatting, workspace clippy with warnings
  denied, release build, and `git diff --check` pass.
