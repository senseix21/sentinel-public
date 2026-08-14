# Phase 7 Implementation Plan

Reads with `docs/adr/008-persistent-configuration.md` (decisions) and
`docs/phase-7/contracts.md` (types, table, schema). This file is sequencing and
acceptance only.

Written 2026-08-14. Branch: `phase-6-detection-and-events`.

## Sequencing rationale

The order is forced by three dependencies, not by preference:

1. `Secret<T>` precedes every persisted and API path. Introducing it after those
   paths exist means auditing each one for leakage instead of preventing leakage
   structurally.
2. The `SettingId` table precedes persistence, because a setting key is a durable
   identity that appears in the database, the API, metric labels, and audit rows.
   Renaming one after P7.5 is a migration.
3. The event (P7.7) follows persistence (P7.5), because ADR 008 requires
   publication strictly after commit. Building the event first would invite
   publishing from inside the transaction.

P7.8 lands last only because it is mechanical, not because it is optional. It is
the item the handoff names as most likely to be quietly skipped, and it is the
one that closes exit gate 15.

## P7.1 — Configuration domain

**Files.** `crates/config/Cargo.toml` (add `serde`, `toml`, `sha2`),
`crates/config/src/secret.rs` (new), `crates/config/src/classification.rs` (new),
`crates/config/src/lib.rs`.

`toml 0.8.23` and `sha2 0.10.9` are present in the local cargo registry cache,
so the pinned additions resolve without network access.

**Acceptance.**
- `SettingId` covers every row of the contracts table; `const fn key`, `scope`,
  `class`, `boundary`, `secrecy` are exhaustive matches.
- A test asserts `SettingId::ALL` has no duplicate keys, no empty keys, and a
  length equal to a declared constant — so adding a variant without adding it to
  `ALL` fails.
- A test asserts every `Bootstrap` setting has boundary `ProcessRestart`.
- `Secret<T>` `Debug`, `Display`, and `Serialize` render `***` for a value whose
  plaintext appears nowhere in the output. Asserted by searching the rendered
  string for the plaintext, not by comparing to an expected literal.
- `Fingerprint::of` matches published SHA-256 test vectors, and differs when the
  salt differs for identical values.

## P7.2 — Layered resolution

**Files.** `crates/config/src/resolution.rs` (new),
`crates/config/src/validation.rs` (stub → real), `config/default.toml`,
`config/example.toml`, `apps/server/src/runtime_config.rs`.

Precedence `defaults < file < persisted < environment`, per ADR 008. The existing
environment loader is not replaced; it becomes the highest-precedence layer and
keeps its typed errors.

**Acceptance.**
- Round-trip: defaults → file override → persisted override → environment
  override, asserting the effective value and the reported `Origin` at each step.
- A value invalid at each layer is rejected with the layer named.
- `config/default.toml` parses into exactly the compiled defaults. A test asserts
  equality, so the file cannot drift into documentation again.

## P7.3 — Change classification and reporting

**Files.** `crates/config/src/change.rs` (new).

**Acceptance.**
- Each boundary class produces the correct `ChangeOutcome`.
- A change to an environment-pinned setting reports `PersistedButShadowed` with
  the winning origin, not `Applied`.
- A change to the current effective value reports `Unchanged` and writes no audit
  row.
- Pending counts are exposed per boundary.

## P7.4 — Transactional application

**Files.** `crates/config/src/change.rs`, `crates/config/src/validation.rs`.

Group changes route through the existing `OneCameraConfig::validate` arithmetic
rather than duplicating the memory-budget computation.

**Acceptance.**
- A batch with exactly one invalid field rejects wholly; no field is applied and
  no audit row is written.
- A single-key watermark update that breaks the joint invariant is rejected with
  `GroupInvariant`.
- No coercion: an out-of-range value is rejected, never clamped. Asserted by
  passing a value one step out of range and checking the error, not the result.
- A camera memory-budget breach caused by a legal-looking single field is
  rejected.

## P7.5 — Persistence and audit

**Files.** `migrations/0002_settings.sql` (new),
`crates/database/src/settings.rs` (stub → real), `crates/config/src/cache.rs`
(new), `crates/config/src/repository.rs` (stub → real).

`repository.rs` holds the store-shaped trait so `crates/config` does not depend
on `crates/database`; the Postgres implementation lives in `crates/database` and
the dependency points inward.

**Acceptance.**
- Settings row and audit row commit together; an induced audit failure rolls back
  the settings write. Proven with a real transaction, not a mock.
- Startup with store present, absent, and partially populated.
- `Stale` on store unavailability with a valid cache; cached values effective;
  defaults never substituted.
- `Stale` clears on the first successful load without a restart.
- Corrupt cache is preserved, not overwritten; reported as `CacheState::Corrupt`.
- Cache rewritten on every successful load; survives `SIGKILL` mid-operation.
- Cache write failure does not fail the load.
- Corrupt or schema-mismatched persisted row is rejected with the key named.

