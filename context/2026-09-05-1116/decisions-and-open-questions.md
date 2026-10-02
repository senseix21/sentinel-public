# Decisions and Open Questions — 2026-09-05 11:16 +06

## Decisions

- Remove MediaMTX from current production and local-webcam workflows.
- Keep direct Retina RTSP for IP cameras and managed FFmpeg capture for local
  webcams behind one exhaustive session enum.
- Require FFmpeg to emit H.264 Annex-B with AUDs, repeated parameter sets,
  baseline profile, no B frames, and a fixed configured cadence.
- Bound parser bytes and access-unit bytes separately and include both in the
  checked per-camera memory budget.
- Start a camera epoch only after receiving a parameterized keyframe.
- Stop the Rust server before LiveKit in the launcher, allowing the recorder to
  finalize its active segment.

## Open questions and external gates

- Which repository-owned RTSP peer will replace deleted MediaMTX transport,
  authentication, and reconnect fixtures?
- Do Linux and Windows deployments need webcam capture as a product feature or
  only developer smoke support?
- Should source selection become part of the persisted camera API, or remain a
  bootstrap-only local-development option until a product workflow requires it?
- Production camera codecs/profiles, viewer concurrency, latency objectives,
  hardware, and deployment model remain unconfirmed.
