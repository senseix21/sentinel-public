# Next Actions — 2026-08-10 23:04 +06

1. Define owned async recorder task lifecycle, wake-up, cancellation, and join
   behavior; no detached tasks.
2. Wire camera depayloading through distributor and recorder consumer in one
   deterministic integration harness.
3. Implement symlink-safe partial-file creation and atomic durable rename after
   choosing the target OS/filesystem policy.
4. Spike the H.264 fragmented-MP4 sink with representative access units.
5. Measure queue byte budgets and close external requirements gates.
