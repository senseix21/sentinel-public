# Next actions — 2026-08-11 22:59 +06

1. Finish P4.9 telemetry: reconciliation backlog/classification, query/playback,
   storage-pressure, and retention counters/readiness with bounded labels.
2. Add the remaining metadata-writer shutdown/recovery evidence and consolidate
   the Phase 4 fixture into the strongest locally reproducible end-to-end run.
3. Run the final workspace gate, PostgreSQL Phase 4 fixture, Phase 0 RTSP fixture,
   Phase 3 multi-camera fixture, and `git diff --check` after all changes.
4. Update the roadmap and snapshot only from those final results. Keep external
   camera, hardware, production database, and target-storage gates explicit.
