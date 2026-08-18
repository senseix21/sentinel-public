# Next actions — 2026-08-18 15:27 +06

Ordered by what unblocks or protects the most. Every item says what evidence
would close it, because none of these should be closed by reading the code.

Item 1 is new and did not exist as an engineering item before, because it is not
an engineering problem. Items 2–11 carry forward from
`context/2026-08-17-2159/next-actions.md` with their order intact; that file
holds their full reasoning and is not repeated here.

## 1. Get the working tree under version control

New, and first because everything else in this list assumes the code still
exists tomorrow.

93 untracked files, 8124 lines of untracked Rust, and 53 modified tracked files
(+6712 / −703) sit on top of `e65a318`. That includes the whole of `crates/auth`,
the server's authentication and trust-boundary modules, the 2424-line dashboard,
three of five migrations, ADRs 009–012, every Phase 8 and Phase 9 document and
fixture, and the six context snapshots — two of which `context/README.md` already
directs the next reader to. See `repository-status.md` for the full table.

None of it has ever been committed or pushed. A `git clean -fd` ends Phase 8.

This is deliberately *not* one commit. The tree contains at least four separable
bodies of work — the Phase 5 live-publication path (ADRs 011, 012), Phase 8
security (ADR 009, `crates/auth`, migrations 0003–0004), the dashboard (ADR 010),
and the Phase 9 evidence drafts. Sequencing them keeps the history reviewable and
keeps `git bisect` meaningful across the reconnect and transcode fixes.

Two things to do before staging anything broadly: extend `.gitignore` — it
already has an uncommitted addition of `/.local/` and `/recordings/` — to cover
`.playwright-mcp/` and `makerun-live.png`, which are run artifacts; and confirm
`.cargo/audit.toml` and `deny.toml` go *in*, since without them a fresh clone's
`make audit` is not the gate that passed here.

What would close it: `git status --porcelain` showing only intentional in-flight
edits, and `make check` passing from a clean clone of the branch rather than from
this directory.

Do not rewrite `AGENTS.md`'s 508-test figure until this lands. It should describe
a state someone else can check out.

## 2. Account for the frames the live path loses, with a viewer attached

Unchanged, and still the top engineering item.

15.000 fps source proved frame by frame from the recorder's own segments; 8.85/s
published with a viewer attached. About 41% never reaches publication and nothing
says where it goes. The leading candidate — the live distributor's
freshness-preserving eviction — is eliminated: `live_offered: 713,
live_dropped: 12`, 1.7%, which cannot account for 41%. No replacement candidate.

Two constraints from the reconnect fix, both saying the loss is not where the
stall was: at `viewers` 0 the lane drains at full source rate with evictions flat,
so neither the subscription nor the owner's loop loses frames on its own; the loss
therefore appears only while the adapter is decoding and re-encoding, leaving the
FFmpeg write path and libwebrtc's encoder queue as the unmetered steps.

What would close it: one run with a subscriber attached, sampling
`surveillance_livekit_adapter_drops_total`,
`surveillance_livekit_published_frames_total`, and
`surveillance_camera_live_evictions_total` together. Do it before item 3 —
measuring CPU per viewer while the path silently runs at 59% of source produces a
number that understates its cost, and ADR 012's reversal criterion 2 inherits the
error.

## 3. Measure CPU per concurrent viewer

Unchanged, still blocked behind item 2. ADR 012 accepted decode-and-re-encode with
a reversal criterion that has no number behind it.
`surveillance_livekit_viewers` supplies the denominator; the numerator is process
CPU under 1, 2, 4 subscribers on one camera, then across cameras. Host CPU is not
instrumented, so this is an external measurement, not a metric.

## 4. Does a viewer survive a reconnect?

Unchanged. The reconnect fix resynchronises at the next parameterised keyframe;
with `-g 15` at 15 fps that is up to a second of no new frames. Every reconnect
measurement so far ran at `viewers` 0. Attach a subscriber, force a reconnect with
the recipe in `context/2026-08-17-2159/repository-status.md` — kill the webcam
bridge, let MediaMTX drop the path, restart the bridge — and observe both the page
and `surveillance_livekit_published_frames_total`. Fold item 7 into this run.

## 5. Confirm the dashboard renders playback

Unchanged. Playback is proven at the API — 11 274 711 bytes, 900 frames,
60.000000 s, decoded clean — and unproven in the page. The Playwright check timed
out at 120 s and was killed. Repeat in a headed browser with a longer budget.

## 6. Qualify a real camera

