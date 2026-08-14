# Decisions and Open Questions — 2026-08-15

## Accepted decisions (ADR 008)

1. **One `authorize(request, action)` chokepoint, plus a default-off mutating
   surface.** Two controls because they answer different questions: the flag is
   a deployment posture, `authorize` is a request property. `Authorized` has a
   private field and no public constructor, so a handler that skips the check
   does not compile. A disabled surface answers `404`, not `403` — it does not
   advertise itself.

2. **Last-known-good cache with a transient `Stale`; defaults are never silently
   substituted.** Retention age and storage watermarks *delete recordings* when
   they revert, so starting with stale-but-real configuration is strictly safer
   than starting with fresh-but-wrong configuration. `Stale` clears on the first
   successful load without a restart; the retry budget (300 s) outlasts typical
   service start ordering.

3. **Audit commits in the same transaction as the settings change.** "Changed
   but unlogged" is unrepresentable rather than rare. The bus notification is
   published after the commit and is explicitly not the record.

4. **Runtime application failure does not roll back a committed change.**
   Validation is pure and runs before the commit, so a failure at application
   time is environmental. The row stays, the previous value stays effective, and
   the setting is marked `ApplyFailed`. This is why reads report stored and
   effective values separately.

## Decisions made during implementation

- **Secrets are neither persisted nor API-editable.** Writing credentials into
  the settings table would decide ADR 003's open secret-store question by
  accident. Enforced in three places: the planner, the repository, and a schema
  CHECK constraint.
- **Precedence dominates specificity.** A higher layer's all-camera template
  overrides a lower layer's camera-specific entry. One rule, stated once.
- **Durations require an explicit unit.** A bare `30` was ambiguous between the
  old `_seconds` and `_milliseconds` key suffixes.
- **Legacy `SURVEILLANCE_*` variables keep working** through an alias table, so
  existing deployments and the `scripts/phase*` fixtures are unaffected.

## Corrections made

- The classification table initially declared seventeen settings `Immediate`
  whose owners capture them at construction. Reporting `Immediate` would have
  told an operator a change was in force when it was not. They are now
  `ProcessRestart`; three per-camera recording settings are `PipelineRestart`.
- Four legacy environment variables were listed as aliases while already being
  canonical names, which made them self-conflict. Caught by the alias test.

## Open questions

| Question | Blocks | Owner |
|---|---|---|
| Production secret store and rotation | Per-camera credentials; secret persistence | Product / deployment |
| ROI polygon coordinate semantics | P7.10, exit gate 17 | Product |
| Rule conditions and actions | P7.9 | Product |
| Meaning of "heatmap" | P7.11 | Product |
| Whether analytics need identity over time | P7.12 | Product |
| Authentication and authorization model | Phase 8; the `authorize` body | Product / security |

## Working assumptions

- PostgreSQL is per-unit and local (confirmed 2026-08-14). The cache-as-fallback
  decision inverts if it becomes central and remote.
- The 300 s retry budget is a reversible guess. No measurement of local
  PostgreSQL start time was taken.
- Subsystem priority (recording first) remains the stated assumption from
  `AGENTS.md`, unconfirmed by the product owner. The `Stale` design depends on
  it: configuration freshness never blocks media preservation.
