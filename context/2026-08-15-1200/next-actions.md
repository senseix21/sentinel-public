# Next Actions — 2026-08-15

## Position

Phase 7 packages P7.1–P7.8 are implemented and verified locally. P7.9–P7.12 are
untouched and blocked. `make check` passes; both PostgreSQL fixtures pass. The
worktree is dirty and uncommitted.

## Immediate work, in order

1. **Failure-matrix case 4 — concurrent updates.** Two writers against one
   database, overlapping and non-overlapping key sets. This is the largest
   remaining gap in Phase 7's own evidence standard: the design relies on one
   transaction plus `settings_generation_sequence`, and ADR 008 explicitly
   forbids treating that structure as proof. Belongs in
   `crates/database/tests/settings_persistence.rs` and the Phase 7 fixture.

2. **Failure-matrix case 15 — boundary change during live recording.** Change a
   `NextSegment` setting while a camera streams and records; confirm the change
   lands at the segment roll and never mid-segment. Needs MediaMTX plus the
   webcam or a file-backed RTSP fixture, so it belongs alongside the Phase 3
   fixtures rather than in a unit test.

3. **Decide whether restart-scoped is acceptable for retention and watermarks.**
   Today an operator who raises retention must restart the process. Making them
   live means `RetentionOwner` and `ReconciliationOwner` re-read configuration on
   their loops. That is a change to working Phase 4 code and should be its own
   slice with its own evidence, not folded into Phase 7.

## Blocked, needing a product decision

| Package | Decision required |
|---|---|
| P7.9 rules | The conditions and actions operators actually want |
| P7.10 ROI | Polygon coordinate system and validation semantics |
| P7.11 analytics | What a heatmap means: density, occupancy, or movement |
| P7.12 tracking | Whether confirmed analytics need identity over time |

Also still open from ADR 003: the production secret store and rotation. Until it
is decided, credentials are environment-only, and cameras cannot have different
credentials from one another.

## Do not

- Do not report the settings API as authenticated. It is loopback-gated with a
  default-off surface; Phase 8 owns identity.
- Do not close exit gate 17. ROI, rules, and analytics semantics are unconfirmed.
- Do not mark a setting `Immediate` unless its consumer genuinely re-reads it.
  The table was corrected once for exactly this reason.
