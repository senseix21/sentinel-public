# ADR 010: A LiveKit Browser Client in the Operator Dashboard

Status: accepted; the SDK version is pinned and its digest is enforced by a test

Date: 2026-08-15

Closes `docs/phase-8/ui-spec.md` §7 gap 5, which recorded that the dashboard
issued a viewer token and then had nothing to do with it.

## Context

Known facts about the tree as of this date:

- `apps/server` publishes each camera to LiveKit through one `lk room join
  --publish h264:///<socket>` child per camera, into room
  `<namespace>-<camera-id>-live`. This is an independent consumer of the media
  distributor; its failure does not touch recording.
- `POST /api/cameras/<id>/viewer-token` authorizes through
  `Action::IssueViewerToken`, then returns `endpoint`, `room`, `token`,
  `expires_in_seconds`, and `development_only: true`.
- Before this change, `apps/server/ui/dashboard.html` rendered that response as
  JSON on the page, with a comment saying a player would be a mock because no
  SDK was vendored. So the one screen an operator would use to check a camera
  showed a credential and no video.
- `apps/server/ui/dashboard.html` is `include_str!`d into the binary and served
  at `/` and `/ui`. `CLAUDE.md` states the constraint directly: "No build step
  and no external request — do not introduce either."
- The test `the_dashboard_fetches_nothing_from_outside_this_server` enforces the
  second half of that by scanning the shipped bytes for `http://`, `https://`,
  protocol-relative `src`/`href`, and `@import`.
- The deployment target is unconfirmed (ADR 009). A self-hosted unit on a
  segmented network is the assumed environment.

WebRTC playback cannot be hand-rolled at a proportionate cost. Subscribing to a
LiveKit room means signalling over its WebSocket protocol, ICE, DTLS-SRTP, and
the server's own participant and track-publication semantics. That is a client
library, not a fetch call.

## Decision 1 — Ship a real player, not a token dump

The dashboard connects to the room and attaches the subscribed video track to a
`<video>` element. The viewer token is used to connect and is never written into
the DOM.

Printing it was the worse of the two failures it caused. A viewer token grants
live footage of a physical space to whoever holds it, and on that page it
survived into any screenshot or shoulder-glance. `a_viewer_token_is_never_
rendered_on_the_page` asserts the token appears exactly once in the shipped
bytes and that the occurrence is the `room.connect` call.

The player keeps the development-only labelling. Nothing about issuing tokens
from this endpoint changed; it is still gated by `IssueViewerToken` and still
declares `development_only: true`, and the page still says so.

## Decision 2 — Vendor the SDK into the repository, not a CDN

`livekit-client` 2.21.0's prebuilt UMD bundle is committed at
`apps/server/ui/vendor/livekit-client.umd.js` (561,757 bytes, Apache-2.0),
`include_str!`d into the binary, and served from `/ui/livekit-client.js`.
Provenance, version, license, digest, and the two commands that reproduce it are
recorded in `apps/server/ui/vendor/README.md`.

Options considered:

1. **Load it from a CDN.** Smallest diff, no vendored blob to review or update.
   Rejected: it fails closed in exactly the environment this product targets — a
   self-hosted unit on a segmented network, where the dashboard would load and
   the player would not. It also hands a third party one request per operator
   per page load, from a surveillance console, and makes every page load depend
   on a host outside the operator's control. It contradicts a stated repository
   constraint.
2. **Add a frontend build step and bundle from `node_modules`.** Produces a
   smaller, tree-shaken bundle. Rejected: it introduces npm, a lockfile, a
   toolchain version, and a build stage into a repository whose UI is one file
   with no build step, in exchange for bytes that are already free — the asset
   is served over loopback from memory. It also makes the shipped JavaScript no
   longer readable in the repository.
3. **Implement a minimal WebRTC subscriber by hand.** No dependency. Rejected:
   it is a re-implementation of the LiveKit signalling protocol that would have
   to track the server's version, with no test coverage available here and a
   large surface for silent breakage. The cost is out of proportion to a
   development-only viewer.
4. **Vendor the prebuilt UMD bundle.** Chosen. Zero runtime requests outside
   this server, no build step, the exact bytes visible in review, and an
   upgrade is a deliberate two-command change with a recorded digest.

The trade-off accepted is a 561 KB binary artifact in version control that
cannot be meaningfully diffed, and an SDK upgrade that is manual rather than
driven by a dependency resolver. Both are mitigated by recording the digest and
enforcing it: `the_live_player_sdk_is_served_as_the_bytes_that_were_vendored`
computes SHA-256 over the shipped constant and compares it to the value in
`ui/vendor/README.md`, so a modified bundle fails the build unless its
provenance is updated in the same change.

## Decision 3 — Fetch the SDK on the first press, and never cache it

The bundle is not referenced from the markup. `loadLiveKit()` injects a
`<script src="/ui/livekit-client.js">` the first time "Watch live" is pressed,
memoizing the promise; a load failure clears the memo so a later press retries.

Live viewing is opt-in on one panel of one screen, and the fleet view is the
screen an operator watches. Half a megabyte does not belong in its critical
path.

The asset is served by the same `response()` helper as everything else and
therefore carries `Cache-Control: no-store`. That is deliberate and consistent
with the dashboard itself: the SDK version is fixed at compile time, so a cached
copy after an upgrade means a browser running the previous client against the
current server, with nothing on screen to indicate it. Over loopback the re-fetch
costs nothing worth trading for that.

## Decision 4 — Live sessions are owned in one place and torn down explicitly

*Amended 2026-08-15 by Decision 5. The rule below is unchanged; what changed is
that "at most one" became "one per watched camera".*

