# Phase 5 native-publication local evidence — 2026-08-16

Contract: `docs/phase-5/SPEC.md`. Plan: `docs/phase-5/PLAN.md`. Decision:
`docs/adr/011-native-encoded-livekit-publication.md`, in particular Decision 5.

This file records one thing above all: **the native encoded publication path is
implemented and cannot run on this machine.** Everything below separates what was
measured from what was assumed, and names each item that remains unmeasured.

## What was implemented

- `crates/livekit/src/native.rs` publishes encoded H.264 access units in process
  through `livekit` 0.8.3 — `NativeVideoSource::new_encoded(VideoResolution)` and
  `capture_encoded_frame` — with no pixel transcode. The `lk` child-process
  boundary is removed rather than left in parallel (P5N.3).
- The same file refuses to publish on Apple targets, before the session is
  created, returning `AdapterErrorKind::Unavailable`. The refusal is not a
  capability probe: `VideoEncoderBackend::list_available()` reports `PreEncoded`
  unconditionally and is a static enumeration, not a check.
- `crates/livekit/src/owner.rs` gained `SessionExit::Terminal`. The send path had
  been discarding `AdapterError::kind()` and reporting every send failure as
  `Transport`, which is retryable; an adapter that can never publish therefore
  reconnected on every backoff interval and reported itself merely `Degraded`.
- `crates/livekit/src/domain.rs` gained the `Publishing → Terminal` lifecycle
  edge. It did not exist while the only non-retryable failures were raised at
  connect time, and its absence logged `ERROR invalid publication lifecycle
  transition` once the send path could raise one.
- `apps/server/build.rs` passes `-ObjC` on Apple targets. Without it the linker
  drops libwebrtc archive members that define only Objective-C categories, and
  the process dies on an uncatchable `NSException` for
  `+[NSString stringForAbslStringView:]`.
- `scripts/dev/local-stack.sh` refuses to start when any of its four ports is
  already held. See "Two failures that were not what they looked like" below.

## Why it cannot run here

`TrackPublishOptions::video_encoder` does not choose an encoder. It installs a
`FixedVideoEncoderSelector` on the sender
(`webrtc-sys-0.3.42/src/rtp_sender.cpp:209`, class at :91), and libwebrtc
consults a selector only *after* an encoder already exists. The first encoder is
built from the negotiated SDP format, which carries no backend tag, so
`VideoEncoderFactory::InternalFactory::Create`
(`video_encoder_factory.cpp:506`) takes its automatic-fallback path — which
explicitly skips the pass-through factory, because `IsAutomaticFallbackBackend`
returns false for `PreEncoded` (:176) — and picks the first real backend that
supports H.264. On Apple that is VideoToolbox, registered unconditionally under
`#ifdef __APPLE__`.

The real encoder then receives a pre-encoded access unit, `ToObjCVideoFrame`
casts the native `EncodedVideoFrameBuffer` to an `ObjCFrameBuffer`, and
`objc_retain` dereferences the payload as a pixel buffer.

**Measured, four consecutive runs: exactly 9 access units published, then
`SIGSEGV` / `EXC_BAD_ACCESS` on the `EncoderQueue` thread.** That is not a Rust
panic. Nothing catches it, and recording dies with the process — which inverts
the stated subsystem priority that puts preserving recordings first.

Where no real H.264 encoder is registered, initial creation fails,
`OnEncoderBroken` fires, and the selector switches to pass-through as intended.

