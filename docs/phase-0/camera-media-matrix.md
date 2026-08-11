# Camera and Media Capability Matrix

Status: awaiting representative production cameras

Do not fill unknown fields with generic camera specifications. Capture values from configuration, SDP/caps, and measured streams.

| Source | Role | Resolution | FPS | Codec/profile | GOP | Avg/peak bitrate | Audio | RTSP transport | ONVIF/PTZ | Evidence |
|---|---|---:|---:|---|---:|---:|---|---|---|---|
| Mac webcam bridge | Manual development | 1280×720 tested | ~15 | H.264 Constrained Baseline/yuv420p | 30 frames tested | ~1.78 Mbps from 10s remux | None | Local RTSP/TCP | N/A | 2026-08-10 AVFoundation → FFmpeg → MediaMTX 1.20.0 → copy remux passed |
| Synthetic H.264 | File-backed spike | 320×180 | 10 | H.264/libx264 | 300 frames | ~0.31 Mbps from output size | None | Not RTSP | N/A | 2026-08-10 segmentation spike |
| Synthetic H.265 | File-backed spike | 320×180 | 5 | HEVC/libx265 | 300 frames | ~0.12 Mbps from output size | None | Not RTSP | N/A | 2026-08-10 segmentation spike; negative-PTS warnings |
| Synthetic live H.264 | RTSP mechanics | 640×360 | 15 | H.264/libx264 | 30 frames | Not a capacity input | None | RTSP/TCP | N/A | 12.000s copy remux passed through MediaMTX 1.20.0 |
| Synthetic live H.265 | RTSP mechanics | 640×360 | 15 | HEVC/libx265 | 30 frames | Not a capacity input | None | RTSP/TCP | N/A | Copy remux/decode passed; client start offset observed |
| Production camera A | Qualification | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD | Required |
| Production camera B | Codec/profile variant | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD | Required |

## Qualification rules

- Include at least one representative source for every codec/profile/resolution combination to be supported.
- Record actual bitrate distribution over a realistic period; do not infer it only from resolution.
- Capture keyframe interval because segment boundaries and copy-mode seeking depend on it.
- Record audio codec separately from video codec.
- Test authentication, disconnect, reconnect, timestamp discontinuity, malformed media, and credential rotation.
- A synthetic source validates mechanics, not camera interoperability or production capacity.
