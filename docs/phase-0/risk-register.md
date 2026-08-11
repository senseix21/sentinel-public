# Phase 0 Risk Register

| Risk | Impact | Probability | Evidence | Mitigation | Contingency / reversal criterion |
|---|---|---|---|---|---|
| Unknown camera codec/profile/GOP | High | High | No representative production stream | Obtain capability matrix and samples before packet API freeze | Restrict supported profiles explicitly |
| H.265 timestamp/segment behavior | High | High | Synthetic spike emitted negative PTS and non-exact boundaries | Test real RTSP timestamps, B-frame reordering, muxer flags, and discontinuity policy | Support H.264 first or use codec/container-specific segmenter |
| LiveKit codec/access-unit incompatibility forces transcode or drops | High | Medium | Single-slice webcam H.264 sustained 15 FPS; multi-slice input was mis-paced and startup-before-consumer caused RTSP drops | Production publisher parses timestamped access units, exposes queue/rate/drop metrics, and starts downstream-first; test representative codecs/browsers | Replace adapter, normalize packetization, allocate isolated transcode capacity, or restrict input |
| Storage estimate based on assumed bitrate | High | High | No production bitrate measurement | Measure VBR distribution and use sizing worksheet | Reduce retention/resolution/bitrate or provision tiered storage |
| Database outage loses recording index | High | Medium | Reconciliation contract not designed | Durable segment-finalization journal/reconciliation spike | Degraded local index with bounded recovery queue |
| Slow consumer causes unbounded memory | High | High | Prototype lacks distribution boundary | Define capacity/overflow per consumer and metrics | Shed detection/live work; surface recording failure |
| Camera reconnect duplicates sessions/tasks | High | Medium | Prototype has no lifecycle supervisor | Typed state machine plus cancellation/leak tests | Restart isolated camera pipeline |
| Disk exhaustion corrupts recent recording | High | Medium | No production disk policy | Watermarks, reservation, cleanup, alerts, forced failure tests | Stop non-critical consumers; explicit recording degraded state |
| Detection hardware/model insufficient | Medium/High | High | Production GPU/model unknown | Benchmark chosen model and sample rate | Lower sampling/model size or extract detector workers |
| Development webcam differs from IP cameras | Medium | High | Webcam requires AVFoundation/transcode bridge | Keep deterministic and real-camera RTSP qualification fixtures | Never use webcam success as interoperability evidence |
| Premature external bus/service extraction | Medium | Medium | 200-camera direction may invite overdesign | Modular monolith until measured bottleneck | ADR required for extraction |

Review this register at every Phase 0 experiment and phase gate. Probability is a current engineering judgement, not measured frequency.
