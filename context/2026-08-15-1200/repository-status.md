# Repository Status — 2026-08-15

Branch `phase-6-detection-and-events`, on top of `c71f952`. Uncommitted.

## What changed

Phase 7 packages P7.1 through P7.8 are implemented. P7.9 through P7.12 were not
started; each is blocked on a product decision.

### New documents

- `docs/adr/008-persistent-configuration.md` — four decisions: the `authorize`
  chokepoint with a default-off mutating surface; last-known-good cache with a
  transient `Stale`; audit in the settings transaction; and no rollback of a
  committed change when runtime application fails. ADR 003's "Open details" are
  superseded except the production secret store, which stays open.
- `docs/phase-7/contracts.md` — the 79-setting classification table and types.
- `docs/phase-7/PLAN.md` — sequencing and per-package acceptance.
- `docs/phase-7/local-evidence-2026-08-15.md` — measured results, including the
  four failure-matrix cases that are **not** covered.

### New code

| Path | What |
|---|---|
| `crates/config/src/classification.rs` | The setting table as a macro: a row missing any of seven axes does not parse |
| `crates/config/src/secret.rs` | `Secret<T>` and salted `Fingerprint` |
| `crates/config/src/value.rs` | Kind-directed parsing; duration units required |
| `crates/config/src/resolution.rs` | Layered resolution with per-setting origin |
| `crates/config/src/defaults.rs` `file.rs` `environment.rs` | The three bootstrap layers |
| `crates/config/src/change.rs` | Change classification and all-or-nothing planning |
| `crates/config/src/assemble.rs` | Bridges resolved settings to the existing validated types |
| `crates/config/src/cache.rs` | Last-known-good cache and the `Stale` retry |
| `crates/config/src/repository.rs` | Store contract; the impl lives in `database` |
| `crates/database/src/settings.rs` | Postgres repository; settings and audit in one transaction |
| `migrations/0002_settings.sql` | Schema, with partial unique indexes and CHECK constraints |
| `apps/server/src/authorize.rs` | The chokepoint; `Authorized` has no public constructor |
| `apps/server/src/settings_service.rs` | Owns effective configuration, applies changes |
| `scripts/phase7/config-fixture.sh` | The Phase 7 evidence run |

`config/default.toml` and `config/example.toml` are now real inputs. A test
asserts `default.toml` resolves to exactly the compiled defaults.

## Verified 2026-08-15

```
cargo fmt --all --check                                         clean
cargo clippy --workspace --all-targets --all-features -- -D warnings   clean
cargo test --workspace --all-features                           296 passed, 0 failed, 32 ignored
cargo build --workspace --release                               clean
scripts/phase7/config-fixture.sh                                passed
scripts/phase4/postgres-fixture.sh                              passed
```

PostgreSQL was available, so the store-dependent tests ran rather than being
reported unverified.

## What is deliberately not true yet

- **No authentication.** `authorize` grants on loopback. Phase 8 owns identity.
  Roadmap item "authenticated settings APIs" is `[~]`, not `[x]`.
- **Secrets are not persisted and not API-editable.** ADR 003's secret store is
  open; writing credentials into the settings table would decide it by accident.
  Consequence: cameras cannot have *different* credentials, because the
  environment layer supplies templates and has no per-camera syntax.
- **Only two settings are `Immediate`.** The initial table declared seventeen,
  but their owners capture them at construction, so the report would have lied.
  They are `ProcessRestart` now. `api.max_request_bytes` and `api.io_timeout`
  are genuinely re-read per request.
- **Concurrent updates are untested.** The transaction boundary and generation
  sequence make the design defensible; ADR 008 forbids calling that proof.
- **A boundary change during live recording is untested.** It needs a running
  pipeline with a camera.
- **Exit gate 17 cannot close** while ROI, rules, and analytics are open.

## Next

1. Two concurrent writers against one database (failure-matrix case 4).
2. A media fixture for a boundary-scoped change during active recording (case 15).
3. Obtain the four product decisions blocking P7.9–P7.12, or start Phase 8.
4. If retention and watermarks need to be live rather than restart-scoped, each
   owner must re-read on its loop. That is a change to Phase 4 owners and should
   be its own slice.
