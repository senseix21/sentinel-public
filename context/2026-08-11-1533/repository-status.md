# Repository Status — 2026-08-11 15:33 +06

- `muxide` 0.2.5 can repeatedly flush independent `moof+mdat` fragments while
  retaining decode-time continuity.
- ADR 005 now separates short internal fragments from 30–60 second recording
  file rotation.
- `SegmentFileStore` exposes singular `begin`, `append`, and `finalize` stages.
- Dropping an active writer leaves its real appended partial bytes and never
  creates a durable path; the new regression test passes.
- Existing whole-segment persistence now delegates through the same staged API.
