# Next Actions — 2026-08-10 19:40 +06

## Current position

Phase 0 has synthetic evidence for RTSP copy-remux, reconnect timestamp resets, crash-resilient container selection, webcam bridging, and direct H.264 LiveKit publication/subscription. Representative hardware and product decisions now dominate the remaining gate.

## Next executable sequence

1. Obtain a representative ONVIF Profile T H.264 High Profile camera or licensed capture and record non-secret SDP/caps, GOP, bitrate, PTS/DTS, disconnect, fragmented-MP4, and LiveKit/browser evidence.
2. Confirm production Linux host, CPU/GPU, RAM, disk/filesystem/layout, networking, and power-loss test method.
3. Confirm viewer concurrency/live latency, playback boundaries, detector model/GPU, heatmap semantics, reliability/data-loss, disk-full policy, and security identity.
4. Accept or revise ADR 001 using representative evidence.
5. Close Phase 0, then begin Phase 1's minimal compile/CI vertical slice without pre-creating unused abstractions.

If representative hardware remains unavailable, the next safe local task is a browser subscriber/latency harness or a narrow production-publisher API investigation. Neither substitutes for production-camera qualification.
