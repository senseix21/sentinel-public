# Phase 8 Plan — ordered slices and status

Companion to `SPEC.md`. This file carries status; `SPEC.md` carries contracts.

Legend matches `ROADMAP.md`: `[ ]` not started, `[~]` partial, `[x]` done with
evidence, `[!]` blocked on external hardware or a product decision.

## Ordering rationale

P8.1 first: it is the only package with no prerequisite, and it decides which of
the rest matter most. Then the identity spine (P8.2 → P8.3 → P8.4), because each
is the previous one's prerequisite and together they are what let the listener
leave loopback. P8.5 is independent of that spine and unblocks per-camera
credentials, the single most likely thing to block a real ten-camera deployment.
P8.6 needs a principal to attribute to, so it follows P8.2. P8.7 and P8.10 are
independent and run last. P8.8 is blocked; P8.9 is partial.

## Slices

| # | Slice | Depends on | Status |
|---|---|---|---|
| 1 | P8.1 threat model | — | `[x]` `docs/phase-8/threat-model.md`, 7 boundaries, 6 accepted risks |
| 2 | P8.2 `crates/auth` domain: principal, token, verification | 1 | `[x]` 58 tests |
| 3 | P8.2 persistence: `0003_auth.sql`, `crates/database/src/auth.rs` | 2 | `[x]` schema refusals qualified against real PostgreSQL |
| 4 | P8.2 wiring: extract bearer, fill `authorize` authentication step | 3 | `[x]` Argon2id off-thread, bounded concurrency |
| 5 | P8.3 `Action` scope + `Role` matrix; authorize body complete | 4 | `[x]` exhaustive role×action matrix |
| 6 | P8.3 apply to every protected route; table-driven chokepoint test | 5 | `[x]` 13 routes, 3 open endpoints asserted open |
| 7 | P8.4 trust boundary + proof of origin | 4 | `[x]` peer allowlist + constant-time proof, rightmost XFF |
| 8 | P8.4 relax `HealthBindNotLoopback` under proven conditions | 5, 7 | `[x]` `security_config::resolve_bind`, test per condition |
| 9 | P8.5 secret store, permissions, rotation, per-camera credentials | — | `[x]` 17 tests; **ownership check not implemented** (see below) |
| 10 | P8.6 `0004_audit.sql` + access/camera/export/authz audit | 4 | `[x]` all five categories have a producing route; verified live |
| 11 | P8.7 auth metrics, dashboards, alert rules + firing tests | 5 | `[~]` metrics verified live; 3 dashboards + 13 alert rules written with every metric name machine-checked against a real server; **expressions and panels themselves unvalidated** (no promtool, no Grafana) |
| 12 | P8.10 `deny.toml`, `cargo audit` in gate, fuzz targets | — | `[~]` policy in the gate; **fuzzing not implemented** |
| 13 | P8.9 target-independent runbooks | 9, 10 | `[~]` 4 written, 3 exercised, none by a separate operator |
| 14 | `scripts/phase8/security-fixture.sh` + failure matrix evidence | all | `[x]` 18 checks, `docs/phase-8/local-evidence-2026-08-15.md` |
| — | P8.8 packaging | deployment target | `[!]` unconfirmed target |
| — | P8.9 deploy/upgrade/rollback/backup runbooks | deployment target | `[!]` unconfirmed target |

## What did not get done, and why

Stated plainly rather than buried in the table:

- **Packaging** and four runbooks: no confirmed Linux target. ADR 009 declines
  to guess one, because a runbook written against a guessed target will be
  followed.
- **Alert firing tests**: `promtool` is not installed here. Thirteen rules and a
  firing/silent case per critical alert are written and unvalidated.
- **Dashboards**: three written, none rendered. Metric *names* are verified by
  `scripts/phase8/check-metric-names.py` in the fixture; whether a panel reads
  well, or a threshold is sensible, is unproven.
- **The operator UI does not exist.** `docs/phase-8/ui-spec.md` specifies it and
  names seven server-side gaps that block a complete one — chiefly no audit-log
  endpoint, no system-health JSON endpoint, and no decided browser auth path.
- **Fuzzing**: `cargo-fuzz` needs nightly; this gate is stable 1.88 with
  `unsafe_code = "forbid"`. Targets are named and prioritised.
- **Secrets-file ownership**: needs the process euid, which needs a `libc` call
  that `unsafe_code = "forbid"` rules out. Mode `0600` is enforced; ownership
  belongs with the service user that packaging creates.
- **Failure-matrix cases 9 and 10**: carried from Phase 7 and still untested.
  Concurrent settings updates need two writers against one database.
- **No runbook executed by a separate operator.** The three exercised were
  exercised by their author.
- **The export audit row itself has not been observed live**, only its
  authorization row. Producing one needs real recorded segments in the index and
  on disk; the range/byte-count mapping is unit-tested instead.
- **`mint-token` is an example, not a product surface.** It is the only way to
  create the first identity; promoting it to a subcommand belongs with the
  blocked packaging work.

## Per-slice exit condition

A slice is done when: its contracts compile, its focused tests pass, no new
`unwrap`/`expect`/`panic!` exists in a production path, no hardcoded deployment
value or credential was introduced, and `ROADMAP.md` reflects it with evidence.

The full gate (`make check`, `cargo deny check`, the four fixtures) runs once at
the end, per `AGENTS.md` §"Continue an authorized phase to its gate". Focused
tests and compilation checks remain required per slice.

## Known limits of this plan

Stated now so they are not discovered as surprises at the gate:

- **Packaging cannot be completed.** No confirmed Linux target. ADR 009 refuses
  to guess one.
- **Four runbooks cannot be written** for the same reason.
- **Alert firing is tested against synthetic series**, not against a live
  Prometheus. That is a weaker claim than "the alert fired in production" and is
  labelled as such.
- **Runbook execution is by me against local fixtures**, not by a separate
  operator following it literally. The handoff's standard is the latter; the gap
  is recorded rather than papered over.
- **No representative cameras and no target hardware**, unchanged from Phases
  3–7. Failure-matrix cases involving real camera flapping use synthetic sources.
