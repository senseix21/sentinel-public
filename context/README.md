# Repository Context Index

This directory contains compact, dated handoff snapshots. Its purpose is to let a new Codex or engineer resume work without rereading all of `analysis.md` and `src/` on every session.

## Latest snapshot

**2026-10-23:** [`context/2026-10-23/`](2026-10-23/) — RTSP consumer recovery fixed and regression-tested; three real two-viewer outage replays passed, 590 Rust tests passed, 36 recordings decoded and Rust server/detection teardown is prompt. Whole-stack shutdown still varied while waiting on LiveKit. Long-outage metadata conflicts remain a separate investigation.

**2026-10-20 12:35 +06:** [`context/2026-10-20-1235/`](2026-10-20-1235/) — README startup/testing walkthrough; local system qualification passed initial live/recording checks but exposed live/detection stalls after RTSP recovery and failing dependency advisories. Read this snapshot before the historical entries below.

**2026-10-09 23:36 +06:** [`context/2026-10-09-2336/`](2026-10-09-2336/) — combined default implemented; real back-facing RTSP/browser and face failure tests pass. Sustained recorder backpressure remains unresolved.

**2026-10-09 17:38 +06:** [`context/2026-10-09-1738/`](2026-10-09-1738/) — specification for continuous person presence with face-only boxes; proposed bounded dual-model design, no runtime changes.

**2026-10-09 15:03 +06:** [`context/2026-10-09-1503/`](2026-10-09-1503/) — face-only real ONNX default at 70%; model, Chromium live and failure isolation verified. Production accuracy and actual Firefox remain open.

**2026-10-09 01:00 +06:** [`context/2026-10-09-0100/`](2026-10-09-0100/) — Firefox still blocks loopback WebRTC; Chromium success did not qualify it. Added persistent local-Firefox connection guidance, with 34 dashboard tests passing. User-side Firefox setting and actual Firefox playback remain unconfirmed.

**2026-10-09 00:49 +06:** [`context/2026-10-09-0049/`](2026-10-09-0049/) — fixed blank local live video by explicitly advertising loopback WebRTC media. Actual Chromium decoding now passes across dashboard refreshes; a repeatable browser qualification target and real LiveKit config regression were added.

**2026-10-09:** [`context/2026-10-09/`](2026-10-09/) — `make run` enables real ONNX inference, caches the model/runtime, and opens the dashboard. Verified with the Apple Silicon webcam; fixed Bash empty-array startup and changed local FPS to a supported 30 FPS mode. Read this snapshot first for launcher status; the prior snapshot below retains the broader detection handoff.

**2026-10-05 00:00 +06:** [`context/2026-10-05-0000/`](2026-10-05-0000/) — PR #11 is merged. A real ONNX detection path, a cross-frame tracker, a browser-side bounding-box overlay with per-tile person counts, four env knobs for the model and thresholds, and an ROI exclude mask all now live on `main`. The 1-in-3 restart flake that opened the session is deterministic. Score and NMS threshold classifications stayed `PipelineRestart` with a documented reason: the inference capture sites don't re-read live, so `Immediate` would be a lie. Previous snapshot, **2026-10-04 20:21 +06:** [`context/2026-10-04-2021/`](2026-10-04-2021/) — ONNX backend measured against ground truth: 73.7% recall, 89.7% precision, 36.9% small-subject recall on sixty COCO frames.

Read in this order:

1. [`next-actions.md`](2026-10-23/next-actions.md) — remaining reliability and production gates.
2. [`repository-status.md`](2026-10-23/repository-status.md) — implementation and verification.
3. [`architecture.md`](2026-10-09-2336/architecture.md) — combined mode and ownership.
4. [`decisions-and-open-questions.md`](2026-10-09-2336/decisions-and-open-questions.md) — qualification gates.

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
