# Repository Status — 2026-08-10 23:06 +06

- Branch `main`; HEAD `9ec5175`; pre-existing dirty worktree preserved.
- Added a recorder integration test spanning camera, media, and recorder crates.
- The deterministic path covers RTP/H.264 depayloading, access-unit assembly,
  independent bounded queues, ordered recorder delivery, closure, and isolated
  live freshness loss.
- This remains SDK-neutral: there is no RTSP connection, async task, or muxer.
