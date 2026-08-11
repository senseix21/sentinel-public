# Decisions and Open Questions — 2026-08-10 22:05 +06

## Decisions

- Implement only RFC 6184 packetization-mode 1 forms needed by the H.264 MVP.
- Require explicit allocation limits; do not publish unmeasured production defaults.
- Treat RTP marker as an access-unit boundary only after aggregation/fragment reconstruction.
- Drop incomplete FU state rather than emit damaged encoded media.

## Open questions

- Representative maximum NAL/access-unit sizes and STAP counts.
- RTP reordering tolerance versus immediate gap failure.
- Packet loss and jitter measurements exposed by the eventual RTSP library.
- Whether SDP parameter sets must seed the access-unit parameter cache.
