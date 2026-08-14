# Next actions — Phase 6 committed, detection blocked on hardware

Unblocked, in recommended order:

1. Add CI for format, clippy, tests, and release build. Phase 1 requires it, no
   `.github` directory exists, and its absence is why `AGENTS.md` drifted six
   phases out of date.
2. Reconcile the 13 unchecked Phase 1 boxes against evidence, and decide whether
   to delete the empty crate stubs or accept the exit gate as open.
3. Start the Phase 7 configuration slice: precedence, validation reporting the
   application boundary, transactional application, and settings APIs. Decide
   the authentication dependency first.
4. Design the detection benchmark harness. It does not depend on Erik's answers
   because it is what produces them, and it turns "varies by site" into cameras
   per module.

Blocked:

5. Detection model implementation, pending the hardware answers in
   [`detection-model-design.md`](detection-model-design.md).
6. Hardware-decode change to the existing FFmpeg decoder, pending the same.
7. Phase 7 ROI, rules, heatmap, and tracking, pending confirmed product
   semantics. Do not implement a heatmap before its meaning is agreed.
