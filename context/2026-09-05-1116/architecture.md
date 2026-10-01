# Architecture — 2026-09-05 11:16 +06

## Accepted boundary

```text
RTSP camera -> Retina session ---------.
                                       v
Webcam -> owned FFmpeg -> Annex-B -> H264 source session
                                       |
                                       v
                         bounded media distributor
                           |-> recorder
                           |-> detector -> event bus
                           `-> LiveKit -> WebRTC viewers
```

There is one selected source owner per camera. FFmpeg is an implementation
detail of the webcam source, not a gateway or second media backbone. LiveKit
remains a downstream interactive-viewing consumer and does not own canonical
ingestion or recording.

The encoded media contract remains `H264AccessUnit`. Source neutrality was
needed now; speculative codec/raw-frame neutrality was not. A fresh managed
FFmpeg process creates a fresh camera epoch. Its fixed frame cadence is mapped
to the existing 90 kHz H.264 clock before distribution.

See `docs/ffmpeg-webcam-source/SPEC.md`, `PLAN.md`, and ADR 013.
