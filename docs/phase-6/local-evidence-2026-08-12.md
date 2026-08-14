# Phase 6 Local Evidence — 2026-08-12

## Implemented locally

- A third freshness-oriented detection queue shares the camera-owned access
  units with recorder and LiveKit without another RTSP connection.
- Camera lifecycle stamps every distributed access unit with its explicit epoch.
- Validated RGB24 frame, sampling, inference, detection, versioned event, and
  bounded event-bus contracts replace the placeholder modules.
- One owned FFmpeg process adapter continuously decodes H.264 off Tokio workers,
  waits for random access after reset, bounds encoded input/RGB output, and owns
  process/thread teardown.
- Camera, recording, storage-pressure, detector-state, and person facts publish
  through one process-owned event bus.
- Health and Prometheus output expose detector/event state, bounded queues,
  sampling, failures, delivery outcomes, and fixed latency buckets.

Focused warning-free Clippy and focused ordinary tests passed while developing
these contracts. The complete workspace/release and external-fixture gate is
recorded only after the final commands below are rerun.

## Focused evidence

- Event tests cover validation/serialization, ordering, all four overflow
  policies, unsubscribe, close/wake, and closed-bus behavior.
- Sampler tests cover nonzero origins, variable FPS, duplicates, gaps, backward
  timestamps, timebase changes, and reconnect epochs.
- Media tests prove detection saturation retains newest input without rejecting
  recorder admission.
- Detection-owner tests prove typed publication, input-close drain, requested
  joined shutdown, and terminal detector isolation from recorder/live.
- `cargo test -p surveillance-detector --test ffmpeg_decode -- --ignored`
  passed real FFmpeg/libx264 random-access and 40-frame inter-frame fixtures with
  bounded RGB output and ordered source timestamps.

## Named deterministic workload

`/usr/bin/time -l cargo run --release -q -p surveillance-detector --example
fake_workload` ran `deterministic-fake-person-1.0.0-rgb640x360` for 10,000
sequential inferences. The loop measured 4,453 µs total, p50 204 ns, p90 212 ns,
and p99 340 ns. macOS reported a 1,986,560-byte peak footprint. Compilation
dominated the 23.22-second command wall time and is excluded from inference
latency. This is fixture evidence, not accuracy or camera-capacity evidence.

## Failure matrix

| Case | Local evidence and expected behavior |
|---|---|
| Detection disabled | Validated setting drops its queue; health is `Disabled`; no decoder/model owner starts. |
| Decoder missing/incompatible | Construction or terminal decode affects only detection; recorder/live queues retain ownership. |
| Malformed/oversized media/frame | Typed errors reset or terminate detection; item/byte bounds remain enforced. |
| Timestamp discontinuity/reconnect | Sampler and decoder reset; epoch is camera-owned. |
| Detection queue saturation | Oldest detection input is replaced; recorder admission is unchanged. |
| Decode/inference/event saturation | Reader/event queues are bounded; inference is one-at-a-time; counters are visible. |
| Inference unavailable/malformed | Retryable failures degrade with bounded delay; malformed/terminal output stops only detection. |
| No/slow/dead subscriber | Zero delivery is visible; independent queues isolate and unsubscribe releases backlog. |
| Critical lifecycle overflow | Admission failure is visible; indexed recording/current state remain reconciliation evidence. |
| Camera/LiveKit/PostgreSQL outage | Existing independent owners remain unchanged; reconnect resets detection. |
| Shutdown during work | Decoder response and fake inference are bounded; every owner is joined. |
| Two cameras/one terminal detector | Per-camera queues/tasks have no shared failure or cancellation channel. |

## External production gates

Not locally verified or claimed: accepted production model/version, labeled
dataset and accuracy targets, confidence policy, target accelerator/driver/Linux,
representative 4 MP cameras, tracking need, accepted freshness p90, production
model cancellation, and a 10-camera combined recording/live/detection soak.
