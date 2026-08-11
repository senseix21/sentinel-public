# Repository Status — 2026-08-10 19:31 +06

## Identity

- Branch: `main`.
- HEAD: `9ec5175`.
- Worktree remains uncommitted and dirty with the workspace migration and Phase 0 work. No user changes were reverted.

## Material changes

- Added executable `scripts/phase0/container-crash-fixture.sh`.
- Added `docs/phase-0/container-finalization.md`.
- Accepted ADR 004 for the H.264 MVP persistence/container lifecycle.
- Updated ADR 001, timestamp/media evidence, and Roadmap Phase 0.

## Verified crash matrix

Fixture output: `/private/tmp/surveillance-container-fixture.EM0yJB`.

- Normally finalized H.264/H.265 Matroska, fragmented MP4, and classic MP4 all probed and decoded.
- Interrupted H.264/H.265 Matroska files probed and decoded; duration was unavailable.
- Interrupted H.264 fragmented MP4 retained two decodable seconds.
- Interrupted H.265 fragmented MP4 retained five decodable seconds.
- Interrupted classic MP4 failed to probe for both codecs.

The first three-second HEVC interruption attempt occurred before sufficient muxer output was flushed and was rejected as a confounded run. The corrected run allowed six seconds for HEVC and is the decision evidence.

No repository files were used for generated media. Each interrupted encoder was killed by its exact owned PID and the harness cleaned up any active child on exit.

