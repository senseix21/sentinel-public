# Repository Status — 2026-08-16 07:08 +06

Commit at snapshot: `e65a318` ("docs: add Phase 8 handoff"). The worktree is
still intentionally dirty and nothing has been committed. This snapshot
continues [`../2026-08-16-0151/repository-status.md`](../2026-08-16-0151/repository-status.md);
read that one for the operator-dashboard rebuild and the N-tile measurement,
neither of which is repeated here.

## Verified this session

`make check` and its parts, all green:

```
cargo fmt --all --check                                               ok
cargo clippy --workspace --all-targets --all-features -- -D warnings  ok
cargo test --workspace --all-features   469 passed; 0 failed; 32 ignored
                                        (43 suites)
cargo build --workspace --release                                     ok
make audit                                                            exit 0
```

`cargo fmt --all --check` failed once mid-session on my own edits; `cargo fmt
--all` then a re-check cleared it. The `cargo audit` warning is the same allowed
one as before: RUSTSEC-2025-0119, unmaintained `number_prefix`, reached through
`indicatif` ← `muxide` ← `surveillance-recorder`.

Test count moved 460 → 469. Ignored tests are unchanged and still require
FFmpeg/libx264, MediaMTX, PostgreSQL, or a LiveKit server.

## What this session was for

The previous snapshot's "Not verified" list named three things. All three are
now addressed:

1. `AddCameraRequest` carries no LiveKit field — **removed as a limitation**.
2. Playback with real segments — **verified**, after fixing two defects.
3. The Settings write path — **verified**, including its refusals.

All three needed the same missing thing: an authenticated session. The dashboard
holds no credential by design (ADR 009 Decision 6), so the only session it can
have is one a trust-boundary proxy establishes in front of it. Standing that up
is what made the rest reachable.

## What changed

**`scripts/phase9/authenticated-ui-fixture.sh`** — new. Brings up a temporary
PostgreSQL, `livekit-server`, MediaMTX with `testsrc2` sources,
`surveillance-server` behind a trust boundary, and a header-injecting Python
proxy. It refuses to declare itself ready unless the proxied API answers `200`
**and** the direct listener answers `401`, so a run cannot silently degrade into
an unauthenticated one and then be mistaken for evidence. The proxy authenticates
nobody; it proves the origin the server already requires.

It also sets `SURVEILLANCE_API_MUTATING_SURFACE_ENABLED=true`, because ADR 008
ships that surface disabled and `PUT /api/settings` answers `404` while it is —
without the opt-in the Settings screen cannot be exercised at all. A fixture
opting in says nothing about what a deployment should do.

**`apps/server/ui/dashboard.html`** — playback search parameter names fixed at
both call sites; module-scope `add` state for the add-camera form; attached
`<video>` nodes lifted out of `#main` across a repaint instead of being
destroyed.

**`apps/server/src/health_http.rs`** — `RANGE_START`/`RANGE_END` named once so a
test can assert the shipped page against the names the route actually reads; a
new `playback_stall_timeout()` read per request; three test additions including
`playback_is_not_bounded_by_the_json_response_timeout`.

**`crates/config`** and **`config/default.toml`** — new
`api.playback_stall_timeout` setting (`Operational`, `Immediate`, `Public`,
`Global`, `Duration`), default `60s`. The two row-count and default-file guards
in `crates/config` both caught the incomplete change first, which is what they
are for.

**`docs/phase-9/authenticated-ui-2026-08-16.md`** — new; the evidence.
**`ROADMAP.md`** — one new `[~]` item under "Operator dashboard", and the stale
`AddCameraRequest` sentence in the tile-cost item corrected. **`AGENTS.md`** —
`Last verified` and the test counts. **`docs/phase-9/live-grid-tiles-2026-08-16.md`**
and **`docs/phase-9/livekit-load-2026-08-15.md`** — the same stale sentence put
into past tense, with an explicit note that the numbers were not re-measured on
the newer binary and do not change.

## Defects found by driving an authenticated session

