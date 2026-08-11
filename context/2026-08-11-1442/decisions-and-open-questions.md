# Decisions and Open Questions — 2026-08-11 14:42 +06

## Decisions

- Use `tracing` events at state/failure boundaries, not per media item.
- Libraries remain subscriber/backend neutral.
- Camera ID and bounded enums/counters are acceptable dimensions; paths appear
  only on segment/recovery events and must not become metric labels.

## Open

- Server log subscriber format/filter and metrics exporter.
- In-progress health snapshot publication.
- Forced-process harness and runnable configuration parser.
