# Repository Context Index

This directory contains compact, dated handoff snapshots. Its purpose is to let a new Codex or engineer resume work without rereading all of `analysis.md` and `src/` on every session.

## Latest snapshot

**2026-10-05 00:00 +06:** [`context/2026-10-05-0000/`](2026-10-05-0000/) — PR #11 is merged. A real ONNX detection path, a cross-frame tracker, a browser-side bounding-box overlay with per-tile person counts, four env knobs for the model and thresholds, and an ROI exclude mask all now live on `main`. The 1-in-3 restart flake that opened the session is deterministic. Score and NMS threshold classifications stayed `PipelineRestart` with a documented reason: the inference capture sites don't re-read live, so `Immediate` would be a lie. Previous snapshot, **2026-10-04 20:21 +06:** [`context/2026-10-04-2021/`](2026-10-04-2021/) — ONNX backend measured against ground truth: 73.7% recall, 89.7% precision, 36.9% small-subject recall on sixty COCO frames.

Read in this order:

1. [`next-actions.md`](2026-10-05-0000/next-actions.md) — current position and immediate work.
2. [`repository-status.md`](2026-10-05-0000/repository-status.md) — verified repository facts, test results, and the prototype record.
3. [`architecture.md`](2026-10-04-2021/architecture.md) — child-process ownership, recording retrieval, and the enforcement boundary (unchanged this session).
4. [`decisions-and-open-questions.md`](2026-10-04-2021/decisions-and-open-questions.md) — decisions, evidence, and blockers (unchanged this session).

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
