# Decisions and open questions — Phase 5 checkpoint

Accepted locally:

- Use pinned `lk` 2.18.2 as the first owned encoded H.264 process adapter rather
  than an unreleased official Rust SDK branch.
- Use `livekit-api` 0.6.3 only for short-lived subscribe-only token issuance.
- Use one deterministic room/publisher/track identity set per camera.
- Keep viewer access on the loopback development API until Phase 8 security.

Open/blocking:

- Can the CLI socket path pass a sustained real-time multi-slice decode and
  stable clock test, or must ADR 007 reverse to another encoded adapter?
- How will real LiveKit stop/restart be deterministically owned and verified?
- What named latency boundary, percentile objective, and viewer workload does
  the product require?
- Representative camera/browser/device compatibility, target concurrency,
  target hardware/network, TURN/TLS, deployment ownership, and secret rotation.
- H.265 and any allowed isolated transcode fallback.
