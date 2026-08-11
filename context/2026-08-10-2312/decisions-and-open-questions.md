# Decisions and Open Questions — 2026-08-10 23:12 +06

## Decisions

- Adopt pinned Tokio for application async task/runtime semantics.
- Keep media in the bounded queue and use `watch` only for generations.
- Treat input closure and cancellation as distinct recorder exits.
- Return failed access unit, sink error, queue, and sink on task failure.
- Abort on task-owner drop rather than silently detaching.

## Open questions

- Application shutdown deadline and forced-abort policy/metrics.
- Real sink finalization behavior on drain versus cancellation.
- Supervisor restart/escalation after recorder sink failure.
- Multi-thread runtime sizing and blocking media-library isolation.