Unchanged. Every live figure to date is a Mac webcam at Constrained Baseline,
1280x720, a measured 15.000 fps, one slice. Representative High Profile cameras,
multi-slice input, H.265 policy, and browsers other than Chromium all need
hardware. `max_lag` is now load-bearing in both pacing directions, so a camera
with a long GOP or unusual timestamp behaviour is the test of whether one second
is the right bound. It is configuration, not a constant — a qualification
question, not a code question.

## 7. Time to first picture

Unchanged and not instrumented. A viewer waits for the next keyframe after
subscribing. Fold into item 4.

## 8. Latency, or an explicit decision not to have it

Unchanged. `TimestampAuthority::AdapterClock` makes a media-timestamp latency
claim structurally impossible through this adapter, deliberately. Either build a
viewer-side harness or record a decision that live latency is qualified by
observation rather than by metric. Do not add a latency metric to this path — it
would be fabricated.

## 9. Alert rules, including the new series

`promtool` is still not installed, so `docs/phase-8/alerts.yml` has never been
executed, and `surveillance_livekit_timestamp_discontinuities_total` has no rule.
The rule is not "greater than zero": one discontinuity per camera reconnect is
correct and expected. The alertable condition is a *rate* — discontinuities
accruing while the camera holds a single session, meaning a source whose
timestamps cannot be trusted for pacing. Choosing the window is a small product
decision, not a mechanical addition.

Also still open: `[~]` LiveKit server stop/restart while recording,
deterministically qualified. The runbook is written and unexercised.

## 10. Decide what clears an `Inconsistent` segment

Unchanged. `RecordingStatus::Inconsistent` may transition only to `Quarantined`
(`crates/storage/src/metadata.rs`), and `reconciliation_owner.rs` takes the
`Ok(_) => {}` no-op when a later probe succeeds for an already-flagged row. The
flag is permanent, nothing calls the only legal exit, and the local index holds
108 flagged rows whose files are present and probe cleanly.

The `Degraded → Publishing` edge added on 2026-08-17 is the same defect class
solved in the publication lifecycle. That does not settle this one — footage the
index once disagreed with is a higher bar than a transient pacing fault, and
re-blessing it silently is worse than leaving it flagged. It does mean the
question is "what is the audited operator path back", not "should there be one".

Needs a product decision: is re-indexing an explicit, audited, per-segment
operator action, or does a successful probe plus a matching size clear the flag
automatically? The second is convenient and quietly weakens the guarantee.

## 11. A reconnect mid-segment can bury playable footage

Unchanged. One occurrence: `ERROR recording metadata persistence rejected
immutable evidence category=Conflict`. The file is decodable (224 frames) with a
garbage container duration; the row is permanently `incomplete`/`reconciliation`;
the API 404s it; and `surveillance_metadata_rejected_full_total` is 0, so no
metric counts the rejection.

Smallest honest fix first: **count it.** A rejection no metric records is
invisible exactly where an operator looks. Worth doing independently of deciding
what the row should become. Then answer it together with item 10.

## Not yet on this list, and deliberately so

Phase 9 has 12 unchecked items and three drafted evidence documents
(`docs/phase-9/`), but its first item — an unoptimized one-camera baseline — is
untouched, and item 2 above means the live path's own throughput is not yet
understood. Starting capacity qualification on top of an unexplained 41% loss
would produce numbers that qualify the defect rather than the system.

## Do not do

- Do not `git clean`, `git checkout .`, or stash-and-drop in this tree until
  item 1 is done. See `repository-status.md` for what is at stake.
- Do not commit `.playwright-mcp/` or `makerun-live.png`. Ignore them.
- Do not delete `crates/livekit/src/native.rs`. It is ADR 012's reversal path and
  the cost of keeping it is one unused constructor.
- Do not raise `H264ToI420::WRITE_TIMEOUT` to make restarts go away. That is the
  steady-state overload detector; startup has its own budget.
- Do not raise `max_lag` to reduce timestamp discontinuities. It is the bound in
  both directions: raising it lengthens the pacing sleep the 2026-08-17 fix exists
  to bound, and re-admits the stall for smaller jumps.
- Do not treat `surveillance_livekit_timestamp_discontinuities_total` above zero
  as a fault. One per camera reconnect is correct behaviour. See item 9.
- Do not describe detection as working. It is still `DeterministicFakeInference`.
- Do not claim a camera count from any of this.
- Do not measure CPU per viewer until item 2 is closed.
- Do not clear the 108 `inconsistent` rows by hand or by a migration. They are the
  evidence for item 10, and the transition they would need does not exist.
- Do not describe live video as working without saying which case. Cold start and
  post-reconnect are both measured at `viewers` 0; with a viewer attached across a
  reconnect is item 4 and is unmeasured.
- Do not read the three `docs/phase-9/` documents as Phase 9 progress. None of
  their roadmap items is checked.
