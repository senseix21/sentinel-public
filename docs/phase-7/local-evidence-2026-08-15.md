# Phase 7 Local Evidence — 2026-08-15

Machine: macOS (Darwin 24.6.0), Apple Silicon. Branch
`phase-6-detection-and-events`, on top of `c71f952`.

What was measured and what was assumed are separated below. Every command named
here was run; results are quoted, not summarised from expectation.

## Commands run

```
cargo fmt --all --check                                         clean
cargo clippy --workspace --all-targets --all-features -- -D warnings   clean
cargo test --workspace --all-features                           296 passed, 0 failed, 32 ignored
cargo build --workspace --release                               clean
scripts/phase7/config-fixture.sh                                passed
scripts/phase4/postgres-fixture.sh                              passed
```

Phase 6 recorded 126 passed / 20 ignored. Phase 7 adds 170 passing tests and 12
ignored integration tests.

PostgreSQL 17 was available locally, so the store-dependent tests **ran** rather
than being reported as unverified.

## `scripts/phase7/config-fixture.sh`

```
Phase 7 schema constraint fixture passed
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
Phase 7 restart persistence fixture passed
Phase 7 store-outage fixture passed
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 11 filtered out
Phase 7 store-recovery fixture passed
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 128 filtered out
Phase 7 configuration fixture passed
```

The fixture stops PostgreSQL mid-run, confirms a load fails as a typed error
rather than a panic or a silent default, restarts it, and confirms the same load
succeeds — which is the mechanism by which `Stale` clears without a restart.

## `scripts/phase4/postgres-fixture.sh`

```
Phase 4 PostgreSQL migration, constraint, overlap, and index-plan fixture passed
test result: ok. 7 passed; 0 failed
test result: ok. 1 passed; 0 failed        (playback_assembler)
test result: ok. 1 passed; 0 failed        (reconciliation_owner)
test result: ok. 1 passed; 0 failed        (retention_owner)
test result: ok. 1 passed; 0 failed        (database_outage)
```

Run deliberately as a regression check: P7.8 deleted the environment-only
retention loader that hardcoded `StorageWatermarks::new(80, 90, 97, 2)`, and
retention now assembles from the configuration domain. The Phase 4 retention and
outage evidence is unchanged.

## Failure matrix

The 18 cases in ADR 008, and where each is proven.

| # | Case | Where | Status |
|---|---|---|---|
| 1 | Invalid value at each precedence layer | `file.rs`, `environment.rs`, `value.rs` | covered |
| 2 | Conflicting values across layers; declared precedence wins | `resolution.rs` | covered |
| 3 | Multi-field update, exactly one field invalid | `change.rs::one_invalid_field_rejects_the_whole_batch` | covered |
| 4 | Concurrent updates, overlapping and not | — | **not covered** |
| 5 | Update during shutdown | `authorize.rs::shutdown_refuses_changes` | partial — chokepoint only |
| 6 | PostgreSQL unavailable at startup, and lost while running | `settings_persistence.rs`, fixture | covered |
| 7 | Store unavailable with valid / corrupt / absent cache | `cache.rs` | covered |
| 8 | `Stale` clears on first success without restart | fixture store-outage → store-recovery | covered |
| 9 | Cache rewritten on every load; survives forced exit | `cache.rs` rewrite + atomic rename | partial — no `SIGKILL` test |
| 10 | Settings commit but audit write fails; neither commits | `settings_persistence.rs::an_audit_write_failure_rolls_back_the_settings_change` | covered |
| 11 | Mutating endpoints disabled by default, rejected when disabled | `health_http.rs`, `defaults.rs` | covered |
| 12 | Every mutating endpoint reaches `authorize`, by test | `health_http.rs::every_mutating_settings_request_reaches_the_chokepoint` | covered |
| 13 | Persisted row corrupt or schema-mismatched | `settings_persistence.rs::a_corrupt_stored_row_is_reported_rather_than_dropped` | covered |
| 14 | Restart with settings present / absent / partial | `settings_persistence.rs`, fixture restart section | covered |
| 15 | Boundary change while streaming and recording | — | **not covered** |
| 16 | Restart-scoped change does not apply early | `change.rs::a_restart_scoped_setting_is_pending_restart_and_does_not_apply_early` | covered |
| 17 | Secret read through every path including audit and logs | `secret.rs`, `value.rs`, `settings.rs`, `health_http.rs` | covered |
| 18 | Audit event admission failure under bus overflow | `event.rs` bounds; publication counted separately | partial — no live overflow test |

