# Next actions — 2026-08-17 14:50 +06

Ordered by what unblocks the most. Every item says what evidence would close it,
because none of these should be closed by reading the code.

## 1. Measure CPU per concurrent viewer

This is the one number the design is waiting on. ADR 012 accepted decode-and-
re-encode with an explicit reversal criterion: if CPU per viewer makes the 10- and
20-camera targets unreachable, the alternative is forking `webrtc-sys` to fix
backend selection. That criterion currently has no number behind it, so the ADR
cannot be revisited on evidence.

`surveillance_livekit_viewers` now exists, which makes the denominator readable.
What is needed is the numerator: process CPU under 1, 2, 4 subscribers on one
camera, then across cameras. `scripts/phase9/livekit-load-fixture.sh` already
drives multiple cameras; the missing piece is subscriber fan-out and a CPU
sample. Note that host CPU is *not* instrumented — the dashboard says so — so
this is an external measurement, not a metric.

## 2. Account for the frames the live path loses — server side first

**Raised in priority by the 2026-08-17 correction.** The source is 15.000 fps,
proved frame by frame from the recorder's own segments. The adapter published
8.85/s. About 41% of the stream never reaches publication, and nothing currently
says where it goes.

`surveillance_livekit_adapter_drops_total` was flat across the subscriber window,
so the loss is before the adapter.

**Amended 2026-08-17 evening — the leading candidate is eliminated.** This item
proposed the live distributor's freshness-preferring eviction. Measured
pre-reconnect health: `live_offered: 713, live_dropped: 12` — **1.7%**, which
cannot account for a 41% shortfall. Eviction is not the answer, and there is no
replacement candidate yet. The next measurement should instrument the owner's own
stale-drop decision: it is the only unmetered step between the subscription and
the adapter. Note this is now entangled with item 8 — do that one first, because a
stalled owner and a lossy owner may be the same bug at two severities.

Do this *before* item 1. Measuring CPU per viewer while the path is silently
running at 59% of source would produce a number that understates the real cost,
and reversal criterion 2 of ADR 012 would then rest on it.

Note that dropping stale media to preserve latency is the policy `AGENTS.md` sets
for this consumer, so the answer may be that this is correct behaviour. The defect
is that nobody chose 59% and no metric names it.

## 2b. Explain the browser frame rate

Downstream of 2, and no longer the first suspect. Headless Chromium's
`getVideoPlaybackQuality()` reported far below even the 8.85/s the server
published. Candidates, in the order worth eliminating: headless rendering
throttling (cheapest to rule out — repeat in a headed browser), libwebrtc encoder
queue drops on the send side, and the receiver's jitter buffer. Until both this
and item 2 are resolved, no claim about what a viewer sees per second is
supportable.

## 3. Qualify a real camera

Every live figure in this snapshot is a Mac webcam at Constrained Baseline,
1280x720, a measured 15.000 fps, one slice. The open gates are unchanged:
representative High Profile
cameras, multi-slice input, H.265 policy, and browsers other than Chromium.
`ROADMAP.md` Phase 5 exit keeps these as `[~]`/`[!]`; they need hardware.

## 4. Time to first picture

Not instrumented. A viewer waits for the next keyframe after subscribing — with
`-g 15` at 15 FPS that is up to a second, and on a real camera with a longer GOP
it is longer. Worth measuring before deciding whether it needs a fix, and worth a
metric either way, because it is the part of "seamless" that this session did not
touch.

## 5. Latency, or an explicit decision not to have it

`TimestampAuthority::AdapterClock` makes a media-timestamp latency claim
structurally impossible through this adapter, deliberately. Two honest ways
forward: build a viewer-side measurement harness, or accept that live latency is
qualified by observation rather than by metric and record that as a decision. Do
not add a latency metric to this path — it would be fabricated.

## 6. Phase 5 exit items that remain open

- `[~]` LiveKit server stop/restart while recording, deterministically qualified.
  The runbook is written and unexercised.
- `[~]` `promtool` is still not installed, so `docs/phase-8/alerts.yml` rule tests
  have still never been executed. The three new `surveillance_livekit_*` series
  are covered by `scripts/phase8/check-metric-names.py` against a live server,
  which is the check that matters most, but the alert rules themselves are
  unrun.

## 7. Decide what clears an `Inconsistent` segment

Found while verifying recording. `RecordingStatus::Inconsistent` may transition
only to `Quarantined` (`crates/storage/src/metadata.rs`), and
`reconciliation_owner.rs` takes the `Ok(_) => {}` no-op when a later probe
succeeds for an already-flagged row. So a flag is permanent.

That is the right default — an index that silently re-blesses footage it once
disagreed with is worse than one that stays suspicious. But there is no
implemented operator path back, and the local index already holds **108 flagged
rows whose files are present and probe cleanly**. Here that is dev-history
residue from database wipes. On a deployment it is retrievable footage marked
unusable with no supported recovery.

