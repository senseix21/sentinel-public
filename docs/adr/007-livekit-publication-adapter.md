# ADR 007 — Owned LiveKit CLI encoded-publication adapter

Status: accepted for the Phase 5 local H.264 slice; production qualification pending

Date: 2026-08-12

## Context

The media plane already produces complete, timestamped `H264AccessUnit` values
from one RTSP connection per camera. Live publication must consume that bounded
fan-out without decoding pixels and must fail independently from recording.

The released official Rust client SDK (`livekit-api` 0.6.3 for token issuance;
the realtime SDK is maintained in `livekit/rust-sdks`) publishes ordinary raw
video tracks through an encoder. Direct pre-encoded video ingest remains
unreleased upstream work in pull request 1048 as checked on 2026-08-12. Depending
on that branch would make the surveillance workspace own an unpublished native
WebRTC fork and its upgrade policy.

The released LiveKit CLI accepts Annex-B H.264 through a bounded Unix socket and
has already been exercised by the Phase 0 spike. It creates a process boundary,
but avoids an additional RTSP connection and avoids pixel transcoding.

## Options

1. Pin the unreleased Rust SDK branch. This gives an in-process boundary but
   assumes maintenance of unpublished API and substantial native dependencies.
2. Implement a new in-process WebRTC/LiveKit client. This provides full RTP
   control but duplicates protocol, congestion-control, and interoperability
   work with the highest correctness and maintenance risk.
3. Own a released `lk` process per publishing camera through a private Unix
   socket. This is reversible and operationally explicit, at the cost of a
   process and one Annex-B assembly/copy per access unit.
4. Transcode to raw frames and use the released SDK. This violates the MVP
   pass-through requirement and spends CPU/GPU without evidence.

## Decision

Use option 3 for the initial local Phase 5 slice. The runtime requires an exact
configured `lk` version (development default `2.18.2`), starts it with secrets in
the child environment rather than arguments, owns its Unix socket, child,
cancellation, timeout, and join policy, and suppresses credential-bearing child
output. The adapter consumes the existing live subscription only.

The Rust boundary separately implements and tests deterministic 90 kHz RTP
timestamp conversion and bounded RFC 6184 single-NAL/FU-A packetization. The CLI
boundary receives one reconstructed Annex-B access unit at a time, paced from
media timestamps. It caches SPS/PPS, waits for a parameterized IDR after startup
or discontinuity, and never decodes pixels.

Every `PublicationAdapter` send now carries the Annex-B bytes, mapped 90 kHz RTP
timestamp, and publication epoch. An adapter must declare whether media or its
own clock is authoritative. The CLI declares `AdapterClock` because its released
socket protocol cannot accept caller timestamps. Health/API/metrics expose this
fact, so the CLI path cannot be mistaken for timestamp- or latency-qualified
publication. A deterministic retry test proves a failed session is disconnected
and the replacement session starts a fresh epoch and timestamp origin.

Costs per access unit are:

- existing immutable `Arc` NAL payloads remain shared by recorder and publisher;
- the CLI adapter allocates one bounded Annex-B buffer and copies NAL bytes once;
- the Unix socket copies into the kernel/process boundary;
- one owned `lk` child and one publication owner exist per enabled camera;
- no media is placed on the domain event bus and no second camera connection is
  opened.

The live distributor queue remains freshness-oriented. It is bounded in access
units by camera configuration and in bytes by the validated maximum access-unit
size. Network/child failures retry with bounded exponential backoff. Missing
credentials, adapter/version mismatch, and invalid configuration are terminal.
Recorder admission and lifecycle never await the adapter.

## Security

LiveKit URL, key, and secret enter through server environment configuration.
Secret-bearing types redact `Debug`; child arguments contain identities and the
socket path but no secret. Browser access uses `livekit-api` 0.6.3 to issue
short-lived, room-scoped, subscribe-only tokens. The endpoint remains available
only on the existing loopback development API until Phase 8 authentication and
authorization are accepted.

## Deployment and compatibility

The local adapter supports Unix-domain sockets and therefore macOS/Linux, not
Windows. Self-hosted LiveKit is the locally qualified topology. H.264 is the
only accepted live codec; H.265 and transcode fallbacks remain explicit product
gates. Room topology is one room and one deterministic publisher participant and
track name per camera.

## Failure model

The publication owner distinguishes connecting, publishing, degraded, backoff,
terminal, and stopping states. A child exit, socket failure, or server outage
affects only live publication. Queue eviction and lag drops are measured.
Credential/configuration/version failures stop retrying. Dropping the owner
kills the child and removes the socket as a final RAII safeguard.

## Reversal criteria

Replace the CLI boundary when a released official Rust SDK accepts complete
encoded access units with explicit timestamps, cancellation, keyframe feedback,
and bounded send feedback on supported deployment platforms. Also reconsider if
measurements show per-camera process or copy cost is material, CLI parsing cannot
correctly preserve representative multi-slice High Profile streams, or target
deployment does not support Unix sockets.

Do not remove the SDK-neutral domain, adaptation, ownership, health, or token
contracts when replacing the adapter.

The official encoded-ingest Rust SDK work was rechecked on 2026-08-12 and remains
open as `livekit/rust-sdks` pull request 1048. The released `lk` 2.18.2 help lists
Annex-B/length-prefixed socket input and an FPS hint, but no timestamped RTP input.

## Local acceptance evidence required

- deterministic single- and multi-slice parameterized H.264 reaches a decoding
  subscriber;
- timestamps remain access-unit based across nonzero origins and fresh epochs;
- LiveKit stop/restart and slow socket writes do not interrupt recording;
- reconnect leaves one participant/track and shutdown leaves no child/socket;
- queue, lag, copy, task/process, and latency/resource costs are recorded.

Representative cameras, browsers, viewer concurrency, target hardware/network,
TURN/TLS, and production secret rotation remain external qualification gates.
