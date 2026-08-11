# Next Actions — 2026-08-11 11:54 +06

1. Add the one-camera application owner that concurrently polls camera and
   recorder futures, orders graceful shutdown, and returns both outcomes.
2. Implement startup reconciliation for partial and durable-but-unindexed files,
   with deterministic forced-shutdown tests.
3. Add validated Phase 2 runtime configuration for source, credentials, queue
   bounds, segment duration/limits, storage root, and reconnect/clock policy.
4. Add structured tracing and bounded-cardinality health/queue/segment metrics.
5. Close remaining H.264-only qualification and memory-bound evidence, then
   audit every Phase 2 checkbox and exit gate.
