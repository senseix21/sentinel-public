# Repository status — 2026-08-18 17:17 +06

Base at the start of this continuation: `e65a318` on
`phase-8-handoff-and-config-metrics`, with a large verified but untracked tree.

## Durable repository state

The existing work is now captured in four commits:

- `1add6a2` — local runtime and browser artifacts ignored.
- `da09e1c` — interdependent LiveKit publication, security, dashboard, and
  migration implementation.
- `2525744` — audit policy, runbooks, alerts, and repeatable fixtures.
- `801f520` — roadmap, governing documentation, and immutable context chain.

The implementation was kept together because `health_http.rs` interleaves the
security chokepoint, dashboard routes, and LiveKit metrics; path-only splitting
would have created revisions that did not compile. No generated runtime,
recording, Playwright, or screenshot artifact was committed.

## Clean-checkout gate

Verified from a detached worktree at `801f520`, not from the development tree:

- `cargo fmt --all --check`: clean.
- workspace clippy with all targets/features and `-D warnings`: clean.
- workspace tests: 509 passed, 0 failed, 34 ignored across 43 suites.
- release workspace build: succeeded.
- `cargo deny`: advisories, bans, licenses, and sources passed.
- `cargo audit`: 461 dependencies, one documented allowed warning,
  RUSTSEC-2025-0119 through `number_prefix <- indicatif <- muxide`.

The first attempt could not download LiveKit's pinned WebRTC archive under
sandboxed DNS. The permitted rerun downloaded it and passed; this was an
environmental dependency, not a source failure.

## Live-path measurements

Detailed evidence is in
`docs/phase-9/live-publication-qualification-2026-08-18.md`.

- Current synthetic 1280x720@15 publication was lossless with 1, 2, and 4
  viewers: 926/926, 927/927, and 928/928 access units/frames.
- The real webcam's recorder independently proved 15.000 FPS. With one viewer,
  publication advanced 301/301 frames over 20 seconds (15.05/s), with zero
  stale drops and zero live evictions.
- The previous 8.85 FPS shortfall is not reproducible on the current pacing
  implementation. No root cause is fabricated retrospectively.
- With one viewer attached across an RTSP interruption, the camera advanced to
  epoch 2, publication counted one discontinuity, the same viewer remained,
  and post-recovery publication ran at 14.96 FPS with no decoder restart.
- Server CPU stayed approximately flat across 1, 2, and 4 viewers (14.5%,
  15.4%, 15.2% of one core); SFU and subscriber CPU increased with fan-out.

## Changed after `801f520`

- `scripts/phase9/livekit-load-fixture.sh`: one-camera BSD `seq` empty-range
  defect corrected with arithmetic loops.
- `ROADMAP.md`: published-versus-source frame accounting closed with current
  evidence.
- New Phase 9 evidence and this context snapshot.

No production Rust changed after the clean-checkout gate.
