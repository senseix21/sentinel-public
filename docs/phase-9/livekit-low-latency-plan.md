# LiveKit low-latency qualification plan

Date: 2026-08-19

## Scope and facts

This slice improves the existing browser live path without changing its owner or
adding another camera connection:

```text
RTSP H.264 -> bounded media distributor -> FFmpeg H.264/I420 decoder
          -> libwebrtc H.264 encoder -> LiveKit -> browser video element
```

Recording remains the higher-priority encoded consumer. Live viewing may drop
stale media, but it may neither back up ingestion nor affect recording.

The transcode adapter uses `TimestampAuthority::AdapterClock`. FFmpeg can reorder
decoded pictures, so the implementation has no defensible mapping from a
camera-media timestamp to a browser frame. This work therefore does **not** claim
camera-to-glass latency. It measures the operator-visible interval from pressing
Watch to the first rendered browser frame, subsequent frame gaps, and recovery
after a browser reconnect.

The first viewer starts publication only at an access unit that begins a
decodable run. Startup is consequently bounded by the source keyframe interval
in addition to signalling, process startup, encode, network, decode, and render
costs. The deterministic Phase 9 fixture starts with 1280x720 H.264 at 15 fps and
a GOP of 30 frames (two seconds).

## Provisional objectives

These are reversible engineering objectives, not confirmed product SLOs:

- watch-to-first-rendered-frame p90 at or below 1 second on the local fixture;
- reconnect-to-first-rendered-frame p90 at or below 2 seconds;
- no browser frame gap above 500 ms during a healthy 15 fps local run;
- no sustained live queue growth, stale eviction, or adapter restart;
- no regression in recording segment completion while viewing starts/stops;
- zero decoding after the final viewer's bounded idle grace expires.

The product owner still needs to confirm the real camera keyframe interval,
viewer concurrency, target hardware, network topology, and live-latency SLO.

## Measurement contract

An authenticated, headless Chrome fixture will run repeated cold and warm watch
trials. Each trial records monotonic browser timestamps for:

1. Watch requested.
2. Viewer token returned.
3. LiveKit room connected.
4. Video track subscribed.
5. First rendered frame reported by `requestVideoFrameCallback`. The native
   source can emit black keepalive frames before camera media, so this is a
   render-start measurement, not a claim that camera content is already usable.
   The following maximum frame gap captures the visible startup discontinuity.
6. Maximum inter-frame callback gap during the observation window.
7. Stop completed.

The fixture also snapshots the existing bounded-cardinality LiveKit metrics:
viewer count, queue drops, stale drops, published frames, and adapter restarts.
Results include every sample and p50/p90/max; no percentile is inferred from a
single trial. Tests cover the statistics and browser state-transition recorder.

## Decision procedure

1. Establish the unchanged baseline with at least ten cold watches.
2. Attribute latency from the browser milestones and server metrics.
3. Change only the dominant controllable component.
4. Repeat the identical workload and retain the change only if the objective
   improves without queue, restart, recording, lifecycle, or shutdown regressions.

Likely interventions, in decision order:

- retain the published track and decoder for a short, configured grace after the
  last viewer leaves, eliminating keyframe/process churn during navigation;
- document/configure an appropriate camera GOP when cold-start keyframe wait is
  dominant (the server cannot manufacture a source keyframe);
- cap publication resolution only when CPU measurements show encode saturation;
- change FFmpeg/libwebrtc options only with an isolated measurement proving an
  improvement and an H.264 compatibility test.

Encoded pass-through is not part of this slice: ADR 011 and ADR 012 record that
the upstream `webrtc-sys` pass-through encoder is not selected on supported
platforms. Simulcast is also excluded because it multiplies encoding work and no
current requirement needs multiple layers.

## Acceptance and rollback

The slice is complete when the benchmark is reproducible, before/after evidence
is recorded, lifecycle and overload tests pass, the authenticated browser
fixture renders every route and live video, and the full workspace gate passes.
Any tuning is reverted if it increases live drops/restarts, affects recording,
leaves decoding active beyond its documented bound, or does not materially
improve the repeated benchmark.
