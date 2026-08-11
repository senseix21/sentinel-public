# Next Actions — 2026-08-10 19:22 +06

## Current position

Phase 0 has a passing deterministic RTSP reconnect/reset harness and a provisional timestamp contract. Production Rust packet types remain intentionally deferred.

## Next executable sequence

1. Build a bounded crash-finalization experiment comparing Matroska and fragmented MP4 for H.264 and H.265.
2. Record which partial files remain probeable/decodable and what metadata/finalization guarantees each container provides.
3. Update ADR 001 and ADR 004 with the chosen MVP container/finalization policy.
4. Obtain representative ONVIF Profile T camera streams and rerun packet/reconnect evidence.
5. Test H.264 publication against LiveKit.
6. Close remaining product objectives for playback, viewers, detection, analytics, reliability, hardware, and security.

After steps 1–3, the H.264 packet and segment lifecycle types can be designed with focused tests for Phase 2. Do not generalize those types into a broad media framework before real-camera evidence.

