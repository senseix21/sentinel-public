# Phase 5 local evidence — 2026-08-12

## Implemented

- `surveillance-livekit` now owns validated identities, secret-redacted endpoint
  and credentials, lifecycle/failure contracts, bounded retry settings, H.264
  parameter-set handling, 90 kHz RTP conversion tests, bounded RFC 6184
  packetization, the owned `lk` process adapter, publication health/metrics, and
  subscribe-only viewer-token issuance.
- `CameraPipeline` retains the existing live freshness subscription when
  publication is enabled. The publisher is optional and supervised inside the
  pipeline lifetime; its exit does not terminate the recorder.
- The loopback API exposes credential-free publication state/metrics and a
  visibly development-only camera-scoped viewer-token endpoint.
- ADR 007 pins `lk` 2.18.2 and documents one bounded Annex-B allocation/copy and
  one child per enabled camera. `livekit-api` 0.6.3 is pinned for token issuance.

## Reproducible evidence

`scripts/phase5/livekit-fixture.sh` ran self-hosted `livekit-server` 1.13.5,
`lk` 2.18.2, the repository MediaMTX/RTSP fixture, the real surveillance server,
and an independent subscriber. The run observed a camera-scoped H.264 publisher
and track, attached the subscriber, produced and decoded three recording
segments, preserved one RTSP ingest, and left no owned publisher process after
shutdown. The first run found and fixed a macOS Unix-socket path-length defect.

`scripts/phase5/outage-fixture.sh` points the real process adapter at an absent
server, publishes into the existing distributor, and proves recorder queue
acceptance remains successful while the live owner attempts connection and
shuts down without publishing media.

Focused tests cover validated configuration and redaction, identity collision
prevention, lifecycle transitions, backoff bounds, single-NAL/FU-A/multi-slice
packetization, parameterized-keyframe waits, nonzero timestamps, epoch reset,
subscribe-only grants, and requested publication shutdown.

The adapter contract now requires every send to carry a mapped 90 kHz timestamp
and publication epoch and to declare its timestamp authority. A deterministic
retry test forces the first send to fail and observes `(epoch 1, timestamp 0)`
followed by `(epoch 2, timestamp 0)`, with two owned connects/disconnects and one
reconnect. The CLI explicitly reports `adapter_clock`; API and Prometheus expose
that limitation. Both Phase 5 fixtures passed again after this change.

## Evidence that blocks a local completion claim

The released CLI socket adapter does not accept per-access-unit timestamps. It
parses Annex-B and generates its own RTP clock. The real fixture produced
LiveKit sender-report clock-skew/out-of-order warnings, particularly when the
existing accelerated RTSP fixture was used. The Rust 90 kHz mapping contract is
tested, but it is not the timestamp source used by this CLI adapter. Therefore:

- no p50/p90/p99 publication or glass-to-glass latency is accepted;
- full subscriber decoding over a sustained, real-time, multi-slice fixture is
  not yet proven by a retained subscriber artifact;
- real LiveKit stop/restart, one-track recovery, and duplicate cleanup are not
  yet deterministically orchestrated;
- the CLI adapter remains a reversible local implementation, not a qualified
  production boundary.

These are correctness/qualification gaps, not documentation-only tasks. ADR 007
requires replacement by a released timestamp-aware encoded SDK path, or a
separately justified maintained adapter, if the CLI cannot pass them.

## External gates

Representative 4 MP H.264 High Profile cameras, supported browsers/devices,
agreed viewer concurrency and latency, target host/network, TURN/TLS, production
LiveKit deployment, production authentication/authorization, and secret rotation
remain unavailable or undecided and are not claimed.
