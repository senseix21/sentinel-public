# Decisions and Open Questions — 2026-08-11 16:00 +06

## Accepted decisions

- One-second internal fMP4 fragments are distinct from 30–60 second file
  rotation and are a reversible default.
- Every complete muxer emission is synced. This bounds crash loss to the active
  muxer fragment at the cost of higher sync frequency; Phase 9 measurements may
  justify a different durability interval.
- Phase 2 operational HTTP binds only to loopback. Authentication and a trusted
  external API boundary belong to later security/API phases.
- Phase 2 supports only the qualified H.264 subset. Unsupported codecs and caps
  fail visibly instead of receiving guessed handling.

## Open gates

- Representative camera profiles, especially B-frame/DTS behavior.
- Confirmed production resolution, FPS, bitrate, audio, and codec matrix.
- Measured CPU, memory, disk latency, and sync cost on target hardware.
- Phase 3 multi-camera ownership, failure isolation, and camera-scoped API.