**Four cases are not fully covered, and are not claimed to be:**

- **Case 4 (concurrent updates)** is not tested. The transaction boundary and the
  `settings_generation_sequence` make the design defensible, but ADR 008 says
  transactional safety is not to be inferred from code structure. This needs two
  concurrent writers against one database.
- **Case 15 (boundary change during live recording)** needs a running pipeline
  with a camera. It belongs in a media fixture, not a unit test.
- **Case 9** proves atomic replacement and checksum rejection but does not kill
  the process mid-write.
- **Case 18** proves the payload is bounded and that audit and notification
  outcomes are counted separately; it does not drive a subscriber into overflow.

## Exit gate 15 — no hardcoded operational value in core pipeline logic

Every value in the P7.8 inventory now resolves from configuration:

| Value | Was | Now |
|---|---|---|
| `MAX_REQUEST_BYTES` | `health_http.rs:35` | `api.max_request_bytes`, read per request |
| `IO_TIMEOUT` | `health_http.rs:36` | `api.io_timeout`, read per request |
| viewer token TTL | `main.rs:56` | `livekit.viewer_token_ttl` |
| reconciliation queue / interval / backoff | `main.rs:78-80` | `reconciliation.*` |
| metadata queue / flush / retry | `main.rs:106-108` | `metadata.*` |
| playback probe timeout | `main.rs:117` | `playback.probe_timeout` |
| storage watermarks `80, 90, 97, 2` | `runtime_config.rs:94` | `storage.watermark_*` |

`load_retention_from_environment` and `load_retention_with` were deleted rather
than left as dead public API carrying the constants. Their test was **ported,
not removed**: `retention_defaults_are_versioned_and_invalid_overrides_are_rejected`
asserts the same two claims through the configuration domain, and
`storage_watermarks_come_from_configuration_and_match_the_previous_constants`
asserts the shipped defaults equal the constants they replaced.

`api_concurrent_requests` was removed from `config/default.toml` rather than
promoted to a setting. The accept loop is serial; a setting would have implied a
limiter that does not exist.

## A correction made during implementation

The classification table initially declared seventeen settings `Immediate` —
retention, watermarks, reconciliation and metadata intervals, playback, and the
viewer token TTL. Their owners take those values once at construction, so
reporting `Immediate` would have told an operator a change was in force when it
was not. They are now `ProcessRestart`, and the three per-camera recording
settings are `PipelineRestart`.

`api.max_request_bytes` and `api.io_timeout` are the only settings that remain
`Immediate`, and the HTTP adapter genuinely re-reads them per request
(`immediate_api_bounds_are_read_from_configuration`).

Making the others live requires each owner to re-read its configuration on a
loop. That is a real change to Phase 4 and Phase 5 owners and was not attempted
here.

## Assumptions, stated separately

- PostgreSQL is per-unit and local (confirmed 2026-08-14, commit `c71f952`).
  The cache-as-fallback decision depends on this and inverts if it changes.
- The retry budget default of 300 s is a reversible guess at "longer than
  typical service start ordering". No measurement of local PostgreSQL start time
  was taken.
- No capacity or performance claim is made. Nothing here was benchmarked.

## Not implemented

P7.9 (rules), P7.10 (ROI), P7.11 (analytics), P7.12 (tracking) were not started.
Each is blocked on a product decision, recorded as `[!]` in `ROADMAP.md`. Exit
gate 17 cannot close while they are open and is not reported as closed.

Secrets are not persisted and not API-editable, because ADR 003's production
secret store remains open. Consequently a deployment whose cameras need
*different* credentials cannot express that today: the environment layer supplies
templates and has no per-camera syntax.
