# Repository Status — 2026-08-10 23:12 +06

- Branch `main`; HEAD `9ec5175`; pre-existing dirty worktree preserved.
- Tokio `1.53.1` is pinned at the workspace boundary with only macros, current
  thread runtime, sync, and time features enabled.
- Media subscribers now have cancellation-safe async receive wake-ups without
  moving media onto the notification channel.
- Recorder now provides an owned task handle. Normal exits return queue/sink
  ownership; owner drop aborts instead of detaching.
- Focused runs pass 14 media tests, 11 recorder unit tests, and one recorder
  integration test.
