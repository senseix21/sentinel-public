# Decisions and open questions — Phase 4 local gate

## Decisions implemented

- PostgreSQL metadata is asynchronous to durable media publication.
- Reconciliation and retention are continuously owned, bounded, cancellable tasks.
- Indexed half-open queries are the only production search path.
- Playback is reversible segment-aligned H.264 stream-copy, bounded to 24 hours,
  1,000 segments, one sequential viewer, and 64 KiB of application buffering.
- Retention claims before unlink and protects the newest configured window.

## Open product/deployment decisions

- Playback overshoot, download/range support, viewer concurrency, and any
  frame-accurate transcoding mode need product approval.
- Retention thresholds and emergency disk policy need deployment approval.
- Production PostgreSQL, target filesystem durability, representative cameras,
  and hardware capacity require qualification outside the local repository.
