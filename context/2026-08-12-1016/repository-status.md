# Repository status — 2026-08-12 10:16 +06

## Verified facts

- All locally implementable Phase 4 work packages P4.1–P4.9 are implemented.
- The final gate passes: formatting, workspace Clippy with warnings denied, 95
  ordinary tests, all doc tests, optimized workspace build, and diff check.
- The PostgreSQL 14.18 fixture passes migrations, seven repository cases, index
  plan, three-segment full-decode playback, divergence reconciliation, interrupted
  retention recovery, and database stop/start catch-up.
- Phase 0 RTSP/crash recovery and Phase 3 ten-camera/two-camera isolation fixtures
  pass after the Phase 4 changes.
- Phase 4 readiness and metrics now distinguish database, metadata delivery,
  camera, and storage-pressure degradation and expose constant-memory counters
  for reconciliation, queries, playback, storage, and retention.
- The working tree is intentionally uncommitted.

## External gates

- Representative cameras, target hardware/storage soak, production PostgreSQL
  topology/TLS/backups, and target-filesystem permission/disk-full injection are
  unavailable and remain explicitly unverified.
