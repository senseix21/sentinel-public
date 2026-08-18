# Repository status — 2026-08-16 23:41 +06

Base commit: `e65a318` (`docs: add Phase 8 handoff`). The worktree is dirty with
in-flight Phase 5-native work and is intentionally left that way. Nothing was
committed.

## Verified — full gate

`make check`, exit 0:

| Step | Result |
|---|---|
| `cargo fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | clean |
| `cargo test --workspace --all-features` | **43 suites, 492 passed, 0 failed, 33 ignored** |
| `cargo build --workspace --release` | succeeded |
| `make audit` | `cargo deny`: advisories ok, bans ok, licenses ok, sources ok. `cargo audit` ran. |

`cargo deny` warns about duplicate `thiserror` 1.0.69 / 2.0.20. It is a warning,
not a gate failure, and is not fixed here.

The previous recorded baseline in `AGENTS.md` was 469 passed / 32 ignored. The
delta is not attributable to this slice alone — the worktree carries other
in-flight work — so `AGENTS.md` should be re-verified before its number is
updated.

## Verified — Slice 7 metric names, first run ever

```
python3 scripts/phase8/check-metric-names.py --url http://127.0.0.1:3000/metrics
```

exit 0. `camera-detail.json`, `fleet-overview.json`, `security.json`,
`alerts.yml`, and `alerts_test.yml` all pass: *all referenced metric names exist
(127 exported)*. This item (P5N.10) had never been executed before today.

## Verified — live stack run

Mac webcam → FFmpeg → MediaMTX → Retina RTSP → recorder + native LiveKit
publisher, behind the trust-boundary proxy. `server=200 proxy=200`.

```
surveillance_camera_connection_epochs_total{camera_id="mac-webcam"} 1
surveillance_recorder_segments_completed_total{camera_id="mac-webcam"} 8
surveillance_livekit_connection_attempts_total{camera_id="mac-webcam"} 1
surveillance_livekit_connection_successes_total{camera_id="mac-webcam"} 1
surveillance_livekit_send_failures_total{camera_id="mac-webcam"} 1
surveillance_livekit_state{camera_id="mac-webcam"} 8      # Stopped
```

`surveillance_livekit_access_units_total` is 0. One camera connection with no
reconnect, one LiveKit connection, one refused access unit, one stop, no retry —
and a live process reporting it. Exactly one `WARN live publication cannot run on
this build and will not be retried; recording is unaffected
failure=AdapterUnavailable`, then `WARN optional LiveKit publication stopped;
recording continues reason=Terminal state=Stopped`. **Zero** `invalid publication
lifecycle transition` errors.

Recording ran uninterrupted throughout (P5N.4).

## Verified fact — the native path cannot run on Apple

Implemented, and refused at runtime before the session is created.
`webrtc-sys` 0.3.42 builds the *first* encoder from the negotiated SDP format,
which carries no backend tag, so the requested pass-through backend is never
selected and VideoToolbox dereferences the pre-encoded payload as a pixel buffer.
Measured four times: exactly 9 access units, then `SIGSEGV` on the
`EncoderQueue` thread — not a Rust panic, uncatchable, and it takes recording
down with the process.

Full mechanism with source citations: `docs/adr/011-...`, Decision 5.
Full evidence: `docs/phase-5/local-evidence-2026-08-16.md`.

`VideoEncoderBackend::list_available()` cannot detect this — it pushes
`PreEncoded` unconditionally. An earlier guard built on it was written, tested
against a real run, found ineffective, and removed rather than kept as false
reassurance.

## Changed paths

Modified: `ROADMAP.md`, `crates/livekit/src/{domain,owner,native,lib,adapter,config,token}.rs`,
`crates/livekit/Cargo.toml`, `crates/livekit/tests/publisher_fixture.rs`,
`scripts/dev/local-stack.sh`, `docs/adr/011-native-encoded-livekit-publication.md`.
New: `apps/server/build.rs`, `docs/phase-5/local-evidence-2026-08-16.md`.

Two own-code defects were fixed, both only reachable once a non-retryable send
failure could exist:

1. The send path discarded `AdapterError::kind()`, turning every send failure
   into a retryable `Transport` failure. An adapter that could never publish
   reconnected every backoff interval and called itself `Degraded`.
2. `domain.rs` had no `Publishing → Terminal` edge, so the correct transition
   logged `ERROR invalid publication lifecycle transition`.

Covered by `a_non_retryable_send_failure_stops_publication_instead_of_reconnecting`
in `crates/livekit/src/owner.rs`.

## Not verified — say so plainly

- **P5N.6** published-versus-source frame rate.
- **P5N.7** subscriber decode in a browser (`readyState`, `videoWidth`,
  advancing `currentTime`, sampled non-zero-YUV pixel check).
- **P5N.8** time to first picture.
- **P5N.9** recovery across LiveKit restart and camera reconnect.

All four need a subscriber receiving media. Nothing is published on this host, so
none can be evaluated. Each is `[!]` in `ROADMAP.md`.

The ~91% frame loss of the removed `lk` child-process path is **not** fixed by
anything measured here. The replacement that would fix it does not run on this
machine.

## Open, unrelated

`RecorderBackpressure` stops the camera when startup reconciliation over a large
recording root starves the recorder. Observed, not in this slice, not fixed.
