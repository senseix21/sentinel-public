# Repository Status — 2026-08-11 14:48 +06

- Camera supervisors publish latest lifecycle state through a Tokio watch
  channel with constant memory and explicit publisher closure.
- Media distributors expose cloneable read-only queue observers that do not
  retain media or affect producer/consumer lifetime.
- `OneCameraHealth` combines current connection and recorder/live queue state.
- A regression test proves latest-value delivery and publisher closure.
- Focused tests and clippy pass; full gate follows this snapshot update.
