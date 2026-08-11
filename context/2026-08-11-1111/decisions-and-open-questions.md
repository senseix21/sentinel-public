# Decisions and Open Questions — 2026-08-11 11:11 +06

## Decisions

- Keep the supervisor as an owned future; application code must explicitly own,
  poll/join, and cancel it rather than receiving a detached task handle.
- Treat recorder overflow as a visible camera-pipeline stop for this slice.
- Treat live oldest-item replacement as non-fatal and count it.
- Inject jitter generation so delay arithmetic is deterministic in tests and the
  eventual application can choose its entropy source without hiding randomness.
- Bound both reconnect sleep and RTSP teardown; shutdown interrupts connection
  attempts and reconnect waits.

## Open questions

- Exact policy after recorder saturation: pause ingest, restart recorder, or
  require operator action. Silent retry is intentionally not implemented.
- When consecutive-failure backoff resets after an unstable short connection.
- How lifecycle snapshots and counters are published to health/metrics readers.
- MediaMTX publisher stop/start recovery through a fresh supervisor epoch.
- Production jitter entropy source and configuration bounds.