**Corrected 2026-08-17.** This file originally called that Apple-specific and
inferred that Linux would work. That inference was wrong, and it was an
assumption stated as a conclusion. `IsAutomaticFallbackBackend`
(`video_encoder_factory.cpp:176`) excludes *only* `PreEncoded`, and the factory
registers OpenH264 under `WEBRTC_USE_H264` (:290), which is not
platform-conditional and **is** present in the prebuilt this repository links
(`-DWEBRTC_USE_H264` in `mac-x64-release/webrtc.ninja`). A real encoder wins over
pass-through on every platform LiveKit ships. Only the crash signature is
Apple-specific. See ADR 011 Decision 5 for the full correction and for what
remains unverified (the Linux prebuilt's own `webrtc.ninja` was not downloaded).

`livekit` 0.8.3 was the latest release at the time of writing (`cargo search`),
so there is no upstream fix to take.

## Measured

Full gate, `make check`, exit 0:

- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets
  --all-features -- -D warnings` clean.
- `cargo test --workspace --all-features`: **43 suites, 492 passed, 0 failed, 33
  ignored.** The ignored tests need FFmpeg/libx264, MediaMTX, PostgreSQL, or a
  LiveKit server.
- `cargo build --workspace --release` succeeded.
- `make audit`: `cargo deny` reports `advisories ok, bans ok, licenses ok,
  sources ok`; `cargo audit` ran. `cargo deny` warns about duplicate `thiserror`
  1.0.69 / 2.0.20, which is a warning and not a gate failure. This closes the
  supply-chain half of P5N.1.

`scripts/phase8/check-metric-names.py --url http://127.0.0.1:3000/metrics`
against the running server, exit 0: the three dashboards
(`camera-detail.json`, `fleet-overview.json`, `security.json`), `alerts.yml`, and
`alerts_test.yml` all check out — **all referenced metric names exist (127
exported)**. This is P5N.10 / Slice 7, which had never been run before today.

Live run of `scripts/dev/local-stack.sh` (Mac webcam → FFmpeg → MediaMTX →
Retina RTSP → recorder + native LiveKit publisher), `/metrics` at the end:

```
surveillance_camera_connection_attempts_total{camera_id="mac-webcam"} 1
surveillance_camera_connection_epochs_total{camera_id="mac-webcam"} 1
surveillance_recorder_segments_completed_total{camera_id="mac-webcam"} 8
surveillance_livekit_enabled{camera_id="mac-webcam"} 1
surveillance_livekit_state{camera_id="mac-webcam"} 8
surveillance_livekit_connection_attempts_total{camera_id="mac-webcam"} 1
surveillance_livekit_connection_successes_total{camera_id="mac-webcam"} 1
surveillance_livekit_send_failures_total{camera_id="mac-webcam"} 1
```

`surveillance_livekit_access_units_total` is absent from that excerpt because it
is **0** — the refusal happens before any frame reaches libwebrtc. Compare the
crashing runs, which reached exactly 9. `state=8` is `Stopped`
(`publication_state_value`, `apps/server/src/health_http.rs:3146`, matching the
table in `docs/phase-8/dashboards/README.md`).

Read together, that is one camera connection and no reconnect, one LiveKit
connection, one refused access unit, one stop, no retry — and a process still
alive to report it. `server=200 proxy=200` on the health probes. The server log
carries exactly one `WARN live publication cannot run on this build and will not
be retried; recording is unaffected failure=AdapterUnavailable
publication_epoch=1`, then `WARN optional LiveKit publication stopped; recording
continues reason=Terminal state=Stopped`, and **zero** `invalid publication
lifecycle transition` errors.

**P5N.4 — recording admission uninterrupted throughout — is verified by this
run:** eight segments completed while live publication went terminal.

## Not measured, and not claimed

These need a subscriber receiving media. Nothing was published, so none of them
can be evaluated on this host with this dependency:

- **P5N.6** published rate versus source rate from `/metrics` over two intervals.
- **P5N.7** subscriber decode — a browser session reporting `readyState`,
  `videoWidth`, advancing `currentTime`, and the sampled non-zero-YUV pixel check
  that distinguishes a decoding track from a green screen.
- **P5N.8** time to first picture, stated with its keyframe interval.
- **P5N.9** recovery across a LiveKit server restart and across a camera
  reconnect.

Each stays `[!]` in `ROADMAP.md`. They are blocked on a host without a registered
hardware H.264 encoder, or on an upstream `webrtc-sys` that honours the requested
backend when the *initial* encoder is created.

Also still open and unaffected by this work: the ~91% frame loss of the removed
child-process path is not "fixed" by anything measured here — the replacement
that would fix it does not run. And `surveillance_livekit_state=8` alone does not
distinguish a requested shutdown from a capability refusal; the failure category
is available on the camera health API and in the log line above, but not as a
metric.

## Two failures that were not what they looked like

Both were orphans of an earlier hard-killed stack run, and both cost real time by
imitating defects in this code:

- **"the proxied API answered 401, not 200."** An orphaned trust-boundary proxy
  still owned port 3001 holding the *previous* run's proof secret. Requests
  reached the server and failed the origin proof. This reads as an
  authentication defect and is not one.
- **"optional LiveKit publication stopped; reason=Terminal"**, seen before any of
  today's changes. An orphaned `livekit-server` from an older run had different
  API keys, so authentication failed.

`scripts/dev/local-stack.sh` now pre-flights all four ports and refuses to start,
naming the holding pid, the same way its PostgreSQL check already did.

## Unrelated defect observed and left open

`RecorderBackpressure` stops the camera when startup reconciliation over a large
recording root starves the recorder. It is not part of this slice and is not
fixed here.
