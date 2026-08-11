# Decisions and Open Questions — 2026-08-10 23:06 +06

## Decisions

- Keep camera-specific RTP parsing upstream of the media distribution boundary.
- Distribute complete H.264 access units, not fragmented RTP payloads or lone
  slices.
- Maintain an integration test for dependency-boundary behavior independent of
  external media services.

## Open questions

- RTSP library/session ownership and blocking boundary.
- Async wake-up/cancellation and supervisor restart policy.
- Real fragmented-MP4 muxer adapter.
- Representative camera bounds and timestamp behavior.
