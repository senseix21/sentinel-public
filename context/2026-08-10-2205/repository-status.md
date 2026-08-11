# Repository Status — 2026-08-10 22:05 +06

- Branch `main`; HEAD `9ec5175`; dirty worktree preserved.
- `surveillance-camera` now depends inward on `surveillance-media`.
- Added bounded RFC 6184 H.264 depayloading for single NAL, STAP-A, and FU-A.
- Seven camera tests cover success, aggregation, fragmentation, sequence wrap/loss, malformed input, bounds, and end-to-end access-unit assembly.
- Six media access-unit tests remain green.
- No third-party Rust dependency was added.
