# Next Actions — 2026-08-10 22:58 +06

1. Implement deterministic camera/UTC partial-path construction inside a
   configured recording root, with validation against traversal.
2. Define owned async recorder task lifecycle and cancellation-safe wake-up.
3. Wire camera depayloading through distributor and recorder consumer in one
   deterministic test harness.
4. Spike a fragmented-MP4 sink against representative H.264 access units.
5. Measure queue byte budgets and close external Phase 0 gates.
