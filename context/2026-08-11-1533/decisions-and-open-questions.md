# Decisions and Open Questions — 2026-08-11 15:33 +06

## Decisions

- Separate one-second provisional internal fragments from file rotation.
- Persist only complete muxer emissions; never write an incomplete MP4 box.
- A dropped writer preserves the partial and cannot publish a durable path.
- Keep the whole-segment `persist` adapter temporarily for migration/tests.

## Open

- Exact configurable fragment interval after disk-sync and crash-loss evidence.
- Whether every fragment or a bounded group receives `sync_data` before file
  finalization.
- Recovery classification of probeable versus truncated partials.