Three of the four were in code that had been reviewed, compiled and shipped.
None was reachable without the session.

1. **Every Playback search returned `400`.** The page sent `?from=…&to=…`;
   `recording_search` reads `start` and `end`. Not a degradation — no search of
   any range could ever have succeeded. Nothing in the build was checking that
   joint in either direction.
2. **The add-camera form was wiped every five seconds.** The fleet screen is in
   `LIVE_SCREENS`, so `render()` replaces `#main` on every poll, and the form's
   only copy of its contents was the DOM. A camera identifier and an RTSP URL had
   to be typed in under five seconds. They cannot be.
3. **Every live tile went black on every poll.** Same root cause: the `<video>`
   carrying the subscribed track was destroyed and the track re-attached to a
   fresh element with a cold decoder, so each tile alternated `640x360@4` with
   `0x0@0`.
4. **`api.io_timeout` cancelled playback for any client that paces its reads.**
   Found by pressing Play. A media element stops reading the moment its buffer is
   full and does not read again until it drains — every browser does this. The
   five-second budget expired against that healthy client, the assembler was
   cancelled and the socket closed, and the browser took the few buffered seconds
   for the whole recording. Confirmed by
   `surveillance_playback_cancelled_total`, which incremented once per browser
   attempt while `curl` — which drains as fast as it can — always completed.
   After the fix the same run reports `started 1, completed 1, cancelled 0`.

## What was verified

- A camera added through the dashboard form with publication on then started:
  `camera-02 running connected lk_enabled=True lk_state=Publishing units=59`.
  The grid then showed 2/2 connected, two subscriptions, both tiles `640×360`
  `readyState 4`, `camera-01` visibly painting frames in a screenshot.
- A playing tile survives the poll: the `<video>` node was stamped with a
  `data-` attribute and was the same node, still `640x360@4`, after twelve
  two-second samples across roughly five poll cycles.
- Playback search returns real segments: `200`, `12 contiguous segment(s),
  53.3 MB`, each row `h264 640×360 ~4.5 MB completed`, 30 seconds apart.
  `ffprobe` on one of those files independently reports `h264 640×360 15/1
  duration 30.000000`, so the displayed metadata matches the media on disk.
- The playback endpoint delivers correct media: one `ftyp`/`moov`, 180 `moof`,
  `duration=360.000000`, delivered in 0.35 s, and played continuously from a
  bare `<video>` on the dashboard's own origin (`3/0 6/3 9/6 … 36/33`).
- The Settings write path: `retention.max_age` `30d → 21d` gave `PUT 200`,
  generation `0 → 1`, value read back `21d` origin `persisted`;
  `not-a-duration` gave `PUT 400` surfaced as `invalid_value` with no generation
  change; reverting gave `PUT 200` and generation `2`.

## Not verified

- **Playback does not play through inside the running dashboard page in headless
  Chromium, and I could not explain why.** The server completes the stream and a
  `fetch()` from that same page pulls all 69.8 MB in 1.1 s, yet the page's
  element sits at `duration ≈ 4s`, `networkState = 1`, rewinding. Ruled out: the
  element, the URL, the response framing, the content-length, the fixture proxy
  (rewritten to stream, then bypassed entirely — the hypothesis that it caused
  this was falsified and is recorded as falsified), the poll interval, and the
  user-gesture requirement. A bare `<video>` appended to the *running* page
  stalls the same way, so it tracks the page being live. Headless main-thread
  contention is a hypothesis, not a finding. **Playback is not to be described as
  working end to end in the UI until a real browser reproduces it.**
- Still no automated browser test. Everything above was driven by hand, in one
  headless browser, in one session. The roadmap items stay `[~]`.
- Every source was `testsrc2` at 640×360. Nothing about real cameras, a real
  GPU, or any machine other than this one.
- The `60s` playback stall budget is a reversible guess documented as one in
  `crates/config/src/defaults.rs` and `config/default.toml`. It has not been
  tuned against a slow client or a large run.
- Nothing about a deployment's authentication. The fixture proxy asserts an
  identity; it does not establish one.
