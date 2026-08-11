# RTP/H.264 Depayloading Contract

Date: 2026-08-10

## Scope

The first H.264 ingestion slice supports RFC 6184 non-interleaved mode:

- single NAL unit packets, types 1–23;
- STAP-A aggregation, type 24;
- FU-A fragmentation, type 28.

STAP-B, MTAP16/24, FU-B, and interleaved/decode-order buffering are rejected. Add them only if representative SDP/camera evidence requires them.

Normative reference: <https://datatracker.ietf.org/doc/html/rfc6184>

## Ownership

`surveillance-camera::h264_depay` owns RTP payload interpretation, FU sequence continuity, aggregation parsing, and transport marker propagation.

`surveillance-media::packet` owns validated immutable NAL units and complete H.264 access units. It does not know about RTP fragmentation headers.

## Bounds

`H264DepayloaderLimits` requires nonzero values for:

- maximum reconstructed NAL size;
- maximum NAL count in one aggregation packet.

There are intentionally no production defaults yet. Representative 4 MP camera evidence must select conservative values, and configuration validation must account for RTP payload size, encoder slice behavior, and worst-case IDR frames. Every oversize/count rejection becomes an observable camera degradation metric.

## Failure behavior

- Empty, forbidden-bit, unspecified, malformed, and unsupported payloads return typed errors.
- STAP-A rejects missing/zero/truncated lengths and excess NAL count.
- FU-A requires a start, ordered wrapping sequence numbers, stable timestamp/header, nonempty fragments, and an end before RTP marker.
- A sequence gap, timestamp/header change, unrelated packet, malformed FU, or explicit discontinuity discards partial fragment state.
- Corrupt/incomplete fragments never become `H264NalUnit` or `H264AccessUnit` values.

## Tests

Camera-crate tests cover:

- single NAL and marker propagation;
- STAP-A ordering and last-NAL marker propagation;
- FU-A reconstruction across `u16` sequence wrap;
- missing packet and state clearing;
- unrelated-packet interruption and early marker;
- truncated, invalid, and oversized payloads;
- STAP-A SPS/PPS plus FU-A IDR into one completed keyframe access unit.

This is a deterministic contract test, not evidence that a specific camera's RTSP stack conforms or that configured production limits are sufficient.
