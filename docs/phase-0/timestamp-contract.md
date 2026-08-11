# Media Timestamp and Discontinuity Contract

Status: provisional contract for the H.264 ingest-to-record vertical slice

Last updated: 2026-08-10

## Problem

RTSP/RTP media time is not wall-clock time and does not necessarily begin at zero when a subscriber connects. A publisher can reset timestamps after reconnect, codecs can reorder decode/presentation order, and a reader may not receive decodable media until a later random-access point.

The deterministic reconnect fixture observed two fresh H.264 publisher generations. Each reader's first packet was a keyframe at PTS/DTS 1.867 seconds, followed by 66 ms packet durations. Both generations reset to the same source values. A nominal FFmpeg `-t 4` copy captured only 32 packets through PTS 3.933 rather than 60 packets/four elapsed seconds because the stop boundary was evaluated against the non-zero media timeline.

Therefore no single timestamp can represent source ordering, segment duration, reconnect continuity, and user-facing recording time.

## Required clocks

### Source media time

Preserve the source PTS, DTS, duration, and timebase exactly when provided. These values order decoding/presentation within one codec configuration and connection epoch. They may be absent, negative, reordered, non-zero at subscription, discontinuous, or reset after reconnect.

### Connection epoch

Assign a monotonically increasing epoch within a camera pipeline whenever a new upstream media session is established or the negotiated stream configuration changes incompatibly. Source timestamps are comparable only within a compatible epoch unless continuity is explicitly proven.

### Observation time

Capture a monotonic process timestamp when the packet crosses the ingestion boundary. Use monotonic elapsed time for timeouts, queue lag, health, and segment-duration policy. Never compute elapsed duration from wall-clock UTC because wall time can jump.

### Recording wall time

Associate the stream/segment with UTC derived from the system clock synchronized by NTP. This time supports camera/date queries and operator display. Record clock uncertainty or discontinuity when known. UTC does not replace PTS/DTS for muxing.

### Segment-relative media time

At a valid random-access boundary, choose an epoch-local origin and normalize output container timestamps relative to that origin according to codec/container rules. The original source timestamps remain available for diagnostics. Segment rotation is triggered by monotonic elapsed policy and completed at a safe media boundary; it is not triggered by `source_pts >= configured_duration`.

## Provisional encoded packet fields

The future Rust representation must be capable of carrying:

- Camera and logical track identity.
- Connection epoch and codec-configuration generation.
- Codec/caps information or a stable reference to it.
- Immutable encoded payload.
- Source PTS and DTS as optional integer ticks plus explicit timebase.
- Optional source duration.
- Random-access/keyframe classification.
- Monotonic observation instant or an ingestion sequence tied to one.
- UTC observation/correlation time at the pipeline boundary.
- Explicit discontinuity flags/reason.

Do not expose raw integer timestamps without their timebase. Do not represent unknown timestamps as zero. Avoid forcing camera time, RTP time, system UTC, and segment-relative time into one `Timestamp` newtype.

## Reconnect behavior

1. Connection loss closes the current epoch and emits a camera/media discontinuity fact.
2. The recorder finalizes the current segment if it remains valid; otherwise it marks it incomplete for reconciliation.
3. Reconnection creates a new epoch even if source PTS resembles the previous epoch.
4. Downstream decoding/recording waits for codec configuration and a valid random-access point where required.
5. The recorder starts a new segment after reconnect; it does not append reset timestamps blindly to the old segment.
6. Recording metadata preserves the wall-clock gap between epochs.
7. Detection drops undecodable/stale frames and resumes from fresh decoded output.

## Invariants

- PTS/DTS arithmetic is checked integer/rational arithmetic; avoid floating-point decisions in the media path.
- Within an epoch, decode ordering follows DTS when present and presentation follows PTS when present.
- Monotonic observation time never crosses process restart; UTC and persisted metadata bridge restart boundaries.
- A segment cannot be marked complete without a defined wall-clock interval, durable path, codec metadata, and successful container finalization.
- A timestamp reset cannot be hidden by adding an arbitrary constant without recording the discontinuity/epoch.

## Evidence still required

- Representative H.264/H.265 cameras, including B-frames, timestamp resets, and caps changes.
- RTP timestamp wrap and packet loss behavior in the selected ingestion library.
- Representative-camera fragmented MP4 normalization and power-loss behavior; synthetic `SIGKILL` evidence is complete.
- LiveKit's required RTP timestamp and codec-configuration behavior.
- Audio/video synchronization if audio is later introduced.
