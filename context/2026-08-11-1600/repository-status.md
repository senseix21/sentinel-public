# Repository Status — 2026-08-11 16:00 +06

## Verified facts

- Phase 2's one-camera H.264 ingest-to-record vertical slice is implemented.
- The muxer emits a separate init section and bounded one-second complete fMP4
  fragments; the recorder appends and `sync_data`s each emission to the active
  partial rather than retaining a complete 30-second file in memory.
- The storage writer enforces the actual output-file byte bound. Graceful
  completion syncs, atomically renames, probes, and publishes truthful evidence.
- Required configured dimensions are compared with Retina-negotiated H.264
  dimensions before media is accepted.
- Loopback-only `/live`, `/ready`, and `/metrics` expose process, connection,
  camera, and bounded queue state without retaining media.
- `scripts/phase0/retina-rtsp-fixture.sh` passed on 2026-08-11. It covered five
  RTSP camera cases, a direct three-segment recorder run, the composed server,
  health probes, full decode, SIGINT, SIGKILL, partial decode, and restart
  reconciliation discovery.
- Final workspace gates passed after implementation: `cargo fmt --all --check`,
  Clippy for the workspace/all targets/all features with warnings denied, 71
  ordinary tests, all doc tests, and the release workspace build. Six
  hardware/service-backed tests remain ignored in ordinary `cargo test`; all
  six were run successfully by the repository fixture (five camera tests and
  one recorder test).

## Scope boundary

Phase 2 qualifies video-only H.264 without B-frames against the deterministic
fixture. It does not claim H.265, audio, arbitrary H.264 composition offsets,
or representative production-camera interoperability.
