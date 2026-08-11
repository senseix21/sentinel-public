# 2026 MVP Qualification Baseline

Status: accepted by product owner on 2026-08-10 as the initial qualification profile

This is the workload the first 10-camera MVP must target. It is not a claim that the current implementation or proposed hardware already supports the workload. Real camera measurements may revise bitrate, storage, and compute without changing the architecture.

## Camera and media profile

| Property | MVP baseline |
|---|---|
| Camera count | 10 initially; qualify 20 after optimization |
| Recording policy | Continuous |
| Resolution | 4 MP / approximately 1440p |
| Frame rate | 15 FPS |
| Required video codec | H.264 High Profile |
| Experimental video codec | H.265/HEVC |
| Planning bitrate | 4 Mbps average per camera; expected operating range 3–5 Mbps |
| Rate control | VBR with configured maximum |
| GOP/keyframe interval | 2 seconds, therefore 30 frames at 15 FPS |
| Camera transport | RTSP interleaved over TCP initially |
| Camera interoperability | ONVIF Profile T |
| Authentication | Digest authentication or TLS/HTTPS where camera support permits |
| Audio | Disabled for MVP unless later explicitly required |
| PTZ | Not required for the baseline; retain an optional capability boundary |
| Clock | NTP-synchronized cameras; UTC internally |
| Physical network | Wired Gigabit Ethernet with PoE |

ONVIF announced the deprecation of Profile S and recommends Profile T, which supports H.264/H.265, metadata, imaging controls, and stronger authentication options. Verify each purchased model in ONVIF's official conformant-products database:

- <https://www.onvif.org/profiles/profile-t/>
- <https://www.onvif.org/pressrelease/onvif-to-end-support-for-profile-s/>

## Pipeline defaults

| Property | MVP baseline |
|---|---|
| Segment duration | 60 seconds |
| Recording retention | 30 days, configurable |
| Detection sample rate | 5 FPS per camera |
| Internal event bus | Bounded in-process implementation |
| Live publication codec | H.264 for browser compatibility |
| Recording representation | Original encoded H.264 packets when container-compatible |
| Detection representation | Sampled decoded frames from one explicit decode boundary |

LiveKit documents H.264 as a supported video codec. Advanced codecs are not supported uniformly by browser clients, so H.265 must not become the MVP live-view dependency without explicit compatibility tests and a backup/transcode policy:

- <https://docs.livekit.io/transport/media/advanced/>
- <https://docs.livekit.io/client-sdk-js/interfaces/TrackPublishDefaults.html>

## Storage qualification target

At exactly 4 Mbps, continuous 30-day recording requires:

```text
10 cameras = 12.96 TB raw video payload
20 cameras = 25.92 TB raw video payload
```

Using a provisional 1.2× operational margin gives 15.55 TB and 31.10 TB respectively. RAID/redundancy, filesystem reserve, metadata, clips, logs, and replacement/rebuild behavior must be added when physical storage is selected.

The initial hardware planning floor is therefore:

- 10-camera deployment: at least 16 TB usable recording capacity.
- 20-camera deployment: at least 32 TB usable recording capacity.
- Separate SSD storage for OS, PostgreSQL, and temporary processing.

## Server qualification starting point

This is a procurement/test starting point, not a guaranteed requirement:

- Production Linux server.
- 12–16 modern CPU cores.
- 64 GB RAM.
- NVIDIA GPU with approximately 12–16 GB VRAM for initial detection qualification.
- Gigabit Ethernet minimum.
- UPS plus SMART/storage-health monitoring.

The selected detector, viewer concurrency, codec conversion requirements, and actual camera bitrate may raise or lower these requirements. Qualification must measure CPU, GPU, memory, disk, network, queue drops, recording gaps, and percentile latency.

## Camera procurement checklist

Require ONVIF Profile T conformance, RTSP/TCP, H.264 High Profile, configurable resolution/FPS/bitrate/GOP, PoE, NTP, supported firmware updates, credential rotation, and stable long-running timestamps. Prefer H.265, metadata/motion/tamper events, multiple stream profiles, secure boot/signed firmware, and edge SD storage where available.

## Reversal criteria

Revise this baseline when representative camera measurements, required scene detail, legal/audio requirements, detector accuracy, LiveKit compatibility, or validated hardware results demonstrate that a value is unsuitable. Record the change in an ADR or dated context snapshot; do not silently alter the qualification workload.

