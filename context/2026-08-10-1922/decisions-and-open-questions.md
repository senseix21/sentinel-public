# Decisions and Open Questions — 2026-08-10 19:22 +06

## Evidence-backed decisions

- Unknown PTS/DTS values are optional, not zero.
- Integer timestamp ticks always carry/reference a timebase.
- Reconnect or incompatible caps change begins a new connection epoch.
- Monotonic observation time drives duration and timeout policy.
- UTC correlates recordings to operator time but does not replace codec media time.
- Segment-relative normalization preserves the original source/discontinuity for diagnostics.
- The recorder does not append a reset epoch blindly to an existing segment.

## Remaining technical gates

- Matroska versus fragmented MP4 behavior during normal and interrupted finalization.
- Representative production H.264/H.265 RTSP sources.
- Ingestion-library RTP wrap/loss/caps behavior.
- Rust-client recovery while a subscription is active across publisher stop/start.
- LiveKit RTP/codec configuration requirements.
- Audio/video time correlation if audio enters scope.

Product/hardware/analytics/security decisions from the previous snapshot remain open.

