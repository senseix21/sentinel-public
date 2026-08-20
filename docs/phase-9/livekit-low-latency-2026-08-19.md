# LiveKit browser latency evidence — 2026-08-19

## Workload

`LIVEKIT_LATENCY_BENCHMARK=1 TRIALS=10 scripts/phase9/authenticated-ui-fixture.sh`
started PostgreSQL, MediaMTX, a synthetic H.264 source, the release server,
LiveKit, the proof-bearing authentication proxy, and headless Chrome. The source
was 640x360 at 15 fps with a 30-frame GOP. Trials alternated a six-second cold
gap and a 500 ms warm reconnect. Each observed two seconds of rendered frames.

The measured boundary is Watch press to the browser's first
`requestVideoFrameCallback`. Libwebrtc can publish a black keepalive before the
first camera picture, so this is render start, not usable-picture or
camera-to-glass latency. The maximum callback gap makes startup discontinuity
visible without guessing at pixel content.

## Result with five-second idle grace

| trial class | n | timeouts | first-frame p50 | first-frame p90 | max frame gap |
|---|---:|---:|---:|---:|---:|
| cold (> grace) | 5 | 0 | 4,463 ms | 5,407 ms | 3,250 ms |
| warm (< grace) | 5 | 0 | 779 ms | 1,014 ms | 233 ms |

The server log recorded completed 30-second recording segments while the trials
ran and contained no live-decoder restart or overload warning. This demonstrates
recording continuity for the local run, not a general performance claim.

A subsequent two-trial harness smoke test proved metric capture: stale drops,
live-lane evictions, and adapter restarts each changed by zero; 79 frames were
published. Adapter drops changed by 58 because that counter intentionally also
counts access units declined while no viewer/decodable run exists; it is not an
overload counter.

Two explicit zero-grace control attempts reached the ten-second usable-pixel
probe timeout. That probe was rejected because canvas pixel classification was
not portable enough to become a stable gate; the final harness instead uses the
browser frame callback and reports gaps. Those aborted controls are not included
in the table and no numerical before/after improvement is claimed from them.

## Interpretation

The bounded warm session produces consistent reconnect continuity in this
fixture. Cold starts remain dominated by source/keyframe and WebRTC startup and
miss the provisional one-second objective. The server cannot manufacture a
camera keyframe, and lowering a real camera's GOP is a camera/product decision
with bitrate and quality consequences. A real acceptance decision still needs
representative cameras, target hardware, network topology, viewer concurrency,
and a confirmed latency objective.
