# Driving the dashboard with an authenticated session — 2026-08-16

Two dashboard screens had never been exercised, and could not be: Playback and
Settings both call `/api/*`, the API is authenticated since Phase 8, and the
dashboard holds no credential by design (ADR 009 Decision 6). The only session
the page can have is one a trust-boundary proxy establishes in front of it.
`scripts/phase9/authenticated-ui-fixture.sh` stands that up, which is what makes
everything below possible.

Driving those screens found four defects. Three were in code that had been
reviewed, compiled, and shipped; none was reachable without an authenticated
session.

## What ran

```bash
WINDOW=3600 CAMERAS=1 scripts/phase9/authenticated-ui-fixture.sh
```

- macOS 15.7.7, Intel Core i9-9880H, FFmpeg 8.0.
- Headless Chromium 151.0.0.0 via Playwright. **Headless is a caveat, not a
  detail** — see *What this does not establish*.
- The fixture brings up a temporary PostgreSQL, `livekit-server`, MediaMTX with
  `testsrc2` sources, `surveillance-server` behind a trust boundary, and a
  Python header-injecting proxy. It refuses to declare itself ready unless the
  proxied API answers `200` **and** the direct listener answers `401`, so a run
  cannot silently degrade into an unauthenticated one.
- The proxy authenticates nobody. It exists to prove the origin the server
  already requires, not to model a deployment's login.

## Defects found and fixed

### 1. Every Playback search returned 400

The page sent `?from=…&to=…`; `recording_search` reads `start` and `end` and
answers `missing_recording_range` to anything else. Not a degradation — no
search of any range could ever succeed.

Fixed at both call sites in `apps/server/ui/dashboard.html`, and the parameter
names are now named once in `apps/server/src/health_http.rs`
(`RANGE_START`/`RANGE_END`) so a test can assert the shipped page against the
names the route actually reads. Nothing in the build had been checking that
joint, in either direction.

The regression test cannot drive the router: `recording_search` checks the
repository before parsing the range, so without PostgreSQL it answers `503
recording_index_unavailable` and never reaches parameter validation. The test
asserts the page's text against the constants instead. The `400` was observed
under this fixture; the test was mutation-checked by reverting the page to
`?from=` (fails) and restoring it (passes).

### 2. The add-camera form was wiped every five seconds

The fleet screen is in `LIVE_SCREENS`, so `render()` replaces all of `#main` on
every poll. The form's only copy of its contents was the DOM, so a camera
identifier and an RTSP URL had to be typed in under five seconds. They cannot
be. The form was unusable on the only screen it appears on.

Fixed by giving it module-scope backing state (`add`), the same pattern `pb`
already used for the playback filters.

### 3. Every live tile went black on every poll

Same root cause. The `<video>` element carrying the subscribed track was
destroyed and the track re-attached to a fresh element on each repaint, so every
tile alternated `640x360@4` with `0x0@0` — a new element has a cold decoder.

Fixed by lifting attached video nodes out of `#main` before the repaint and
putting them back after, rather than re-attaching to new ones.

### 4. `api.io_timeout` cancelled playback for any client that paces itself

Found by pressing Play. `stream_playback` bounded each body write by
`api.io_timeout` (5s). A media element stops reading the moment its buffer is
full and does not read again until it drains — every browser does this. The
server treated that healthy client as a stalled one, cancelled the assembler,
and closed the socket; the browser, handed only the seconds it had managed to
buffer, took that for the whole recording.

Confirmed by the server's own metric, not by inference:
`surveillance_playback_cancelled_total` incremented once per browser attempt
while `curl` — which drains as fast as it can — always completed.

Fixed with a separate `api.playback_stall_timeout` (default `60s`), because the
two budgets bound different things: `api.io_timeout` bounds a request or a JSON
response, where five seconds of silence really does mean the client is gone.
After the fix the same run reports `started 1, completed 1, cancelled 0`.

The `60s` default is a reversible guess and is documented as one in
`crates/config/src/defaults.rs` and `config/default.toml`. It is not a
measurement.

## What was verified

- **A camera added through the dashboard publishes to LiveKit.**
  `AddCameraRequest` now carries `live_publication_enabled`. Added through the
  form with publication on, then started: `camera-02 running connected
  lk_enabled=True lk_state=Publishing units=59`. The live grid then showed 2/2
  cameras connected, two subscriptions, both tiles `640×360` `readyState 4`,
  with `camera-01` visibly painting `testsrc2` frames in a screenshot.
- **A playing tile survives the poll.** The `<video>` node was stamped with a
  `data-` attribute and was still the same node, still `640x360@4`, still
  `liveAttached=yes`, after twelve two-second samples spanning roughly five poll
  cycles. Never `hidden`, never `0x0`.
- **Playback search returns real segments.** `GET
  …/recordings?start=…&end=…` → `200`, `12 contiguous segment(s), 53.3 MB`, each
  row `h264 640×360 ~4.5 MB completed`, timestamps 30 seconds apart. `ffprobe`
  on one of those files independently reports `h264 640×360 15/1 duration
  30.000000`, so the displayed metadata matches the media on disk.
- **The playback endpoint delivers correct media.** The assembled stream for a
  six-minute run is a single `ftyp`/`moov` with 180 `moof` and
  `duration=360.000000` per `ffprobe`, delivered in 0.35 s. Played from a bare
  `<video>` on the dashboard's own origin it runs continuously — sampled
  `3/0 6/3 9/6 … 36/33` (duration/currentTime), no rewind.
- **The Settings write path works, including its refusals.** With
  `api.mutating_surface_enabled` on: editing `retention.max_age` `30d → 21d` and
  saving gave `PUT 200`, generation `0 → 1`, and the value read back as `21d`
  with origin `persisted`. Saving `not-a-duration` gave `PUT 400`, surfaced in
  the page as `invalid_value`, with no generation change. Reverting to `30d`
  gave `PUT 200` and generation `2`. Accept, reject, and revert all observed
  through the UI.

## What this does not establish

- **Playback does not play through inside the dashboard page in this headless
  browser, and I could not explain why.** After fix 4 the server completes the
  stream (`completed 1, cancelled 0`) and a `fetch()` issued from that same page
  pulls all 69.8 MB in 1.1 s, yet the page's own element sits at
  `duration ≈ 4s`, `networkState = 1` (idle), rewinding to 0 and replaying those
  four seconds. A bare `<video>` given the identical URL on the identical origin
  plays through; a bare `<video>` appended to the *running* dashboard page
  stalls the same way. So it tracks the page being live, not the element, the
  URL, the response framing, the proxy, or the poll — clearing every interval
  did not change it. Most likely a headless main-thread contention artefact, but
  that is a hypothesis, not a finding. **Playback is not to be described as
  working end to end in the UI until this is reproduced in a real browser.**
- Nothing about a real browser, a real GPU, or any machine other than the one
  above.
- Nothing about real cameras. Every source was `testsrc2` at 640×360.
- No latency, throughput, or tile-count claim. Nothing here was measured for
  performance.
- Nothing about a deployment's authentication. The fixture proxy asserts an
  identity; it does not establish one. How an operator authenticates remains the
  open decision ADR 009 declines to guess.
- The `60s` playback stall budget has not been tuned against a slow client or a
  large run.

## Reproducing

```bash
WINDOW=3600 CAMERAS=1 scripts/phase9/authenticated-ui-fixture.sh
# then open http://127.0.0.1:3311/ui  (3310 is the server; it answers 401)
```