A module-level `live = { rooms: Map }` is the only thing in the page that holds a
`Room`. `startLive(cameraId, video)` stops that camera's existing session first;
`stopLive(cameraId)` disconnects it and clears the element; `stopAllLive()`
covers every entry. The `hashchange` handler tears down **before** `render()`
runs.

This mirrors the ownership rule the server side is held to. `render()` replaces
the view wholesale, so every element a track is attached to stops existing;
without an explicit teardown the peer connections and the subscriptions outlive
the navigation, and the server keeps counting viewers who left.
`every_live_session_is_torn_down_before_the_view_it_lives_in_is_replaced`
asserts the ordering, and asserts that the teardown iterates the map rather than
one entry.

`Disconnected` and the superseded-session check both compare
`live.rooms.get(cameraId) !== room` before mutating shared state, so a session
that is replaced while connecting cannot clear its successor's state.

## Decision 5 — A grid of concurrent rooms, with no tile-count claim

The operator design is a wall of cameras: the fleet is watched at once, not one
camera at a time. Decision 4 as originally written made that impossible — a
second tile disconnected the first — so the grid required the player to own N
rooms rather than one.

Options considered:

1. **Keep one room and let the operator switch cameras.** No new cost, no new
   measurement. Rejected: it is not the requested capability. A surveillance
   console whose defining screen shows one camera at a time is a different
   product.
2. **One room, cycling tiles on a timer.** Bounds the cost at one subscription.
   Rejected: every tile is stale most of the time and nothing on screen says
   which. That is worse than showing fewer cameras honestly.
3. **N concurrent rooms, one per tile, owned in the same map.** Chosen. It is
   what the design asks for, and it keeps a single ownership site.

What this buys is bounded by what it costs. A first measurement now exists —
`docs/phase-9/live-grid-tiles-2026-08-16.md` — and it moves the open question
rather than closing it.

On one Intel i9-9880H host, each subscribed 1280×720 30 fps tile cost the
browser process tree about **0.17 of one core** and the SFU about **0.046**,
while `surveillance-server` stayed between 4.5% and 5.9% of one core across
every configuration from zero to six tiles. The subscriber side is where the
cost lands; the ingest-once topology holds. Memory grew substantially with tiles
but the readings are single instants and no per-tile figure is claimed.

**This is still not a tile-count claim.** Only two homogeneous points (one tile
and five) were run, nothing above six was tried, and five of the six tracks came
from an `lk --publish-demo` publisher rather than a camera — at the time of the
measurement `AddCameraRequest` carried no LiveKit field, so six real camera
streams could not be put on the screen at all. That field now exists (see *What
this does not decide*), which removes the obstacle but does not retroactively
change what was measured: these numbers are still five demo publishers and one
camera. Nothing here establishes how many tiles a given machine sustains, at
what resolution, or with what effect on the publishers.

Consequences taken deliberately:

- **No tile joins on render.** Joining issues a viewer token, subscribes a real
  consumer, and writes an audit row; doing that on navigation would make idle
  browsing generate live sessions. A press is the only entry point, for one tile
  or, through the same call, for all of them.
- **The grid density control is presentation only.** It changes columns. It does
  not cap subscriptions, because a cap chosen without a measurement would be a
  fabricated limit.
- **No claim is made about supported concurrency.** The engineering policy
  forbids a camera-count claim from architecture, and this is the same claim in
  a different coordinate. Until a documented workload exists, the honest
  statement is that the page will attempt what it is told to attempt.

The measurement this needs — N tiles against N publishers, recording CPU,
memory, dropped frames on the live queue, and the effect on
`recorder_rejected_full` — has not been run. `docs/phase-9/livekit-load-2026-08-15.md`
already records that live publication decimates at the configured live-queue
capacity and that the recorder's backpressure stops ingest under sustained load;
both bear directly on what a grid costs, and neither was introduced by this
change.

## What this does not decide

- **Whether live viewing is a supported production capability.** It is not. The
  token endpoint remains development-only, and this ADR does not change its
  posture. Production live viewing needs an authenticated token path, a
  concurrency decision, and a latency objective — none of which exist.
- **Viewer-perceived latency.** No end-to-end live latency histogram exists, so
  no p50/p90/p99 claim is available. See
  `docs/phase-9/livekit-load-2026-08-15.md`, which records this explicitly as
  not measured.
- **Concurrent viewers.** One session per browser tab is enforced here. Nothing
  measures what N viewers cost the unit.
- **Cameras added through the API.** *Resolved after this ADR was accepted.*
  `AddCameraRequest` now carries `live_publication_enabled: Option<bool>` — one
  bit, no endpoint and no credential. Absent means "follow the deployment", so a
  camera added where LiveKit is configured publishes; asking to publish where it
  is not configured is refused with `live_publication_not_configured` rather
  than accepted and silently disabled. The deployment template is held by the
  process owner in `CameraDefaults` and stamped onto the camera, which is what
  keeps `POST /api/cameras` from being a route by which a caller supplies,
  learns, or substitutes a LiveKit secret. Verified in a browser through
  `scripts/phase9/authenticated-ui-fixture.sh`; evidence in
  `docs/phase-9/authenticated-ui-2026-08-16.md`.

## Reversal criteria

Reverse Decision 2 (vendoring) if the deployment target turns out to be a
network with reliable egress *and* a packaging story that fetches assets at
build time — at which point the bundle moves into that build and out of version
control, and the digest test moves with it.

Reverse Decision 1 (the player) if live viewing is removed from the product, or
if the viewer-token endpoint is withdrawn before an authenticated replacement
exists. In that case the panel returns to reporting publication state only —
not to printing a token.

Upgrade path for the SDK: repeat the two commands in `ui/vendor/README.md` with
the new version, update version, digest, and size in that file, and re-run
`make check`. The digest test fails until the provenance record matches, which
is the intended forcing function.
