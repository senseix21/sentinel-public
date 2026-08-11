# Next Actions — 2026-08-10 17:39 +06

## Current position

Roadmap Phase 0 is in progress. Requirements and risk documents now exist, and the first synthetic codec/segmentation spike is recorded. Phase 0 exit gates are not met.

## Next executable sequence

1. Obtain Erik's answers to the decision list in `docs/requirements.md`, starting with representative camera codec/resolution/FPS/bitrate/GOP and production server hardware.
2. Obtain representative H.264 and H.265 RTSP sample streams without placing credentials in the repository.
3. Establish a local RTSP fixture server through an approved installation path; MediaMTX is not currently on PATH.
4. Investigate H.265 PTS/DTS with packet-level ffprobe output and compare Matroska versus fragmented MP4/remux behavior.
5. Run webcam -> AVFoundation -> FFmpeg -> local RTSP after server setup and macOS permission approval.
6. Qualify GStreamer independently; do not infer it works from executable presence.
7. Use the resulting evidence to accept or revise ADR 001 and select only the dependencies needed by the first ingest-to-record slice.

## Phase gate warning

Do not begin a production packet API or claim Phase 0 completion before representative RTSP timestamps and product topology requirements are resolved or explicitly accepted as reversible defaults.

