# Decisions and open questions

## Decided this session

- Phase 6 work was committed on a branch rather than to `main`, and pushed.
  `main` is untouched pending a merge decision.
- `AGENTS.md` carries the verified repository state; `CLAUDE.md` imports it
  rather than duplicating it, so there is one source of truth.
- Detection model and runtime decisions are recorded in
  [`detection-model-design.md`](detection-model-design.md).

## Open questions

- Hardware for detection. Blocked on Erik as of 2026-08-14; see
  [`detection-model-design.md`](detection-model-design.md) for the seven
  questions sent and why each one blocks.
- Whether `phase-6-detection-and-events` merges to `main` directly or via a
  pull request.
- Whether the empty `crates/api`, `crates/analytics`, `crates/rules`, and
  `apps/worker` stubs are deleted to satisfy the Phase 1 exit gate, or the gate
  is accepted as open.
- Phase 7 needs authenticated settings APIs, but authentication is Phase 8 and
  does not exist. Either authentication moves forward into Phase 7 or settings
  stay loopback-only and the item is explicitly deferred.
- The 64-detection truncation is a product decision hiding in a validation
  constant. Crowded scenes silently drop detections beyond 64, which matters if
  any rule ever counts people rather than detecting presence.

## Unchanged external product gates

Accepted production model and accuracy criteria, labeled dataset, confidence
policy, tracking need, accepted detection freshness p90, representative 4 MP
cameras, and a ten-camera combined recording/live/detection soak.
