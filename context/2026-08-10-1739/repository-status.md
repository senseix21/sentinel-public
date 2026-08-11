# Repository Status — 2026-08-10 17:39 +06

## Identity

- Branch: `main`.
- HEAD: `9ec5175` (`more changes`).
- Worktree remains dirty with the uncommitted workspace migration, planning documents, and pre-existing untracked material. No commit was created.

## Phase 0 increment

- Expanded `docs/requirements.md` into confirmed direction, reversible spike defaults, security/reliability requirements, and explicit product questions.
- Added camera/media matrix, sizing worksheet, risk register, and reproducible media-spike results under `docs/phase-0/`.
- Expanded ADRs 001–003 and added proposed ADR 004 for filesystem video plus PostgreSQL metadata and bounded reconciliation.
- Updated Phase 0 roadmap evidence without declaring the phase complete.

## Local evidence

- FFmpeg/FFprobe 8.0 is installed with libx264, libx265, VideoToolbox, and AVFoundation support.
- MediaMTX was not found on PATH.
- `gst-launch-1.0` exists, but its version probe behaved abnormally in this execution environment; GStreamer is unqualified.
- The development host is an Intel 8-core MacBook Pro with 32 GB RAM and about 105 GiB free. It is not the production target.

Synthetic experiments generated 125 seconds of H.264/30-second segments and H.265/60-second segments. All output decoded without FFmpeg errors. H.264 produced four exact 30-second segments plus a 5-second tail. H.265 produced 59.8, 59.4, and 5.0-second segments and emitted negative-PTS warnings.

Generated media lives only in `/private/tmp/surveillance-phase0.DK0LnJ` and is not a repository fixture.

