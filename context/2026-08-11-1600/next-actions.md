# Next Actions — 2026-08-11 16:00 +06

Detailed implementation handoff: `docs/phase-3/HANDOFF.md`.

1. Begin Roadmap Phase 3 with a process-level supervisor contract that owns a
   map of independent camera pipeline tasks and their join/restart policy.
2. Define explicit add/remove camera commands and cancellation semantics before
   choosing an API representation.
3. Add a two-camera deterministic fixture proving one source failure does not
   interrupt the other recorder.
4. Generalize the current single-camera health view into bounded-cardinality
   per-camera registry snapshots without introducing shared media ownership.
5. Retain representative-camera and target-hardware qualification as explicit
   external gates; do not infer them from the synthetic fixture.
