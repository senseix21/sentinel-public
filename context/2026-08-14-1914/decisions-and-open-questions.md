# Decisions and open questions

## Decided this session

- Phase 6 work was committed on a branch rather than to `main`, and pushed.
  `main` is untouched pending a merge decision.
- `AGENTS.md` carries the verified repository state; `CLAUDE.md` imports it
  rather than duplicating it, so there is one source of truth.
- Detection model and runtime decisions are recorded in
  [`detection-model-design.md`](detection-model-design.md).
- Three Phase 7 architecture decisions, expanded in
  [`../../docs/phase-7/HANDOFF.md`](../../docs/phase-7/HANDOFF.md): settings
  APIs get an `authorize` chokepoint now and a real implementation in Phase 8,
  with the mutating surface opt-in and default-off; operational settings fall
  back to a local last-known-good cache with a `Stale` state rather than to
  defaults; audit records persist in the same transaction as the settings
  change, because the event bus has no durable log and can drop them.
- PostgreSQL is per-unit, not central (confirmed 2026-08-14). The Phase 7
  last-known-good cache is therefore a fallback against startup ordering and
  corruption, not against network partition, and `Stale` is transient with
  retry. Two consequences recorded outside Phase 7: PostgreSQL competes for
  Jetson unified memory and is an input to the detection sizing rule, and
  per-unit storage means there is no fleet-wide configuration view.

## Open questions

- Hardware for detection. Blocked on Erik as of 2026-08-14; see
  [`detection-model-design.md`](detection-model-design.md) for the seven
  questions sent and why each one blocks.
- Whether `phase-6-detection-and-events` merges to `main` directly or via a
  pull request.
- Whether the empty `crates/api`, `crates/analytics`, `crates/rules`, and
  `apps/worker` stubs are deleted to satisfy the Phase 1 exit gate, or the gate
  is accepted as open.
- The 64-detection truncation is a product decision hiding in a validation
  constant. Crowded scenes silently drop detections beyond 64, which matters if
  any rule ever counts people rather than detecting presence.

## Unchanged external product gates

Accepted production model and accuracy criteria, labeled dataset, confidence
policy, tracking need, accepted detection freshness p90, representative 4 MP
cameras, and a ten-camera combined recording/live/detection soak.
