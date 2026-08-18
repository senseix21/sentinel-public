# Next Actions — 2026-08-16 01:51 +06

The five items in [`../2026-08-15-2300/next-actions.md`](../2026-08-15-2300/next-actions.md)
are unchanged and still ordered correctly: confirm the deployment target, run
`promtool test rules docs/phase-8/alerts_test.yml`, test concurrent settings
updates, have a second person follow the runbooks, decide the first-admin
bootstrap path, decide on fuzzing. Nothing this session touched any of them.

What this session added, in the order it unblocks work:

## 1. Give `AddCameraRequest` a LiveKit field

This is now the cheapest item on the list with the largest reach. Because a
camera added through the API is created with publication disabled, only the
bootstrap-configured camera ever publishes. Consequences already recorded in
three documents:

- The N-tile measurement had to borrow five `lk --publish-demo` publishers, so
  it measures subscribers honestly and says nothing about the server at N
  publishing cameras (`docs/phase-9/live-grid-tiles-2026-08-16.md`).
- The dashboard's Live View cannot show a real fleet, only one real camera plus
  disabled tiles.
- ADR 010's "what this does not decide" carries it, and the 2026-08-15 load run
  recorded it before that.

Until it exists, no measurement of the publish side at N is reachable.

## 2. An automated browser test for the dashboard

Both operator-dashboard roadmap items are `[~]` for exactly one reason: nothing
in this workspace runs JavaScript, so no test verifies that a screen renders or
that a click works. Every defect fixed this session was found by a person
looking at the page. The two tests added to `health_http.rs` assert on the
shipped bytes, which catches a reverted line but not a broken render.

The gap is worth naming precisely: a headless-browser test needs a decision
about whether a browser becomes a build dependency of this repository. It is
not obviously yes.

## 3. Decide whether live viewing is a product capability

The token endpoint is still `development_only: true` and ADR 010 does not
change that. The grid now makes live viewing look like a finished feature to
anyone who opens the page, which raises the cost of leaving the question open.
It needs an authenticated token path, a concurrency decision, and a latency
objective — none of which exist, and no end-to-end glass-latency number has
ever been measured.

## 4. If the grid is kept, measure past six tiles

The per-tile figures are two homogeneous points. They do not establish that the
cost stays linear, and the grid density control offers 5× on purpose. A run at
10 and 20 tiles on target hardware would either justify a cap or prove one is
unnecessary; today a cap would be a fabricated limit and the ADR says so.

## Housekeeping

The verification server on `127.0.0.1:3301` and its `surveillance_dashboard_ui`
database were torn down at the end of this session. The six-camera fixture
(`scripts/phase9/livekit-load-fixture.sh`) tears itself down when `WINDOW`
elapses; it has.
