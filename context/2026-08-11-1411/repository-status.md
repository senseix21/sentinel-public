# Repository Status — 2026-08-11 14:11 +06

- Branch `main`; HEAD `9ec5175`; pre-existing dirty worktree preserved.
- Full gate passed before this milestone with 55 default tests, warnings denied,
  formatting, release build, and diff checks.
- `apps/server` jointly owns the camera and mandatory recorder. Camera-first
  exit drains recording; recorder-first exit stops ingest; shutdown joins both.
- Three deterministic orchestration tests pass.
- Bounded, symlink-safe startup discovery classifies partial files only as
  incomplete and durable files as unindexed reconciliation candidates.
- Two recovery tests pass, including fail-closed work limiting.
