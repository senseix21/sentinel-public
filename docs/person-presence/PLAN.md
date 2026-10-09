# Person presence implementation plan

Date: 2026-10-09
Status: local slice implemented and qualified; external and sustained reliability gates open.
Contract: [SPEC.md](SPEC.md). ADR: [017](../adr/017-person-presence-with-face-overlays.md).

1. [x] Reproduce current face-only turn-away semantics with deterministic input;
   inventory frame geometry, queues, gating and model artifacts.
2. [x] Add bounded evidence/snapshot contracts and a time-based tracker. Verify
   confirmation, misses, reacquisition, expiry, saturation and resets.
3. [x] Register validated combined-mode settings. Add canonical preprocessing,
   independently owned latest-frame model lanes, global admission, health and
   supervisor shutdown. Verify stale work and partial model failure.
4. [x] Add camera-scoped presence events/transport and independent face/presence
   browser state. Preserve existing topics and face-only rendering.
5. [x] Qualify real models, turn-away/stationary fixtures, two browser viewers,
   recording continuity and failure isolation. Change launcher default only
   after local end-to-end qualification.
6. [x] Finish formatting, workspace Clippy/tests/release, UI/helper checks and
   required security checks. Record exact unavailable external gates.
7. [x] Update operator docs, roadmap, verification and dated context snapshot.

Representative accuracy, numerical product targets, finite recovery from hung
foreign calls, actual Firefox and target-hardware 10/20-camera qualification
remain separate gates. They must not be reported complete from local fixtures.

Task 6 is locally complete except dependency audit tooling and plain stripped macOS release.
Task 5 proves controlled short-run isolation; sustained recording failed and remains open.
See VERIFICATION.md for precise evidence and limitations.
