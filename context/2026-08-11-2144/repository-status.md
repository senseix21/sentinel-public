# Repository Status — Phase 4 First Slice

## Verified changes

- ADR 004 now fixes crash ordering, provisional bounds, SQLx/PostgreSQL boundary,
  reconciliation and deletion ordering, acceptance test names, and reversal
  criteria while retaining external product/deployment gates.
- `storage` owns validated completed metadata, safe relative durable locators,
  lifecycle transition legality, and bounded half-open range semantics.
- The initial migration adds only cameras and recording segments, with database
  checks for identifiers, interval/evidence validity, relative paths, lifecycle
  values, uniqueness, and overlap/reconciliation/retention indexes.
- `scripts/phase4/postgres-fixture.sh` owns a temporary trust-authenticated local
  PostgreSQL cluster on a private Unix socket and leaves no credential or state.

## Verification

- PostgreSQL 14.18 fixture passed empty migration, migration-runner re-entry,
  invalid completed-row rejection, camera-scoped three-segment half-open overlap,
  deterministic order, and an `Index Only Scan` on
  `recording_segments_completed_overlap_idx`.
- Workspace format and all-target/all-feature Clippy with warnings denied passed.
- Workspace tests passed: 85 ordinary tests, 6 environment-owned tests ignored,
  and all doc tests.
- Workspace release build and `git diff --check` passed.

The SQLx adapter, metadata writer, outage recovery, API, playback, and retention
are not yet implemented and are not claimed by this milestone.
