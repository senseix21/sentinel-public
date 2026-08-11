# Decisions and Open Questions — 2026-08-11 14:24 +06

## Decisions

- Phase 2 configuration is one transaction, not independently mutable fields.
- Credentials remain separate from RTSP URLs.
- Per-camera in-process media memory has an explicit conservative upper bound.
- Queue/item bounds and a total byte budget are both required.

## Open

- File/env parsing and secret-provider boundary for the runnable server.
- Continuous metrics exporter and log subscriber.
- Forced-process test harness.
- Representative-camera-derived production limits.
