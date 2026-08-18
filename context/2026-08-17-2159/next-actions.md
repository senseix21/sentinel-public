# Next actions — 2026-08-17 21:59 +06

Ordered by what unblocks the most. Every item says what evidence would close it,
because none of these should be closed by reading the code.

Items 1–7 carry forward from `context/2026-08-17-1450/next-actions.md`, which
holds their full reasoning. Only what changed is restated here.

## 1. Account for the frames the live path loses, with a viewer attached

Was item 2, promoted because the live-lane stall that outranked it is fixed.

The source is 15.000 fps, proved frame by frame from the recorder's own segments.
The adapter published 8.85/s with a viewer attached. About 41% of the stream never
reaches publication and nothing says where it goes.

**The leading candidate is eliminated.** This item used to propose the live
distributor's freshness-preferring eviction. Measured: `live_offered: 713,
live_dropped: 12` — 1.7%, which cannot account for 41%. There is no replacement
candidate yet.

Two facts from the reconnect fix constrain the next attempt, and both say the loss
is *not* where the stall was:

- At `viewers` 0 the lane now drains at full source rate with evictions flat, so
  neither the subscription nor the owner's loop loses frames on its own.
- Therefore the loss appears only when the adapter is actually decoding and
  re-encoding. The FFmpeg write path and libwebrtc's encoder queue are the
  remaining unmetered steps.

What would close it: one run with a subscriber attached, sampling
`surveillance_livekit_adapter_drops_total`,
`surveillance_livekit_published_frames_total`, and
`surveillance_camera_live_evictions_total` together. Do it before item 2 —
measuring CPU per viewer while the path silently runs at 59% of source produces a
number that understates its cost, and ADR 012's reversal criterion 2 would inherit
the error.

## 2. Measure CPU per concurrent viewer

Unchanged, and still blocked behind item 1. ADR 012 accepted decode-and-re-encode
with an explicit reversal criterion that has no number behind it, so the ADR
cannot be revisited on evidence. `surveillance_livekit_viewers` supplies the
denominator; the numerator is process CPU under 1, 2, 4 subscribers on one camera,
then across cameras. Host CPU is not instrumented, so this is an external
measurement, not a metric.

## 3. Does a viewer survive a reconnect?

New, and cheap now that the stall is gone.

The reconnect fix resynchronises at the next parameterised keyframe. With `-g 15`
at 15 fps that is up to a second of no new frames. Nothing measured what a
subscriber present during the discontinuity actually sees — a brief freeze, a
keyframe interval of black, or a dropped track.

Every reconnect measurement so far ran at `viewers` 0. Attach a subscriber, force
a reconnect with the recipe in `repository-status.md`, and observe both the page
and `surveillance_livekit_published_frames_total`. This is also the natural place
to close item 8's time-to-first-picture question.

## 4. Confirm the dashboard renders playback

Playback is proven at the API — 11 274 711 bytes, 900 frames, 60.000000 s,
decoded clean. The page drawing it is unproven: the Playwright check timed out at
120 s and was killed. Repeat it in a headed browser with a longer budget.

## 5. Qualify a real camera

Unchanged. Every live figure to date is a Mac webcam at Constrained Baseline,
1280x720, a measured 15.000 fps, one slice. Representative High Profile cameras,
multi-slice input, H.265 policy, and browsers other than Chromium all need
hardware. `ROADMAP.md` Phase 5 exit keeps these `[~]`/`[!]`.

Worth noting against the reconnect fix: `max_lag` is now load-bearing in both
directions. A real camera with a long GOP or unusual timestamp behaviour is the
test of whether one second is the right bound. It is configuration, not a
constant, so this is a qualification question rather than a code question.

## 6. Time to first picture

Unchanged and not instrumented. A viewer waits for the next keyframe after
subscribing. Fold into item 3.

## 7. Latency, or an explicit decision not to have it

Unchanged. `TimestampAuthority::AdapterClock` makes a media-timestamp latency
claim structurally impossible through this adapter, deliberately. Either build a
viewer-side harness or record a decision that live latency is qualified by
observation rather than by metric. Do not add a latency metric to this path — it
would be fabricated.

## 8. Alert rules, including the new series

`promtool` is still not installed, so `docs/phase-8/alerts.yml` has never been
executed. `surveillance_livekit_timestamp_discontinuities_total` is new this
session and has no rule.

It deserves one, and the rule is not "greater than zero": one discontinuity per
camera reconnect is correct and expected. The alertable condition is a *rate* —
discontinuities accruing while the camera holds a single session, which means a
source whose timestamps cannot be trusted for pacing. Writing that rule requires
deciding the window, so it is a small product decision rather than a mechanical
addition.

Also still open: `[~]` LiveKit server stop/restart while recording,
deterministically qualified. The runbook is written and unexercised.

## 9. Decide what clears an `Inconsistent` segment

Unchanged from the previous snapshot's item 7, and now with a precedent.

`RecordingStatus::Inconsistent` may transition only to `Quarantined`
(`crates/storage/src/metadata.rs`), and `reconciliation_owner.rs` takes the
`Ok(_) => {}` no-op when a later probe succeeds for an already-flagged row. So the
flag is permanent, nothing calls the only legal exit, and the local index holds
108 flagged rows whose files are present and probe cleanly.

The precedent: this session added the `Degraded → Publishing` edge because a
one-way door in the publication lifecycle made the state useless after the first
reconnect. The storage-side doors are the same shape. That does **not** settle
them — footage the index once disagreed with is a higher bar than a transient
pacing fault, and re-blessing it silently is worse than leaving it flagged. It
does mean the question is now "what is the audited operator path back", not
"should there be one".

Needs a product decision: is re-indexing an explicit, audited, per-segment
operator action, or does a successful probe plus a matching size clear the flag
automatically? The second is convenient and quietly weakens the guarantee.

## 10. A reconnect mid-segment can bury playable footage

Unchanged from the previous snapshot's item 9.

One occurrence: `ERROR recording metadata persistence rejected immutable evidence
category=Conflict`. The file is decodable (224 frames) with a garbage container
duration; the row is permanently `incomplete`/`reconciliation`; the API 404s it;
and `surveillance_metadata_rejected_full_total` is 0, so no metric counts the
rejection.

Smallest honest fix first: **count it.** A rejection no metric records is
invisible exactly where an operator looks. That is worth doing independently of
deciding what the row should become.

Then the harder question, which is item 9 by another route. Answer them together.

## Do not do

- Do not delete `crates/livekit/src/native.rs`. It is ADR 012's reversal path and
  the cost of keeping it is one unused constructor.
- Do not raise `H264ToI420::WRITE_TIMEOUT` to make restarts go away. That is the
  steady-state overload detector. The startup case already has its own budget.
- Do not raise `max_lag` to reduce timestamp discontinuities. It is now the bound
  in both directions: raising it lengthens the pacing sleep this session's fix
  exists to bound, and re-admits the stall for smaller jumps.
- Do not treat `surveillance_livekit_timestamp_discontinuities_total` above zero
  as a fault. One per camera reconnect is correct behaviour. See item 8.
- Do not describe detection as working. It is still `DeterministicFakeInference`.
- Do not claim a camera count from any of this.
- Do not measure CPU per viewer until item 1 is closed.
- Do not clear the 108 `inconsistent` rows by hand or by a migration. They are the
  evidence for item 9, and the transition they would need does not exist.
- Do not describe live video as working without saying which case. Cold start and
  post-reconnect are both now measured at `viewers` 0; with a viewer attached
  across a reconnect is item 3 and is unmeasured.
