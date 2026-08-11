# Next Actions — 2026-08-10 15:33 +06

## Current position

The workspace portion of Roadmap Phase 1 is compile-safe. Phase 1 is not complete: CI, dependency/system prerequisite documentation, and prototype characterization remain. Roadmap Phase 0 requirements are also unresolved.

## Next executable work

1. Confirm representative camera codec, resolution, FPS, bitrate, audio, and RTSP transport plus target server hardware.
2. Characterize reusable GStreamer, FFmpeg, and H.265 prototype mechanics with focused tests or standalone spikes.
3. Decide and document encoded packet ownership, timestamps/timebase, cancellation, and bounded subscriber behavior in ADR 001.
4. Add only the dependencies required by the one-camera ingest-to-segment vertical slice.
5. Implement validated `CameraId`, codec/time types, and legal camera connection-state transitions with tests.
6. Establish deterministic RTSP fixtures and the manual Mac webcam -> FFmpeg -> MediaMTX test path.

## Do not infer

- The scaffold does not record or ingest video.
- Passing workspace checks do not qualify any camera count.
- Empty traits or broad abstractions should not be added merely to fill module files.
- The worker is not an authorized distributed architecture; it is a requested structural placeholder.

