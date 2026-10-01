# ADR 013: Remove Media Proxy Code and Fixtures

Status: amended 2026-09-05; retirement remains accepted, deletion order changed

Date: 2026-09-01

## Context

The production workspace already connects each physical camera directly through
one Retina-owned RTSP/TCP session. A separate RTSP proxy remained only in the
uncompiled prototype, local webcam launcher, synthetic integration fixtures, and
load/browser harnesses. The product owner directed that all code for that proxy
be removed.

Architecture review and a successful 2026-09-04 baseline run subsequently
established that the proxy-backed assets also carry reconnect, multi-camera,
browser, LiveKit, and load-test behavior. Removing those behaviors before their
replacement is proven would discard evidence rather than merely remove a
dependency.

## Decision

Replace the local webcam bridge with a server-owned FFmpeg H.264 source, then
remove the legacy process manager, proxy configuration, Make targets, and shell
harness dependencies. Keep the production Retina ingestion path and the
physical-camera qualification harness.

Follow the replacement matrix and gates in
`docs/ffmpeg-webcam-source/SPEC.md` and `PLAN.md`. A fixture is removed only
after equivalent behavior is executable or its loss is separately accepted.

Historical evidence documents remain historical records; they are not runnable
instructions. Current documentation must not claim that the removed fixtures
are available.

## Consequences

- The shipped production topology is unchanged: camera to Retina ingestion to
  bounded consumers, with LiveKit downstream of the distributor.
- The target repository has a one-command webcam stack without a MediaMTX
  process or local RTSP hop.
- An external RTSP test peer remains necessary for Retina authentication,
  protocol, and reconnect qualification; a webcam source is not a substitute.
- MediaMTX-dependent fixtures may remain temporarily while their individual
  behaviors are replaced. They are not production dependencies or the desired
  completion state.
- Physical-camera qualification now carries more integration risk and remains
  an external gate.

## Reversal criteria

Add or retain a replacement fixture only through a decision that names its ownership,
process lifetime, network exposure, codec behavior, queueing, deterministic
failure controls, and resource-accounting rules. It must not become a production
media hop or create a second upstream camera connection.

Reconsider the FFmpeg child boundary if no bounded framing option preserves the
timestamp/access-unit contracts, or target measurements show that its resource
or failure behavior cannot meet recording-preservation objectives.
