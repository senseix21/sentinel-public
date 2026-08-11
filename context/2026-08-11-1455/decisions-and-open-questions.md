# Decisions and Open Questions — 2026-08-11 14:55 +06

## Decisions

- No default RTSP endpoint exists.
- Camera dimensions are required until reliable configured/probed comparison is
  integrated into the muxer boundary.
- Environment is the temporary Phase 2 deployment/secret boundary; TOML files
  are explicitly reference documentation, not silently ignored runtime input.
- `tracing-subscriber` 0.3.23 is application-owned.

## Open

- Forced-process partial-file test.
- External end-to-end test through the runnable binary.
- HTTP health/metrics export and production authentication boundary.
