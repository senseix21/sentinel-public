# Architecture Context — 2026-08-10 19:22 +06

The media contract now distinguishes five timing concepts:

1. Source PTS/DTS plus explicit timebase for codec ordering.
2. Connection epoch for reconnect/caps discontinuities.
3. Monotonic observation time for elapsed policy, health, and queue lag.
4. UTC recording correlation for operator/date queries.
5. Segment-relative media time normalized at a safe random-access boundary.

A reconnect always creates a new epoch even when the source timestamps resemble or reset to values from the previous epoch. Recording starts a new segment after reconnect and preserves any wall-clock gap in metadata.

Segment rotation is driven by monotonic elapsed policy and completed at a safe media boundary. It must never assume source PTS begins at zero or remains continuous across upstream sessions.

The Rust packet type remains deferred until container crash/finalization behavior is compared, but its required information and invariants are documented in `docs/phase-0/timestamp-contract.md`.

