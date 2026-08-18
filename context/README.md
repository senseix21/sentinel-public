# Repository Context Index

This directory contains compact, dated handoff snapshots. Its purpose is to let a new Codex or engineer resume work without rereading all of `analysis.md` and `src/` on every session.

## Latest snapshot

**2026-08-18 15:27 +06:** [`context/2026-08-18-1527/`](2026-08-18-1527/) — a state-of-the-tree handoff. No code changed; `make check` was re-run green on the working tree (43 suites, 509 passed, 0 failed, 34 ignored; `cargo deny` clean, `cargo audit` one allowed warning) and the tree itself was audited. That audit is why this snapshot exists: 93 untracked files and 8124 lines of untracked Rust sit on top of `e65a318`, including the whole of `crates/auth`, the server's authentication and trust-boundary modules, the 2424-line dashboard, three of five migrations, ADRs 009–012, every Phase 8 and Phase 9 document and fixture, and six context snapshots — two of which this file already points readers at. None of it has been committed or pushed, and `.cargo/audit.toml` and `deny.toml` are among them, so a fresh clone cannot reproduce the gate result above. Getting that tree committed is next-actions item 1 and outranks every engineering item; the worktree was left untouched. Roadmap position unchanged: Phase 8's gate is open (10 `[x]`, 13 `[~]`, 7 `[!]`) and Phase 9 has 12 unchecked items, three drafted evidence documents, and no baseline. All ten open items from the previous snapshot carry forward unverified, the 41% publish shortfall still first among them. Previous snapshot, **2026-08-17 21:59 +06:** [`context/2026-08-17-2159/`](2026-08-17-2159/) — a defect that killed live video after every camera reconnect is fixed. A fresh RTSP session starts at a new arbitrary RTP base, so a reconnect moved the media timeline *forward*; the publisher's discontinuity guard covered only a timebase change or a backward jump, so pacing slept off the difference, and since that loop is the live subscription's only consumer the whole live lane stalled while health still reported `Publishing`. `max_lag` is now the pacing bound in both directions, the jump resynchronises at the next parameterised keyframe, a new `Degraded → Publishing` lifecycle edge stops `Degraded` being a one-way door, and `surveillance_livekit_timestamp_discontinuities_total` makes the event visible. Regression test proven to fail without the fix; `make check` green at 509 passed; verified live across a forced reconnect with the live lane draining at source rate and evictions flat. Playback and the API were verified end to end on the way in. The 41% publish shortfall is untouched and still open — everything here ran at `viewers` 0, and its leading candidate (distributor eviction, 1.7%) is eliminated with no replacement. Previous snapshot, **2026-08-17 14:50 +06:** [`context/2026-08-17-1450/`](2026-08-17-1450/) — live video works from a cold start: the dashboard's `<video>` element decodes and renders the camera at 1280x720. Encoded pass-through was abandoned for decode-and-re-encode behind a viewer gate (ADR 012), so nothing decodes while nobody is watching. Published versus arrived measured at 98% (8.85 frames/s out of 9.05 access units/s), with zero send failures, zero reconnects and zero decoder restarts over a 60 s session. Three defects found by running it rather than reading it: a silent zero-output FFmpeg flag, a shutdown that killed the decoder before it flushed, and one `Overloaded` restart on every subscribe that turned out to be process startup rather than backpressure. A Phase 5 failure-isolation test that had never been linkable on macOS now runs. No latency claim is obtainable through this path by construction, and CPU per viewer is still unmeasured. Previous snapshot, **2026-08-16 23:41 +06:** [`context/2026-08-16-2341/`](2026-08-16-2341/) — the `lk` child-process publisher replaced by in-process native encoded LiveKit publication, which is then *refused at runtime on Apple targets*: `webrtc-sys` 0.3.42 never selects the requested pass-through backend for the initial encoder, and VideoToolbox segfaults the process after exactly 9 access units. Two own-code defects fixed along the way (a discarded error kind that made every send failure retryable, and a missing `Publishing → Terminal` lifecycle edge). `make check` green; the Slice 7 metric-name check run for the first time. Live video at source frame rate remains unproven and needs a product-owner decision. Previous snapshot, **2026-08-16 07:08 +06:** [`context/2026-08-16-0708/`](2026-08-16-0708/) — the dashboard driven through an authenticated session for the first time, via a trust-boundary proxy fixture; four more defects found that way, including `api.io_timeout` cancelling playback for any client that paces its reads; Playback search and the Settings write path verified against real segments and a real database; `AddCameraRequest` now carries LiveKit publication. One playback stall inside the running page remains unexplained in headless Chromium.

Read in this order:

1. [`next-actions.md`](2026-08-18-1527/next-actions.md) — current position and immediate work.
2. [`repository-status.md`](2026-08-18-1527/repository-status.md) — verified repository facts and test results.
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
