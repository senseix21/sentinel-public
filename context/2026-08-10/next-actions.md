# Next Actions — 2026-08-10

## Current position

The repository is at **Roadmap Phase 0 / pre-Phase 1**. Architecture and execution guidance now exist, but the Rust manifest remains structurally broken and none of the proposed production phases has been implemented or verified.

## Read before acting

1. Root `AGENTS.md` for non-negotiable engineering instructions.
2. Root `ROADMAP.md`, beginning with Phase 0 and Phase 1.
3. This snapshot's `repository-status.md` for prototype hazards.
4. `analysis.md` only when architectural detail or original rationale is needed.

## Immediate executable sequence

### Action 1 — Capture a clean baseline

- [ ] Run `git status --short` and `git rev-parse HEAD` without altering user changes.
- [ ] Re-run `cargo check` and record the exact manifest error.
- [ ] Inventory Rust/system dependency versions and installed GStreamer/FFmpeg tools.
- [ ] Identify which existing paths can be characterized without first compiling the crate.

Output: a new context snapshot if facts differ from this one.

### Action 2 — Obtain topology-changing requirements

Ask Erik for the camera/media, hardware, playback, LiveKit, reliability, and p90 decisions listed in `decisions-and-open-questions.md`. At minimum, obtain representative codec/resolution/FPS/bitrate and target server hardware before sizing storage or compute.

Output: `docs/requirements.md` with confirmed versus assumed fields.

### Action 3 — Repair the build as a controlled vertical change

- [ ] Create the root Cargo workspace and pin shared dependency versions.
- [ ] Keep the legacy prototype buildable before moving its modules.
- [ ] Add basic workspace checks and document system prerequisites.
- [ ] Do not scaffold every target crate yet.

Output: passing format, clippy, tests, and release build, or an exact documented external blocker.

### Action 4 — Run media-risk spikes

- [ ] Qualify H.264 and H.265 sample streams.
- [ ] Compare 30/60-second segmentation behavior.
- [ ] Exercise webcam -> FFmpeg -> local MediaMTX -> RTSP manually.
- [ ] Establish deterministic RTSP fixtures for automated tests.
- [ ] Measure CPU, memory, bitrate, and segment correctness.

Output: ADR inputs and reproducible evidence; not production abstractions.

### Action 5 — Implement the first production slice

Proceed to Roadmap Phase 2 only after the Phase 0/1 gates. Build one camera -> one owned RTSP session -> bounded encoded media -> segmented recorder -> truthful metadata -> graceful shutdown.

## Do not do next

- Do not add detection, ROI, heatmaps, rules, or LiveKit before the ingest/record boundary is proven.
- Do not wire the old three-connection design into new crates.
- Do not move all prototype modules wholesale.
- Do not invent p90 targets, camera properties, or storage capacity.
- Do not introduce Kafka/NATS, Kubernetes, microservices, or distributed databases.
- Do not use the physical webcam as an automated CI dependency.

## Next-session completion note

At the end of the next material implementation session, create a new dated context snapshot containing:

- Commit and dirty-worktree state.
- Files changed and why.
- Commands/tests run with outcomes.
- Decisions made and their ADRs.
- New defects or risks.
- Exact next action and blockers.

