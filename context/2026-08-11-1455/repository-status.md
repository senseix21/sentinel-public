# Repository Status — 2026-08-11 14:55 +06

- `surveillance-server` is a runnable one-camera H.264 ingest-to-record process.
- Required identity, URL, and dimensions load from environment; credentials are
  accepted only as a complete separate pair.
- Startup validates the full runtime transaction and performs bounded recovery
  discovery before opening RTSP.
- The process owns tracing setup, lifecycle-event draining, health, Ctrl-C,
  sibling joins, and final exit reporting.
- Two pure environment-boundary tests pass. Empty-environment startup exits 1
  with the expected missing-camera-ID error and no secret material.
