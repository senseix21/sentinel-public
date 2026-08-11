# Decisions and Open Questions — 2026-08-11 14:11 +06

## Decisions

- Recording is mandatory for Phase 2; recorder exit stops ingest.
- Camera exit closes production and permits queued recording data to drain.
- Recovery scans are bounded and never follow symlinks.
- Orphan partials remain incomplete evidence and are never silently promoted.

## Open

- Runtime configuration format and secret injection boundary.
- Partial-file quarantine/retention policy.
- Metrics exporter and bounded label contract.
- Representative-camera qualification and exact queue byte budgets.
