# Repository status — 2026-08-17 21:59 +06

Base commit: `e65a318` (`docs: add Phase 8 handoff`). The worktree is dirty with
in-flight Phase 5 and Phase 8 work and is intentionally left that way. Nothing
was committed.

Previous snapshot: `context/2026-08-17-1450/`. Its amendments record the
measurements this session started from; read them for the recording and playback
evidence, which is not repeated here.

## What changed this session

**A defect that killed live video after every camera reconnect is fixed,
verified in a unit test and on the local stack.** It was found by checking
whether playback and the API work, which they do.

The chain, in the order it was established:

1. Playback and the API verified end to end (previous snapshot's evening
   amendment).
2. The live lane measured stalled: publication counters frozen while the
   distributor evicted the entire live lane at source rate, and health still
   reported `Publishing`.
3. Root cause located, fixed, and both halves proved — the stall and the lie.

## Verified — full gate

`make check`, exit 0:

| Step | Result |
|---|---|
| `cargo fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | clean |
| `cargo test --workspace --all-features` | **43 suites, 509 passed, 0 failed, 34 ignored** |
| `cargo build --workspace --release` | succeeded |
| `make audit` | `cargo deny`: advisories ok, bans ok, licenses ok, sources ok. `cargo audit`: 1 allowed warning (`number_prefix` unmaintained, RUSTSEC-2025-0119, via `indicatif` ← `muxide`) |

509 passed, up from the 508 recorded in `AGENTS.md`. The one new test is the
regression test below. `cargo deny` still warns about duplicate `thiserror`
1.0.69 / 2.0.20 — a warning, not a gate failure, not addressed here.

## The defect

### Symptom

Measured on the local stack, sampled over 40 s with media arriving:
`live_offered` frozen, `adapter_drops` frozen, `access_units` 0,
`parameter_set_waits` 0, `stale_drops` frozen, `live_queue_depth` pinned at 4/4,
`surveillance_camera_live_evictions_total` climbing at roughly the 15 fps source
rate. `recorder_completed_segments` kept advancing throughout.

Health reported `live_publication_state: "Publishing"` and
`surveillance_livekit_state` 3 (connected).

Every path through the receive arm of `publish_session` increments some counter,
so all of them frozen at once meant the loop was blocked on an `.await`, not
looping and discarding. Zero `optional LiveKit publication stopped` lines in the
log ruled out the owner having exited — `apps/server/src/pipeline.rs` logs that
when the publication future completes.

### Root cause

`crates/camera/src/rtsp.rs` builds `MediaTimestamp` from Retina's raw RTP
timestamp. A fresh RTSP session starts at a new, arbitrary RTP base, so a
reconnect moves the media timeline by an arbitrary amount — usually *forward*.

`publish_session` paced publication against a `pacing_origin` captured at the
first parameterised keyframe. Its discontinuity guard covered a timebase change
and a timestamp that ran *backwards*. A forward jump matched neither, so the
origin stayed at its pre-reconnect value, `media_elapsed` became enormous, and
the pacing sleep was handed that difference.

Because this loop is the only consumer of the live subscription, sleeping stopped
it draining at all. The distributor then evicted the whole live lane, every
counter in the session froze, and no state transition ever fired.

The stale-drop guard could not catch it: that one fires when media runs *behind*
the wall clock, which is the opposite direction.

### Fix

`crates/livekit/src/owner.rs`:

- Pacing is now a named decision, `Pacing`, returned by a new `pace` method that
  uses `max_lag` symmetrically. Media behind the wall clock by more than
  `max_lag` is stale and dropped, which is unchanged and is the
  freshness-over-completeness policy `AGENTS.md` sets for live viewing. Media
  *ahead* of its own pacing origin by more than `max_lag` is now treated as a
  discontinuity and resynchronised at the next parameterised keyframe, rather
  than slept off.
- `note_timeline_discontinuity` counts all three discontinuity conditions.
  Timebase change and backward jump already reset pacing but were **silent**;
  they are now counted too.
- New counter, exposed as
  `surveillance_livekit_timestamp_discontinuities_total`. One per camera
  reconnect is expected. Rising while the camera holds one session means a source
  whose timestamps cannot be trusted for pacing.

`crates/livekit/src/domain.rs`: added the `Degraded → Publishing` lifecycle edge,
and `publish_session` takes it when it resynchronises.

That edge is a deliberate state-machine change, not incidental. Without it
`Degraded` is a one-way door for the life of the LiveKit connection — `Publishing`
was only ever set once, at room connect — so after the first camera reconnect the
state would have read `Degraded` forever. That is the same defect class as the
`Inconsistent` one-way door recorded in the previous snapshot, and a permanent
`Degraded` is a signal an operator learns to ignore. It is no improvement on the
`Publishing` it replaced.

Two supporting extractions, both to keep `publish_session` under the
`clippy::too_many_lines` limit without an `#[allow]`: `pace` as above, and `adapt`
for the annex-B/RTP-timestamp conversion that counts a media rejection rather
than failing the session.