Needs a product decision, not a code change first: is re-indexing an operator
action (explicit, audited, per segment), or does a successful probe plus a
matching size clear the flag automatically? The second is the convenient answer
and the one that quietly weakens the guarantee. Either way the `Quarantined`
transition is currently the only exit and nothing calls it.

## 8. The live lane stalls after a reconnect while health reports `Publishing`

**Highest-priority defect on this list.** Found while verifying playback; see the
2026-08-17 evening amendment in `repository-status.md` for the numbers.

Signature, sampled over 40 s with media arriving: `live_offered` frozen,
`live_dropped` +160, `adapter_drops` frozen, `published_access_units` 0,
`live_queue_depth` pinned at 4/4 — while `recorder_completed_segments` kept
advancing and health reported `live_publication_state: "Publishing"` with
`surveillance_livekit_state` 3.

Two things are already established, so do not re-derive them:

- It is not an empty stream. With the camera down, no live counter moves at all
  across 30 s. The sample was a starved consumer.
- The owner did not exit. Zero `optional LiveKit publication stopped` lines in
  the log, which is what `apps/server/src/pipeline.rs` emits when the publication
  future completes. It is a stall inside the loop, upstream of the adapter.

**It reproduces.** Restarting the webcam bridge created a second camera session,
and across 30 s: evictions +344 (~11.5/s against a 15 fps source) while
`adapter_drops`, `access_units_total`, and `published_frames_total` all stayed
frozen. So this is a repeatable defect, not an artifact of one bad session, and a
reproduction recipe already exists: kill the bridge, let MediaMTX drop the path,
restart the bridge, sample `/metrics` for 30 s.

The sharpest diagnostic is `adapter_drops` with **no** subscriber. It *should*
climb — the adapter is offered every access unit and drops it, which is what the
original evidence table in `repository-status.md` recorded as "250 and climbing".
Frozen means the owner is not consuming at all, not consuming-and-discarding.
That distinction is what makes this a stall rather than a policy.

Where to look: `crates/livekit/src/owner.rs`, at the loop's behaviour when the
camera's session ends and a new epoch begins. Note that `subscriptions.live` is
created once in `apps/server/src/pipeline.rs`, *outside* the reconnect loop, so it
is not a per-epoch channel — whatever ends the owner's consumption is not the
channel being replaced.

Untested, so do not assume either way: whether a subscriber arriving during the
stall still gets a picture. `viewers` was 0 throughout, so nothing decoded and no
CPU was burned.

Two things are wrong here and they need separating. The stall is a bug. That
health reports `Publishing` through it is a **second, worse** bug: an operator has
no signal that live video is dead. Even with the stall unfixed, the state should
not read healthy — `PublicationHealth` is not invalidated when the owner stops
consuming, and the `publication = None` branch in `pipeline.rs` leaves the last
snapshot standing.

## 9. A reconnect mid-segment can bury playable footage

One occurrence: `ERROR recording metadata persistence rejected immutable evidence
category=Conflict`. The file is decodable (224 frames) with a garbage container
duration; the row is permanently `incomplete`/`reconciliation`; the API 404s it;
and `surveillance_metadata_rejected_full_total` is 0, so no metric counts the
rejection.

Smallest honest fix first: **count it.** A rejection that no metric records is
invisible in exactly the place an operator looks. That is a real change
independent of deciding what the row should become.

Then the harder question, which is the same one as item 7 by another route: what
is the supported path for footage that exists, decodes, and is indexed as
unusable? Do not answer item 9 separately from item 7 — one decision should cover
both.

## Do not do

- Do not delete `crates/livekit/src/native.rs`. It is ADR 012's reversal path and
  the cost of keeping it is one unused constructor.
- Do not raise `H264ToI420::WRITE_TIMEOUT` to make restarts go away. That is the
  steady-state overload detector. The startup case already has its own budget.
- Do not describe detection as working. It is still `DeterministicFakeInference`.
- Do not claim a camera count from any of this.
- Do not measure CPU per viewer until item 2 is closed. At 59% of source the
  number would understate the path's cost and ADR 012's reversal criterion would
  inherit the error.
- Do not clear the 108 `inconsistent` rows by hand or by a migration. They are the
  evidence for item 7, and the transition they would need does not exist.
- Do not describe live video as working without qualifying item 8. It works from a
  cold start and it is dead after a reconnect. The earlier "live video works"
  headline in `repository-status.md` was measured before any reconnect had
  happened and is true only for that case.
- Do not "fix" item 8 by making the live subscription per-epoch or by restarting
  the publication owner on reconnect until the stall is reproduced and located.
  Both are plausible-sounding changes that would mask an unidentified stall rather
  than remove it.
- Do not treat the 41% shortfall (item 2) and the stall (item 8) as separate until
  item 8 is located. They may be one bug.
