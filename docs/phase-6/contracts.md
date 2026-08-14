# Phase 6 Detection and Event Contracts

Accepted locally: 2026-08-12

## Ownership and topology

Each camera pipeline retains one RTSP owner and three independent encoded-media
queues: recorder preserves backlog and rejects visibly, LiveKit replaces stale
items, and detection replaces stale items. Detection-disabled cameras drop the
detection subscription and start no decoder/model owner.

`DetectionOwner` owns timestamp sampling, one dedicated FFmpeg decoder worker
and child process, blocking inference jobs, event publication, cancellation,
and join. FFmpeg waits for an IDR containing SPS/PPS after start, reconnect,
backward time, or timebase change. Its stdout reader has capacity one; the media
queue and decoded RGB allocations are independently bounded. Blocking pipe,
decoder, and inference work never runs on a Tokio worker.

Rust retains at most two configured RGB24 allocations (reader handoff and
in-flight frame), plus bounded encoded queues and OS/FFmpeg implementation
memory. The latter is measured during qualification and is not described as
zero-copy or a hard OS memory limit.

## Sampling

Sampling is evaluated after continuous decode because arbitrary H.264 P/B
pictures are not independently decodable. It uses `(connection epoch, ticks,
timebase)` and the accepted reversible 5 FPS maximum. Duplicate timestamps are
skipped. Reconnect epoch, backward time, timebase change, or a gap above the
configured maximum resets state. Rate-skipped frames do not enter inference.

## Detection facts

Only `person` observations exist in Phase 6. Confidence is finite in `0..=1`.
Boxes use normalized image coordinates with a top-left origin, positive x to
the right and y downward; width/height are positive and the complete box must
remain within `[0,1]`. Each event has at most 64 boxes and identifies camera,
connection epoch, media timebase/ticks, UTC observation time, model name, and
model version. Tracking identifiers are intentionally absent.

The deterministic fake model (`deterministic-fake-person` version `1.0.0`) is
local test infrastructure: an odd first RGB byte yields one fixed person and an
even byte yields none. It makes no accuracy or capacity claim.

## Event delivery

ADR 002 is normative. Delivery is bounded, subscriber-independent, at-most-once
within one process lifetime, and ordered only within accepted publications from
one producer. Critical overflow is visible; detection and latest state favor
freshness; telemetry drops new overflow. Closing the bus wakes subscribers and
allows accepted backlog to drain. Media/file/credential/binary types are absent
from the closed public event payload enum.

## Latency clocks

Decode and inference duration totals use monotonic `Instant`. End-to-event
latency begins after receiving an encoded access unit from the detection queue
and ends after event-bus publication. Fixed non-cumulative buckets are `0–10`,
`10–50`, `50–100`, `100–250`, `250–500`, `500–1000`, and `>1000` milliseconds.
No fabricated UTC mapping is added to source media time.
