# Decisions and Open Questions — 2026-08-10 22:56 +06

## Decisions

- Separate domain lifecycle from muxer/filesystem implementation.
- Use consuming Rust transitions so lifecycle terminal actions are single-use.
- Require durable rename/probe evidence before representing completion.
- Retain partial path, stage, and timing on failure for reconciliation.
- Represent only the qualified H.264 fragmented-MP4 format.

## Open questions

- Muxer boundary: in-process library versus supervised FFmpeg child process.
- Production filesystem synchronization and power-loss durability policy.
- Source of authoritative dimensions and exact start/end after media probing.
- Bounded metadata reconciliation when PostgreSQL is unavailable.
- H.265 recording container and timestamp-normalization policy.
