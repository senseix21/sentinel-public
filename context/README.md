# Repository Context Index

This directory contains compact, dated handoff snapshots. Its purpose is to let a new Codex or engineer resume work without rereading all of `analysis.md` and `src/` on every session.

## Latest snapshot

**2026-08-18 17:17 +06:** [`context/2026-08-18-1717/`](2026-08-18-1717/) — the previously untracked implementation is now durable in four commits and a detached clean checkout passes the complete gate (509 passed, 34 ignored). Current live publication is lossless at the measured 1280x720@15 workload with 1, 2, and 4 viewers, the exact webcam path publishes at its independently probed 15 FPS source rate, CPU fan-out is measured, and one attached subscriber remains connected while publication recovers from a forced RTSP interruption. The old 8.85 FPS shortfall is not reproducible on the current pacing implementation and is not assigned a speculative root cause. A macOS `seq` empty-range defect in the one-camera Phase 9 harness was found and corrected. Previous snapshot, **2026-08-18 15:27 +06:** [`context/2026-08-18-1527/`](2026-08-18-1527/) — the state-of-tree audit that found the implementation was not in Git and made durability the first action.

Read in this order:

1. [`next-actions.md`](2026-08-18-1717/next-actions.md) — current position and immediate work.
2. [`repository-status.md`](2026-08-18-1717/repository-status.md) — verified repository facts and test results.
3. [`architecture.md`](2026-08-15-1200/architecture.md) — current owned runtime boundary.
4. [`decisions-and-open-questions.md`](2026-08-15-1200/decisions-and-open-questions.md) — decisions, evidence, and blockers.

The paused detection model/runtime design remains in the previous snapshot:
[`detection-model-design.md`](2026-08-14-1914/detection-model-design.md).

Read root `AGENTS.md` for standing engineering rules and `ROADMAP.md` for phase gates. Read `analysis.md` when changing architecture, resolving an ambiguity, or validating that a summary has not lost important context.

## Snapshot rules

- Treat dated snapshots as immutable historical handoffs.
- Create a new snapshot after material code/architecture changes, a phase gate, or a significant discovery.
- Use `YYYY-MM-DD/`; use `YYYY-MM-DD-HHMM/` for multiple material snapshots on one day.
- Update only the `Latest snapshot` link and reading list in this index.
- Record command output, commit ID, changed paths, and test results where relevant.
- Label each statement as verified fact, accepted decision, working assumption, or open question.
- Keep the snapshot compact. Link to durable detail instead of duplicating `analysis.md`, ADRs, or the roadmap.
- Do not include credentials, private RTSP URLs, tokens, or personal information.

## Topic template

Each snapshot should contain:

```text
context/YYYY-MM-DD[-HHMM]/
├── repository-status.md
├── architecture.md
├── decisions-and-open-questions.md
└── next-actions.md
```

Optional topic files such as `recording.md`, `livekit.md`, `detection.md`, or `benchmark-results.md` may be added when a topic becomes too detailed. Add them only when they reduce reload cost.
