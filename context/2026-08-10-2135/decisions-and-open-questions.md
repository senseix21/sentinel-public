# Decisions and Open Questions — 2026-08-10 21:35 +06

## Decisions

- Physical webcam H.264 format compatibility with LiveKit is demonstrated.
- The CLI/socket path is not qualified for real-time publication.
- Subscription success alone is insufficient; throughput and lag are mandatory acceptance measurements.

## Open questions

- Does the CLI parser, Unix socket, FFmpeg muxer, or another boundary cause the observed throttle?
- Which Rust packet-publishing API provides explicit timestamps, pacing, keyframes, cancellation, and metrics?
- Can a representative H.264 High Profile camera render in target browsers without transcoding?
- What viewer concurrency and latency targets define success?