Tests needing a live database are `#[ignore]`d, consistent with the existing
`recording_segments` tests, and run from the fixture.

## P7.6 — Settings API

**Files.** `apps/server/src/authorize.rs` (new),
`apps/server/src/health_http.rs`.

**Acceptance.**
- Mutating routes return `404` when `api.mutating_surface_enabled` is false, and
  the default configuration has it false.
- Every mutating endpoint reaches `authorize`, asserted by a test that counts
  chokepoint invocations across the full mutating route table, so a future route
  added without the check fails the test.
- Reads never emit a secret value through any path, asserted by scanning the
  serialized response for the plaintext.
- Stored and effective values are reported separately.
- An update arriving during shutdown is rejected with `ShuttingDown`.
- Concurrent updates to overlapping and to non-overlapping settings.

## P7.7 — Configuration event

**Files.** `crates/events/src/event.rs`, `apps/server/src/health_http.rs`.

**Acceptance.**
- Published strictly after commit; a test with an induced commit failure asserts
  no event is published.
- Payload carries keys and outcomes, never values. Asserted by scanning the
  serialized event for a known value.
- Bus overflow during publication does not roll back the change and does not fail
  the response; audit and notify outcomes are counted separately.
- `state_key()` collapsing behaves as `LatestState` requires.

## P7.8 — Remove hardcoded operational values

**Files.** `apps/server/src/health_http.rs` (`MAX_REQUEST_BYTES`, `IO_TIMEOUT`),
`apps/server/src/main.rs` (viewer token TTL, reconciliation queue/interval/
backoff, metadata queue/flush/retry, playback probe timeout),
`apps/server/src/runtime_config.rs` (`StorageWatermarks::new(80, 90, 97, 2)`),
`config/default.toml`.

Full inventory of what moves, verified by reading the tree:

| Value | Location |
|---|---|
| `MAX_REQUEST_BYTES = 16 * 1024` | `health_http.rs:31` |
| `IO_TIMEOUT = 5s` | `health_http.rs:32` |
| viewer token TTL `5 * 60s` | `main.rs:56` |
| reconciliation queue `256` | `main.rs:78` |
| reconciliation interval `120s` | `main.rs:79` |
| reconciliation backoff `30s` | `main.rs:80` |
| metadata queue `256` | `main.rs:106` |
| metadata flush `250ms` | `main.rs:107` |
| metadata retry `30s` | `main.rs:108` |
| playback probe timeout `5s` | `main.rs:117` |
| watermarks `80, 90, 97, 2` | `runtime_config.rs:94` |

`api_concurrent_requests` in `config/default.toml` becomes a comment documenting
the serial accept loop, not a setting. Promoting it would imply a concurrency
limiter that does not exist.

**Acceptance.**
- A test asserts each moved value is resolvable from configuration and that the
  compiled default equals the value it replaced, so the sweep changes
  configurability without changing behavior.
- Exit gate 15 closes on this evidence.

## Failure matrix coverage map

The 18 numbered cases in ADR 008 map to packages as follows. No case is left
without an owner.

| Cases | Package |
|---|---|
| 1, 2 | P7.2 |
| 3 | P7.4 |
| 4, 5, 11, 12, 17 | P7.6 |
| 6, 7, 8, 9, 10, 13, 14 | P7.5 |
| 15, 16 | P7.3 |
| 18 | P7.7 |

Cases 15 and 16 — a boundary change applied while a camera streams and records,
and a restart-scoped change proven not to apply early — need a running pipeline
and belong in the fixture rather than a unit test.

## Deferred

Not started. Each is blocked on a decision gate that no amount of implementation
resolves.

| Package | Blocked on |
|---|---|
| P7.9 Rules as event consumers | The conditions and actions are unknown. The mechanism is implementable; building it against invented conditions produces a rules engine nobody asked for. |
| P7.10 ROI definition | Coordinate system semantics unconfirmed. |
| P7.11 Analytics and heatmap | The meaning of a heatmap — density, occupancy, or movement — is unconfirmed. |
| P7.12 Tracking | Only if confirmed analytics require identity over time. |

`ROADMAP.md` records the corresponding instruction, test, and exit-gate items as
`[!]`. Exit gate 17 ("ROI/rule/analytics behavior matches confirmed product
semantics") cannot close while these are open, and will not be reported as
closed.

## Validation

```
make check
cargo test -p surveillance-config --all-features
cargo test -p surveillance-database --all-features
scripts/phase4/postgres-fixture.sh
scripts/phase7/config-fixture.sh
```

`scripts/phase7/config-fixture.sh` qualifies precedence, restart persistence,
rollback, and PostgreSQL-outage degradation against a real database. Output goes
to `docs/phase-7/local-evidence-2026-08-15.md` in the Phase 6 evidence format.

Anything that cannot run — no PostgreSQL, no camera — is reported as unverified
with the reason, not omitted.
