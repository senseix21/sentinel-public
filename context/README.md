# Repository Context Index

This directory contains compact, dated handoff snapshots. Its purpose is to let a new Codex or engineer resume work without rereading all of `analysis.md` and `src/` on every session.

## Latest snapshot

**2026-10-01 21:35 +06:** [`context/2026-10-01-2135/`](2026-10-01-2135/) — the three uncommitted arcs became pull request #10 with a green gate; a cancelled FFmpeg startup was leaking its child and is fixed; Phase 1 gained CI and now closes except for four members that hold no implementation. `make run` remains unexecuted on this host because the camera consent prompt cannot reach a non-interactive process. Previous snapshot, **2026-09-05 11:16 +06:** [`context/2026-09-05-1116/`](2026-09-05-1116/) — the source-neutral H.264 boundary and owned FFmpeg webcam source.

Read in this order:

1. [`next-actions.md`](2026-10-01-2135/next-actions.md) — current position and immediate work.
2. [`repository-status.md`](2026-10-01-2135/repository-status.md) — verified repository facts, test results, and the prototype record.
3. [`architecture.md`](2026-10-01-2135/architecture.md) — child-process ownership, recording retrieval, and the enforcement boundary.
4. [`decisions-and-open-questions.md`](2026-10-01-2135/decisions-and-open-questions.md) — decisions, evidence, and blockers.

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
