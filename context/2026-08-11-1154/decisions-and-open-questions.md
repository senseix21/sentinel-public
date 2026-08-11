# Decisions and Open Questions — 2026-08-11 11:54 +06

## Decisions

- Critical recording lifecycle facts use bounded backpressure and never a
  drop-oldest policy.
- Storage is awaited only after a bounded completed segment exists; the recorder
  media queue absorbs this pause and exposes overflow upstream.
- Completion is emitted only after durable rename and successful probe.
- Producer closure drains and finalizes; cancellation does not promise queue
  drain but still handles the active segment explicitly.

## Open questions

- Server-level restart policy when camera, recorder, metadata consumer, or API
  finishes unexpectedly.
- Startup reconciliation of `.partial.mp4` and durable-but-unindexed files.
- Whether single-sample shutdown fragments should be retained or discarded.
- Representative-camera DTS/B-frame support and real storage limits.