### Regression test

`a_forward_timestamp_jump_resynchronises_instead_of_stalling_the_lane`
(`crates/livekit/src/owner.rs`). Publishes a keyframe to establish the origin,
then a delta one hour of 90 kHz ticks ahead — what a fresh RTSP session's base
looks like from here, against a one-second `max_lag` — then a recovery keyframe.

**Confirmed to fail without the fix.** With the bound check neutralised the
assertion reports `left: [(true, 0)]` against `right: [(true, 0), (true, 0)]`:
one publication, then the lane stalls, which is the production symptom exactly.

It asserts that publication *continues*, not merely that the jump is counted — a
test checking only the counter would pass against a loop that counted the jump and
stalled anyway. It also asserts the state returns to `Publishing`.

## Verified — live, on the local stack

`scripts/dev/local-stack.sh` components: MediaMTX, a 1280x720@15 H.264 webcam
bridge, `livekit-server`, PostgreSQL, the trust-boundary proxy, the dashboard.
Read from `/metrics` on `mac-webcam`. The reconnect was forced by killing the
webcam bridge, letting MediaMTX drop the path, and restarting the bridge — which
is the reproduction recipe.

| | baseline | post-reconnect | +25 s |
|---|---|---|---|
| `surveillance_livekit_adapter_drops_total` | 698 | 1077 | 1451 |
| `surveillance_camera_live_evictions_total` | 6 | 6 | 6 |
| `surveillance_livekit_timestamp_discontinuities_total` | 0 | 1 | 1 |
| `surveillance_livekit_state` | 3 | 3 | 3 |

Read from that table:

- **The lane keeps draining.** `adapter_drops` +374 across 25 s is about 15/s,
  the source rate. Before the fix this counter froze permanently at the reconnect.
  `viewers` is 0 throughout, so the adapter is correctly offered every access unit
  and drops it; this counter is the proof of consumption, not of loss.
- **No lane loss at all.** Evictions flat at 6 across the whole run. Before the
  fix they climbed at roughly source rate without bound.
- **The jump is visible.** Counted exactly once, at the reconnect.
- **Health is honest.** State recovered to `Publishing` rather than sticking at
  `Degraded`.

An intermediate build — the pacing fix without the `Degraded → Publishing` edge —
showed the state stuck at 5 (`Degraded`) and residual eviction of about 4.2/s.
Both are absent from the final build measured above. The residual eviction in that
intermediate run was not chased, because the build that shipped shows none.

## What is *not* verified, stated plainly

- **The 41% publish shortfall is untouched and still open.** The previous
  snapshot measured the adapter publishing 8.85 frames/s from a 15.000 fps source
  *with a viewer attached*. Everything above ran at `viewers` 0, where the adapter
  drops cheaply and the owner keeps up easily, so it neither reproduces nor
  addresses that. It remains next-actions item 2, and its leading candidate —
  distributor eviction — was eliminated by measurement (1.7% pre-reconnect), with
  no replacement candidate yet.
- **No viewer was attached across a reconnect.** Whether a subscriber present
  during the discontinuity keeps a picture, or pays a keyframe interval of black
  while the session resynchronises, is not measured.
- **The dashboard's own playback rendering.** The Playwright check timed out at
  120 s and was killed. Playback is proven at the API and unproven in the page.
- **No CPU-per-viewer number**, so ADR 012's second reversal criterion still has
  nothing behind it. No capacity claim follows from anything here.
- **One camera, one browser, a Mac webcam at Constrained Baseline 1280x720.**
  Representative High Profile cameras, H.265, and time-to-first-picture remain
  untouched.
- **`promtool` is still not installed**, so the alert rules in
  `docs/phase-8/alerts.yml` have never been executed. The new
  `surveillance_livekit_timestamp_discontinuities_total` series has no alert rule
  and no rule test.

## Still open from the previous snapshot

Both defects recorded there remain unfixed, and both are recorded as
next-actions items rather than silently carried:

- A reconnect mid-segment can produce a decodable file whose row is permanently
  `incomplete`, which the API 404s and which no metric counts
  (`surveillance_metadata_rejected_full_total` is 0).
- `RecordingStatus::Inconsistent` has no implemented operator path back; 108
  present, cleanly-probeable files carry a permanent flag.

The `Degraded → Publishing` edge added above is the same problem solved in the
publication lifecycle. The storage-side equivalents are still one-way doors.
