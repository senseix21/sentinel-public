# Next Actions — 2026-08-10 18:30 +06

## Current position

Phase 0 remains in progress, but the initial media profile, synthetic RTSP copy path, and Mac webcam bridge are now established. The next work should retire timestamp and reconnect risks rather than add production abstractions prematurely.

## Next executable sequence

1. Run MediaMTX from a temporary working directory to prevent generated certificates in the repository.
2. Create a deterministic RTSP fixture harness with controlled publish stop/start, mid-GOP join, and timestamp reset.
3. Capture packet-level PTS/DTS/keyframe evidence for H.264 and H.265 and compare segment-relative normalization policies.
4. Compare Matroska and fragmented MP4 for supported codec/crash-finalization behavior.
5. Obtain representative production camera streams and actual ONVIF model declarations.
6. Confirm production hardware, playback/live-view objectives, detection model, analytics meaning, reliability, and security decisions.
7. Test the chosen H.264 publication path against LiveKit before accepting ADR 001.

Do not begin the production media packet API until steps 2–4 define its timestamp, discontinuity, and cancellation semantics.

