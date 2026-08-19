# Repository status — 2026-08-19 23:00 +06

Branch: `perf/livekit-low-latency`.

## Live viewing

- The singular transcode publication owner has a configured, bounded
  `camera.livekit.viewer_idle_grace` (default five seconds). During the grace it
  retains and feeds the existing track/decoder; it creates no second queue,
  decoder, room, or RTSP connection. Zero preserves immediate teardown.
- A returning viewer cancels teardown. After the bound, the next access unit
  stops the decoder and unpublishes the track. Lifecycle behavior has focused
  unit coverage.
- The authenticated Phase 9 fixture can run repeatable headless-Chrome cold and
  warm trials. The qualified boundary and limitations are recorded in
  `docs/phase-9/livekit-low-latency-2026-08-19.md`.

## Remaining decision gates

Camera-to-glass latency is still unmeasurable through the adapter clock. Cold
startup is not accepted, and representative cameras, target hardware, camera
GOP policy, viewer concurrency, deployment topology, and the product SLO remain
unconfirmed.
