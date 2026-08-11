# Decisions and Open Questions — 2026-08-10 18:30 +06

## Accepted baseline

- 10 continuous cameras initially; measure 20 later.
- 4 MP, 15 FPS, H.264 High Profile, 4 Mbps planning bitrate, two-second GOP.
- RTSP/TCP and ONVIF Profile T.
- Wired PoE, NTP, digest/TLS-capable authentication.
- No MVP audio or PTZ.
- H.264 LiveKit compatibility path, 60-second segments, 30-day configurable retention, and 5 FPS detection sampling.

## Evidence-backed technical decisions

- MediaMTX 1.20.0 is the qualified local RTSP fixture server.
- Synthetic H.264/H.265 RTSP copy-mode remux is feasible.
- The Mac webcam is a usable manual RTSP source only with explicit yuv420p conversion.
- H.265 must remain experimental until timestamps, segment-relative normalization, and container behavior are resolved.

## Remaining gates

- Actual camera models and representative H.264/H.265 RTSP samples.
- Production Linux hardware and storage topology.
- Detection model/GPU/accuracy and latency.
- Heatmap meaning, playback accuracy, viewers/live latency, failure objectives, and security identity.
- LiveKit publisher compatibility test.
- Controlled reconnect/timestamp-reset RTSP fixture.

