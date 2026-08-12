# Decisions and open questions — 2026-08-11 22:59 +06

## Accepted/reversible implementation decisions

- SQLx 0.8.6 with Tokio/Rustls and embedded forward-only migrations implements
  the PostgreSQL boundary. Pool default is 8 with a five-second acquisition
  timeout, applied on restart.
- Recorder publication precedes asynchronous database persistence. Durable final
  and partial files are the crash/outage reconciliation journal.
- ADR 004 now records the reversible segment-aligned streamed playback policy.
- Retention defaults to 30 days, protects the newest hour, uses 80/90/97 percent
  watermarks with two-percent hysteresis, and bounds a pass to 100 files/10 GiB.

## External/open gates

- Product approval is still required for playback overshoot, viewer concurrency,
  HTTP range/download behavior, retention thresholds, and emergency disk policy.
- Production PostgreSQL version/topology, TLS, secrets injection, backups, and
  migration ownership are not qualified by the temporary local fixture.
- Target disk durability, disk-full behavior, and representative 4 MP camera soak
  remain unverified.
