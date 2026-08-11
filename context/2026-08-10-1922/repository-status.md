# Repository Status — 2026-08-10 19:22 +06

## Identity

- Branch: `main`.
- HEAD: `9ec5175`.
- Worktree remains uncommitted and dirty with the workspace migration and Phase 0 work. No user changes were reverted.

## Material changes

- Added executable `scripts/phase0/rtsp-reconnect-fixture.sh`.
- Added `docs/phase-0/timestamp-contract.md`.
- Updated ADR 001, media-spike evidence, and Phase 0 roadmap status.

## Fixture verification

The approved localhost fixture completed successfully and wrote evidence beneath `/private/tmp/surveillance-rtsp-fixture.8wLGTy`:

- Started MediaMTX in its own temporary working directory.
- Published and captured H.264 generation 1.
- Stopped the exact publisher PID and verified the RTSP path was offline.
- Started a fresh H.264 generation 2 with reset source timestamps.
- Captured packet summaries and decoded both outputs successfully.
- Stopped only owned processes through cleanup traps.

Both generations began on a keyframe at PTS/DTS 1.867 seconds. Source timestamps reset identically after reconnect. A nominal four-second FFmpeg copy captured 32 packets through PTS 3.933, demonstrating that source PTS origin cannot define elapsed segment duration.

Generated MediaMTX certificates exist only in the temporary fixture directory. No credential was generated in the repository.

