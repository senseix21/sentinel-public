# Repository Status — 2026-09-05 11:16 +06

## Verified facts

- Branch: `feature/ffmpeg-webcam-source`; base commit:
  `12175b433f87a2afc3b8f3a4321b17523d778951`.
- A source-neutral `H264SourceConfig` now selects either the existing Retina
  RTSP/TCP session or a server-owned FFmpeg session.
- The FFmpeg session has typed AVFoundation, V4L2, and DirectShow inputs,
  bounded Annex-B parsing, a decodable-start readiness gate, a 90 kHz H.264
  clock, explicit read/startup timeouts, and owned child/stderr teardown.
- `make run` no longer starts MediaMTX. On this Mac it reached `connected` and
  `/ready`, recorded 712 access units, completed one 30-second segment, then
  finalized the active segment on Ctrl-C. Both new MP4 files fully decoded.
- The ignored managed-FFmpeg/libx264 process test passed when explicitly run.
- Formatting, Clippy with warnings denied, all non-ignored Rust tests, and the
  release workspace build passed. `cargo deny` and `cargo audit` passed after
  updating transitive `rtrb` from 0.3.4 to 0.3.5 for RUSTSEC-2026-0274.

## Preserved worktree state

The branch began dirty. MediaMTX files and several MediaMTX-backed fixtures were
already deleted, while direct-camera qualification and documentation were
already modified/untracked. Those changes were incorporated rather than reset.

## Still unverified

- Physical webcam capture on Linux and Windows.
- Multi-camera combinations of RTSP and FFmpeg sources.
- Replacement external RTSP auth/reconnect fixtures after the prior MediaMTX
  harness deletion.
- 10/20-camera capacity, viewer latency, and target deployment hardware.
