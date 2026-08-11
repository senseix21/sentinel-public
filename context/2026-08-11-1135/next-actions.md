# Next Actions — 2026-08-11 11:35 +06

1. Build the owned one-camera runtime that joins supervisor, bounded distributor,
   segment sink, storage persistence, lifecycle transitions, and task results.
2. Carry the tested media-epoch-to-UTC mapping into open/finalizing/completed
   segment lifecycle ownership.
3. Deliver completed metadata through a bounded critical channel without
   accumulating an unbounded in-memory catalog.
4. Add graceful final-segment completion and forced-shutdown reconciliation for
   partial and durable-but-unindexed files.
5. Add validated Phase 2 runtime configuration and observable health/metrics.
