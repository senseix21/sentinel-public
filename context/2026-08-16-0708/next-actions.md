# Next Actions — 2026-08-16 07:08 +06

The five carried items in [`../2026-08-15-2300/next-actions.md`](../2026-08-15-2300/next-actions.md)
are unchanged and still ordered correctly: confirm the deployment target, run
`promtool test rules docs/phase-8/alerts_test.yml`, test concurrent settings
updates, have a second person follow the runbooks, decide the first-admin
bootstrap path, decide on fuzzing. Nothing this session touched any of them.

Item 1 of [`../2026-08-16-0151/next-actions.md`](../2026-08-16-0151/next-actions.md)
— give `AddCameraRequest` a LiveKit field — is **done**. Items 2, 3 and 4 stand
and are restated below where this session changed their weight.

## 1. Reproduce the playback stall in a real browser

The only thing this session could not explain, and the only reason the Playback
screen is not `[x]`. The server completes the stream, the same page can `fetch()`
all 69.8 MB in about a second, and the same URL plays through in a bare
`<video>` on the same origin — but the page's own element stalls at ≈4 s and
rewinds. A bare `<video>` appended to the running page stalls too, so it tracks
the page being live rather than the element or the URL.

The cheapest next step is one person opening `http://127.0.0.1:3311/ui` in a
real browser under `scripts/phase9/authenticated-ui-fixture.sh` and pressing
Play. Either it plays, in which case the artefact is headless and the screen can
be claimed; or it does not, in which case there is a real defect the automated
walk narrowed to the page's own environment. Both outcomes are worth more than
more headless work.

Until then: **playback is not to be described as working end to end in the UI.**

## 2. An automated browser test for the dashboard

Unchanged from the previous snapshot, and now more clearly worth having. Every
defect in the last two sessions — seven of them — was found by a person driving
the page. The tests that exist assert on the shipped bytes; that catches a
reverted line, not a broken render, and it did not catch a search that answered
`400` for every range or a form that erased itself every five seconds.

The blocker is still a decision, not effort: does a browser become a build
dependency of this repository? It is not obviously yes. If it is no, then the
byte-level tests plus a documented manual walk is the honest ceiling and should
be written down as such.

## 3. Tune `api.playback_stall_timeout` against a real client

New. The `60s` default is a guess sized to be longer than any media buffer
observed here and short enough that a departed client releases its assembler
child within a minute. It has never been tested against a genuinely slow client,
a long range, or a client that goes away mid-stream. The setting exists and is
`Immediate`, so tuning it costs nothing once there is a number to tune it to.

## 4. Decide whether live viewing is a product capability

Unchanged, and the same argument as before: the token endpoint is still
`development_only: true`, and the grid now makes live viewing look finished to
anyone who opens the page. It needs an authenticated token path, a concurrency
decision, and a latency objective. None exists, and no end-to-end glass-latency
number has ever been measured.

## 5. If the grid is kept, measure past six tiles

Unchanged. Two homogeneous points do not establish linearity, and the density
control offers 5× on purpose. The one thing that did change: with
`AddCameraRequest` carrying `live_publication_enabled`, a re-run can now use real
publishing cameras instead of borrowing `lk --publish-demo` publishers, so the
publish side is measurable for the first time. The existing numbers were **not**
re-taken on the newer binary and are unchanged by it.

## Housekeeping

Torn down at the end of this session: both `authenticated-ui-fixture.sh` runs
(the current one and an orphaned earlier one), their `livekit-server`, MediaMTX,
`ffmpeg` publisher and `surveillance-server` children, their two temporary
PostgreSQL instances (`pg_ctl -m fast stop`, both reported `server stopped`), the
two scratchpad static file servers, and all five
`~/.cache/tmp/surveillance-phase9-ui.*` directories (1.07 GB). No fixture process
remains.

The `surveillance_webcam` database from the earlier webcam loop is still present
and was left alone; it predates this session's work.
