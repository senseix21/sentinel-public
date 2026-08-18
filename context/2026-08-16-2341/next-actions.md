# Next actions — 2026-08-16 23:41 +06

## Where this sits

The native encoded LiveKit publication slice (ADR 011, `docs/phase-5/SPEC.md` and
`PLAN.md`) is implemented, gated, and **blocked on the development platform**.
`make check` is green and the Slice 7 metric-name check passes for the first
time. The one thing the slice existed to prove — live video at source frame rate —
remains unproven, because the path that would prove it segfaults on macOS.

Read `context/2026-08-16-2341/repository-status.md` for the measurements, and
`docs/phase-5/local-evidence-2026-08-16.md` for what was and was not verified.

## The decision that belongs to the product owner

**Corrected 2026-08-17.** An earlier version of this file recommended moving to
Linux. That recommendation was withdrawn after reading the encoder factory
instead of assuming it: `IsAutomaticFallbackBackend` excludes only `PreEncoded`,
and OpenH264 is registered under `WEBRTC_USE_H264` on every platform LiveKit
ships. See ADR 011 Decision 5.

1. **Vendor and patch `webrtc-sys`** so the requested backend is honoured when
   the *initial* encoder is created. This is a material dependency decision: it
   forks a C++ build with its own toolchain, prebuilt binary distribution per
   target, and upgrade cost. It needs its own ADR and is not taken here. It is
   the only option that restores true zero-transcode passthrough.
2. ~~**Relocate to Linux.**~~ **Withdrawn.** The premise was false; a Linux
   prebuilt registers OpenH264 and would take the same automatic-fallback path.
   Only the crash signature is Apple-specific. Before anyone reconsiders this,
   grep the Linux prebuilt's `webrtc.ninja` for `-DWEBRTC_USE_H264` — that
   artifact has not been downloaded and the inference is unverified.
3. **Wait for upstream.** `livekit` 0.8.3 is the latest release; there is nothing
   to take today. Live video stays broken for an unbounded interval.
4. **Decode and re-encode, only while a viewer is attached.** Feed I420 frames to
   an ordinary `NativeVideoSource`; let LiveKit encode. Uses only released,
   supported APIs on every platform, adds no dependency surface, and costs CPU
   only while someone is actually watching. It gives up the zero-transcode
   property of Decision 1 — but AGENTS.md permits transcode when "a documented
   consumer or codec boundary requires it," and an SDK that cannot accept our
   encoded stream is exactly such a boundary. It is also unavoidable for any
   H.265 camera, which browsers cannot play over WebRTC regardless.

**Recommendation: option 4 now, option 1 only if measurement demands it.** Live
viewing is priority 3 of 5 and is typically one or two cameras at a time, so the
transcode cost is bounded by viewer count rather than camera count. Forking
libwebrtc to save CPU nobody has yet measured as scarce is the more expensive
mistake. Option 4 also has to exist eventually for H.265, which makes it the
lower-risk investment.

Option 4 needs its own ADR superseding parts of ADR 011 Decision 1, and its cost
must be *measured* — CPU per concurrent viewer at the real resolution and frame
rate — not assumed.

## Immediate work, in order

1. **Get the product owner's answer on the three options above.** P5N.6–P5N.9
   cannot progress without it.
2. On whichever host is chosen, run the ADR 011 Decision 5 re-verification
   verbatim: bring up the local stack and watch
   `surveillance_livekit_access_units_total` climb past 9 without the process
   dying. Past 9 is the whole test — the crashing runs reached exactly 9.
3. Then, and only then, measure P5N.6 (published vs source rate), P5N.7
   (browser `readyState` / `videoWidth` / advancing `currentTime` plus the
   sampled non-zero-YUV pixel check), P5N.8 (time to first picture, stated with
   its keyframe interval), P5N.9 (recovery across LiveKit restart and camera
   reconnect). Record them in a new dated evidence file; do not edit
   `local-evidence-2026-08-16.md`.
4. Re-verify the `AGENTS.md` "Current Repository Reality" test count. It still
   says 469 passed / 32 ignored; the tree now reports 492 / 33, and the delta is
   not all from this slice.

## Deferred, deliberately

- `surveillance_livekit_state=8` (Stopped) does not distinguish a requested
  shutdown from a capability refusal. The failure category is on the camera
  health API and in the log line, but not as a metric. Adding one touches metric
  cardinality and was not requested; decide before an operator relies on the
  gauge alone.
- The duplicate `thiserror` 1.0.69 / 2.0.20 `cargo deny` warning.
- `RecorderBackpressure` stopping the camera when startup reconciliation over a
  large recording root starves the recorder. Real, observed, unrelated to this
  slice, open.

## Housekeeping

- `makerun-live.png` is sitting in the repository root, untracked. It should not
  be committed.
- The worktree is intentionally dirty. Nothing in this session was committed, and
  nothing should be without an explicit request.
