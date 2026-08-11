# ADR 004: RTSP Client Adapter

Status: accepted for the deterministic H.264 ingest slice; production-camera qualification pending

Date: 2026-08-10

## Context

The MVP requires one authenticated RTSP/TCP session per camera, codec/capability
inspection, RTP timestamps and loss/discontinuity visibility, cancellation, and
clean ownership of encoded media. The prototype proved GStreamer and FFmpeg
mechanics but opened independent sessions and did not provide the new lifecycle
or media-distribution boundaries.

The application has adopted Tokio. The initial qualified media profile is H.264
High Profile over RTSP interleaved TCP; H.265 remains a qualification gate.

## Options

### Retina 0.4.x

Pros:

- Rust/Tokio-native RTSP client and stream API.
- Basic and digest authentication.
- RTP over interleaved TCP.
- H.264 and H.265 depacketization, multiple H.264 slices, and RTCP sender-report
  timestamp handling.
- Explicit typed session progression (`DESCRIBE`, `SETUP`, `PLAY`) and production
  use in Moonfire NVR, including workarounds for surveillance-camera behavior.
- No GStreamer/FFmpeg system-library boundary in the camera connection owner.

Cons/risks:

- The project explicitly reports many missing features, an API that is not yet
  clean/stable, incomplete programmatic error classification, and uneven test
  coverage.
- Its demuxed output contract must be mapped carefully to this repository's
  access-unit, timestamp, loss, and connection-epoch contracts.
- Representative camera behavior remains untested.

### GStreamer `rtspsrc` + `appsink`

Pros:

- Mature media ecosystem and proven prototype mechanics.
- Broad codec/camera interoperability and plugin-based probing/remuxing.

Cons/risks:

- Native system dependencies and GLib/GStreamer task-context integration.
- More complex cancellation, bus, appsink, and ownership boundaries.
- The prototype already demonstrated a stalled bus loop and accidental
  connection coupling; adopting it requires a dedicated-thread/FFI design and
  characterization tests.

### Supervised FFmpeg child process

Pros:

- Broad interoperability and familiar operational diagnostics.
- Existing copy-mode recording spikes are reusable.

Cons/risks:

- A child-process stdout protocol does not naturally expose RTSP/RTP loss,
  capability, session, and complete-access-unit semantics.
- Process supervision and parsing create another buffering/failure boundary.
- It is better retained as a muxing/probing spike tool than made the camera
  lifecycle owner without evidence.

## Decision

Use Retina `0.4.19` as the provisional adapter for the next H.264 RTSP/TCP
ingestion spike. Do not expose Retina types outside `camera::rtsp`, and do not
add the dependency until the adapter slice includes a deterministic RTSP test.

The stable camera lifecycle remains adapter-independent and owns the Retina
playing session only through its generic connected-state parameter. Retina does
not own reconnect policy, recording, media distribution, configuration, or
domain health state.

Retina 0.4.19 requires Rust 1.88, so the workspace MSRV is 1.88. Default Retina
features are disabled because the accepted slice is deliberately H.264-only;
H.265 is not implied by dependency availability.

The spike must prove:

1. Digest/basic credentials are supplied separately from a redacted URL.
2. Exactly one interleaved-TCP upstream session is created.
3. H.264 capabilities and stream selection are validated before `PLAY`.
4. Complete access units, timestamps/timebase, keyframe/configuration state, and
   discontinuities map without fabricated values.
5. Cancellation drops/tears down the session and returns lifecycle ownership.
6. EOF, timeout, authentication, unsupported-media, and protocol errors map to
   the typed retryable/terminal categories.
7. The deterministic MediaMTX reconnect fixture passes through the Rust client.

## Validation result

On 2026-08-11, the repository-owned MediaMTX integration fixture proved items
1–6 for a synthetic H.264 High Profile source: separate credentials, TCP-only
transport, 640x360/RFC 6381 capability inspection, bounded complete access
units, monotonic 90 kHz timestamps, keyframe agreement, explicit packet-loss
outcomes, terminal 401 classification, and bounded teardown. Run it with:

```bash
scripts/phase0/retina-rtsp-fixture.sh
```

RTSP 404 is classified as retryable `ServerUnavailable`, not terminal invalid
configuration. MediaMTX uses 404 while a configured path's publisher is absent,
and the protocol status alone cannot distinguish that transient condition from
a misspelled path. Configuration validation and an observable attempt policy
must surface a permanently invalid path without breaking ordinary recovery.

Item 7 passes in the same fixture: it interrupts the publisher for 750 ms and
restarts it without replacing the Rust supervisor. The final lifecycle evidence
contains at least two connected epochs and one reconnect, and media resumes
before orderly cancellation. Representative physical cameras remain an
explicit qualification gate.

## Primary evidence

- Retina project status and feature matrix:
  <https://github.com/scottlamb/retina>
- Retina client/session lifecycle and Tokio runtime requirements:
  <https://docs.rs/retina/0.4.19/retina/client/struct.Session.html>
- Retina demuxed stream contract:
  <https://docs.rs/retina/0.4.19/retina/client/struct.Demuxed.html>
- Current GStreamer Rust bindings and native installation requirements:
  <https://gstreamer.freedesktop.org/documentation/rust/stable/latest/docs/gstreamer/>

## Reversal criteria

Use GStreamer behind the same camera contract if Retina fails a representative
camera, cannot expose required loss/timestamp/capability information, or its API
maintenance risk exceeds the cost of the native integration boundary. Use a
supervised FFmpeg path only if both library approaches fail qualification and an
explicit child-process media protocol is specified and measured.
