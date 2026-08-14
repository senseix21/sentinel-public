# Repository status — Phase 6 committed, detection model design paused

Verified facts on 2026-08-14:

- Phase 6 is committed as `9d960f5` and documentation as `93e0b36`, both on
  branch `phase-6-detection-and-events`, pushed to `origin`. The worktree is
  clean. `main` is unchanged at `b5a8a01`.
- `make check` passes: `cargo fmt --all --check`, `cargo clippy --workspace
  --all-targets --all-features -- -D warnings`, `cargo test --workspace
  --all-features` (126 passed, 0 failed, 20 ignored), and release build.
- `cargo test -p surveillance-detector --test ffmpeg_decode -- --ignored`
  passes with FFmpeg/libx264 present.
- `AGENTS.md` "Current Repository Reality" was stale — it described the
  pre-Phase-1 prototype as current. It now separates the verified workspace
  state from the inert root-level `src/` prototype.
- `CLAUDE.md` was added; it imports `AGENTS.md` and carries verified commands,
  the real crate layout, and working conventions.

Verified gaps, unchanged by this session:

- No CI exists. There is no `.github` directory. Phase 1 requires one and all
  13 of its checkboxes remain unchecked.
- `crates/api`, `crates/analytics`, `crates/rules`, and `apps/worker` are empty
  stubs, which violates the Phase 1 exit gate.
- `deploy/docker` and `deploy/systemd` are placeholder READMEs. No deployment
  mechanism exists.
