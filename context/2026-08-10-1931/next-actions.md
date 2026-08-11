# Next Actions — 2026-08-10 19:31 +06

## Current position

Phase 0 now has provisional timestamp/reconnect semantics and an accepted H.264 container/finalization lifecycle. Synthetic risk spikes are no longer the primary blocker.

## Next executable sequence

1. Obtain or select a representative ONVIF Profile T H.264 camera and capture non-secret SDP/caps, GOP, bitrate, PTS/DTS, disconnect, and fragmented-MP4 stream-copy evidence.
2. Test H.264 publication against a local/self-hosted LiveKit deployment and document packet versus transcode requirements.
3. Confirm production Linux/storage hardware and run filesystem sync/restart/power-loss qualification.
4. Confirm playback accuracy/client requirements, viewer concurrency/live latency, detector model/GPU, heatmap semantics, reliability objectives, and security identity.
5. Accept or revise ADR 001 after camera and LiveKit evidence.

If representative hardware cannot be obtained immediately, the next reversible engineering task is a LiveKit H.264 integration spike. Do not claim Phase 0 completion without representative camera evidence or an explicitly accepted scope restriction to H.264 synthetic/webcam sources.

