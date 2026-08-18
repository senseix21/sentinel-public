# Operator UI — Specification

Written 2026-08-15 for handing to a designer. Every field, endpoint, state, and
error below is taken from the running server, not from an intended design. Where
something does not exist, it says so rather than describing it aspirationally.

**This UI does not exist.** No frontend code is in the repository. This is a
specification for one.

---

## 1. Read this before designing anything

Four constraints shape the whole product. They are not negotiable by the design.

### 1.1 There is no working detection

Detection runs `DeterministicFakeInference` — a fixture. The pipeline is real:
frames are decoded, sampled, and turned into structured events with bounding
boxes and confidences. But the *inference* is fake, so **no screen may present
a detection as a fact about a person.**

Do not design: "3 people detected", a person-count trend, a face gallery, an
alert on a person appearing. Do design: a clearly-labelled developer/diagnostic
view if one is wanted at all. This is the single easiest way to ship something
dishonest, because the data looks completely real.

### 1.2 Live viewing is not qualified

LiveKit publication works for synthetic H.264 and a webcam. It has **not** been
qualified for representative cameras, browser playback, or any latency target.
Viewer tokens are labelled development-only in the code.

Design live view as a first-class screen if you like, but it cannot be presented
as a supported capability yet, and there is no latency number to display.

### 1.3 Playback is segment-aligned, not frame-accurate

The server responds with `X-Playback-Accuracy: segment-aligned`. A request for
`10:00:05–10:00:35` returns whole 30–60 second segments covering that range, so
the media starts *before* the requested start and ends *after* the requested
end. The response also carries `X-Requested-Start` and `X-Requested-End`.

The UI must show the user what they actually got, not pretend the trim was
exact. A scrubber that silently implies frame accuracy is a lie the server is
being careful not to tell.

### 1.4 Recording is priority 1, everything else degrades

`AGENTS.md` orders the subsystems: **recordings > camera connectivity > live
view > detection > analytics.** The UI should make that ordering visible. A
camera that is recording but whose live view is broken is *mostly healthy*, and
showing it in red trains operators to ignore red.

---

## 2. Authentication — decided: the proxy does it

**Decided 2026-08-15 (ADR 009 Decision 6). The UI contains no authentication
code at all.**

The reverse proxy authenticates the human by whatever means its operator already
runs, and asserts the result to this server in `X-Authenticated-Principal`. The
server reads that header only after proof of origin holds — the peer allowlist
*and* the constant-time shared-secret header from ADR 009 Decision 2 — so an
assertion that did not come through the proxy is discarded rather than
distrusted later.

What this means for the front end, concretely:

- **There is no login screen.** By the time a request reaches the UI, the proxy
  has already authenticated the user or refused them. Do not build one.
- **There is no token in JavaScript.** Nothing to put in `localStorage`, nothing
  to refresh, nothing for an XSS to steal. This was the whole point.
- **There is no logout button that this system can honour.** The session belongs
  to the proxy. If logout is required, it links to the proxy's logout URL, and
  that URL is deployment configuration — not a constant in the bundle.
- **There is no CSRF token,** because there is no ambient credential this
  server issued. The proxy's own session cookie is the proxy's problem.
- **A `401` is not a login prompt.** It means the proxy asserted somebody this
  system does not hold, or holds but has suspended. Show "your account is not
  provisioned for this system — contact your administrator", and do not retry:
  retrying cannot change the answer.

The UI still needs to know *who* it is showing, for the header and for deciding
which controls to render. Read that from `GET /api/cameras` and the role-gated
surfaces, not from a decoded token — the UI has no token to decode.

> Service clients and the CLI keep using `Authorization: Bearer <token>`. That
> path is unchanged and takes precedence when both are present.

---

## 3. Roles — what each user can actually see

Three roles, enforced server-side. The UI must reflect them, but **must not rely
on them for security** — the server refuses regardless.

| Role | Can |
|---|---|
| **Viewer** | List cameras, read camera health, search recordings, watch playback, get a live viewer token |
| **Operator** | Everything a Viewer can, plus start/stop/add/remove cameras |
| **Administrator** | Everything an Operator can, plus settings and identity management |

**Camera scope** is separate from role. A principal is scoped to `all` cameras
or to an explicit list. An out-of-scope camera returns **`404`, not `403`** —
deliberately, so scope cannot be used to enumerate the estate.

**Design consequence:** the UI must never say "you don't have access to
camera-07". It doesn't know camera-07 exists. Out-of-scope cameras are simply
absent from every list — `GET /api/cameras` is already filtered server-side.

---

## 4. Screens

### 4.1 Fleet overview — the landing screen

**Purpose:** one glance answers "is anything wrong, and where".

**Data:** `GET /api/cameras` → array of camera objects (§5.1).

Per camera, the fields that matter for a card:

- `camera_id`, `pipeline_state`, `connection_state`
- `last_media_unix_ms` — **the honest liveness signal.** A camera can be
  `connected` and sending nothing. "Connected" alone is not health.
- `recorder_completed_segments`, `recorder_failed_segments`
- `last_actionable_failure` — a short string, safe to display, credential-free
  by construction
- `reconnects` — a rising count means flapping

**Health rollup.** Derive one status per camera, in priority order:

| Shown as | Condition |
|---|---|
| **Failed** | `pipeline_state` is `terminal`, or `recorder_failed_segments` rising |
| **Not recording** | `connection_state` is `connected` but no new segment in > 2× segment duration |
| **Disconnected** | `connection_state` is not `connected` |
| **Degraded** | `pipeline_state` is `degraded`, or live/detection unhealthy while recording continues |
| **Healthy** | recording, connected, no failures |

"Not recording" deserves the loudest treatment even though nothing looks broken.
It is the failure mode the system exists to prevent, and the one an operator
will not notice on their own.

**Also on this screen:** storage pressure (`Normal`/`Warning`/`Critical`/
`Emergency`) and whether the recording index is available. Both come from
`/metrics` today, **not** from a JSON endpoint — see §7 gaps.

### 4.2 Camera detail

**Data:** `GET /api/cameras/{id}` and `GET /api/cameras/{id}/health` (currently
the same payload).

Sections, in the subsystem priority order:

1. **Recording** — segments completed/failed, recorder queue depth vs capacity,
   `recorder_rejected`. A rising recorder queue is the one an operator acts on;
   live and detection queues drop by design.
2. **Connection** — state, `connection_attempts`, `connection_epochs`,
   `reconnects`, `last_connected_unix_ms`, `last_media_unix_ms`,
   `last_actionable_failure`.
3. **Live** — `live_publication_enabled`, `live_publication_state`,
   `live_publication_failure`, `live_published_access_units` against
   `live_offered`, and `live_publication_adapter_drops`. Present as
   *secondary*; its failure is not an incident. The published count is shown
   against the offered count because a publisher reporting `Publishing` while
   almost nothing reaches a subscriber is otherwise indistinguishable from a
   working live view.
4. **Detection** — `detection_enabled`, `detection_state`, queue depth,
   `detection_dropped`, `detection_inferred`. **Label as fixture output**
   (§1.1).

**Actions** (Operator+): Start, Stop, Remove. Remove is destructive and must
confirm. Each maps to a `POST`/`DELETE` in §5.

### 4.3 Recording search and playback

**Data:** `GET /api/cameras/{id}/recordings?start=&end=` → `{recordings[],
next_cursor}`.

Per recording: `recording_id`, `started_at`, `ended_at`, `codec`, `width`,
`height`, `size_bytes`, `status`, and `playback` (a URL).

**Design notes:**

- Times are RFC 3339 strings in **UTC**. Storage paths are UTC by design. Decide
  and state explicitly whether the UI shows UTC or local — an operator reviewing
  an incident with a timestamp from another system needs to know which.
- `next_cursor` means pagination is cursor-based. Infinite scroll or "load more";
  not page numbers.
- A timeline strip showing segment coverage over a day is the natural
  presentation, and it makes **gaps** visible — which is exactly what someone
  investigating an incident is looking for.
- Playback is `GET .../recordings/playback?start=&end=`, returning
  `video/mp4`, `Transfer-Encoding: chunked`. It is a live stream, so **seeking
  within the response is not supported.** Design the player around
  play-from-a-chosen-range, not a scrubbable file.
- Respect §1.3: show requested range vs delivered range.

**Every playback is audited as an export**, with camera, exact range, and byte
count. Consider telling the user that — it is honest, and it deters casual
browsing of footage.

### 4.4 Settings (Administrator)

**Data:** `GET /api/settings` → `{cache, freshness, generation, settings[]}`.

Per setting: `key`, `value`, `origin` (`default`/`file`/`environment`/
`persisted`), `class`, `boundary`, `secret` (bool).

Three things the design must handle, all unusual:

1. **`boundary` is when a change takes effect**: `immediate`,
   `next_segment`, `next_reconnect`, `pipeline_restart`, `process_restart`.
   The UI must show this *before* the user saves. "Saved" without "takes effect
   on next restart" is misleading.
2. **`secret: true` never shows a value.** The API returns a redaction. Show a
   fingerprint and its source, never a masked input pretending to hold the real
   value.
3. **Updates are whole-batch transactional.** `PUT /api/settings` with
   `{"changes": {...}}` either applies everything or rejects everything, naming
   the offending key. Design the error state around *one named offender*, not
   per-field inline validation.

Also surface `freshness` — `Stale` means the settings store was unreachable and
the configuration came from a last-known-good cache. Recording is unaffected;
say so, or an operator will panic.

### 4.5 Identity management (Administrator)

**Data:** `GET /api/identities`, `GET /api/identities/{id}/tokens`.

Per principal: `principal_id`, `kind` (`human`/`service`), `display_name`,
`role`, `scope` (`"all"` or an array of camera ids), `enabled`, `live_tokens`.

Per token: `token_id`, `label`, `expires_at_unix`, `revoked_at_unix`. **Never a
secret** — the server does not hold one in usable form.

**The one screen that needs real care:** minting a token returns the plaintext
**exactly once**, in the `201` response. It is not recoverable. The UI must:

- show it prominently with a copy affordance,
- state plainly that it will not be shown again,
- require an explicit acknowledgement before navigating away,
- never write it to `localStorage`, a URL, or an analytics event.

Revocation takes effect immediately, with no restart and no cache — verified
live. The UI can promise that.

### 4.6 Audit log (Administrator)

**No API endpoint exists.** The `access_audit` table is populated and correct
(category, action, actor, camera, outcome, reason, client address, and for
exports the range and byte count) but nothing exposes it over HTTP. See §7.

Design it — it is the most valuable screen for an incident — but know that it
needs a server endpoint built first.

---

## 5. API reference

Base: the server's bind address, loopback by default. All responses
`application/json` unless noted.

### 5.1 Camera object

```json
{
  "camera_id": "camera-01",
  "pipeline_state": "running",
  "connection_state": "connected",
  "connection_attempts": 12,
  "connection_epochs": 1,
  "reconnects": 0,
  "terminal_failures": 0,
  "last_connected_unix_ms": 1786779000000,
  "last_media_unix_ms": 1786779002500,
  "last_actionable_failure": null,
  "recorder_queue_depth": 2,
  "recorder_queue_capacity": 256,
  "recorder_rejected": 0,
  "live_queue_depth": 0,
  "live_queue_capacity": 64,
  "live_offered": 0,
  "live_dropped": 0,
  "detection_enabled": false,
  "detection_state": null,
  "detection_queue_depth": null,
  "detection_queue_capacity": null,
  "detection_dropped": 0,
  "detection_inferred": 0,
  "detections_published": 0,
  "recorder_completed_segments": 41,
  "recorder_failed_segments": 0,
  "live_publication_enabled": false,
  "live_publication_state": null,
  "live_publication_failure": null,
  "live_publication_timestamp_authority": null,
  "live_published_access_units": 0,
  "live_published_bytes": 0,
  "live_publication_adapter_drops": 0
}
```

`pipeline_state`: `configured` `starting` `running` `stopping` `stopped`
`degraded` `terminal`.
`connection_state`: `disconnected` `connecting` `connected` `reconnecting`
`stopped` (nullable).

### 5.2 Endpoints

| Method | Path | Role | Notes |
|---|---|---|---|
| `GET` | `/live` | — | Liveness. No auth. |
| `GET` | `/ready` | — | Readiness + per-camera degradation. `503` when degraded. No auth. |
| `GET` | `/metrics` | — | Prometheus text. No auth. |
| `GET` | `/api/cameras` | Viewer | **Filtered by scope** |
| `POST` | `/api/cameras` | Operator | |
| `GET` | `/api/cameras/{id}` | Viewer | |
| `GET` | `/api/cameras/{id}/health` | Viewer | |
| `DELETE` | `/api/cameras/{id}` | Operator | Destructive |
| `POST` | `/api/cameras/{id}/start` | Operator | |
| `POST` | `/api/cameras/{id}/stop` | Operator | |
| `POST` | `/api/cameras/{id}/viewer-token` | Viewer | Development-only |
| `GET` | `/api/cameras/{id}/recordings?start=&end=` | Viewer | Cursor paginated |
| `GET` | `/api/cameras/{id}/recordings/{rid}` | Viewer | |
| `GET` | `/api/cameras/{id}/recordings/playback?start=&end=` | Viewer | `video/mp4`, chunked, audited as export |
| `GET` | `/api/settings` | Administrator | |
| `GET` | `/api/settings/{key}` | Administrator | |
| `PUT` | `/api/settings` | Administrator | Whole-batch; needs mutating surface enabled |
| `GET` | `/api/identities` | Administrator | |
| `POST` | `/api/identities` | Administrator | |
| `GET` | `/api/identities/{id}/tokens` | Administrator | |
| `POST` | `/api/identities/{id}/tokens` | Administrator | Returns plaintext **once** |
| `DELETE` | `/api/identities/{id}/tokens/{tid}` | Administrator | |
| `POST` | `/api/identities/{id}/enable` | Administrator | |
| `POST` | `/api/identities/{id}/disable` | Administrator | |

### 5.3 Error shape

```json
{ "error": "unauthenticated" }
```

| Status | Meaning for the UI |
|---|---|
| `400` | Bad input. Body names which: `invalid_camera_id`, `invalid_recording_range`, `unknown_camera_in_scope`, … |
| `401` | Not authenticated. **Constant body** — the server deliberately does not say whether the token was expired, revoked, or wrong. Show "sign in again", nothing more specific. |
| `403` | Authenticated, wrong role. Safe to explain. |
| `404` | Not found **or out of scope** — indistinguishable by design. Never guess which. |
| `405` | Wrong method. |
| `409` | Conflict: `recording_range_not_contiguous` (a gap in footage), `recording_media_unavailable`. |
| `503` | Dependency down: `identity_store_unavailable`, `recording_index_unavailable`, `settings_unavailable`, `shutting_down`. **Recording is almost certainly still fine** — say so. |

The `503` messaging matters. Every one of these means "part of the API is
degraded"; none means "footage is being lost". A UI that shows a generic red
outage banner will send an operator to the wrong runbook.

---

## 6. States every screen needs

Not decoration — each is a real state this system produces.

- **Loading** — first paint.
- **Empty** — no cameras (fresh install), no recordings in range (a *gap*, worth
  distinguishing from "no search yet").
- **Degraded but recording** — `503` from an index or settings endpoint while
  cameras keep recording. The most common non-nominal state and the one most
  likely to be designed as a catastrophe.
- **Shutting down** — mutations return `503 shutting_down`, reads keep working.
- **Unauthorized** — `403`, explainable.
- **Unauthenticated** — `401`, not explainable beyond "sign in".
- **Not in scope** — indistinguishable from missing; render as missing.

---

## 7. Gaps — build these before the UI can be complete

Honest list. Each needs server work, not design work. Three are now closed; the
rest still block the screens that depend on them, and the shipped dashboard
(§10) simply does not offer those screens rather than faking them.

1. **No audit-log endpoint.** `access_audit` is populated and indexed; nothing
   exposes it. Needed for §4.6. Probably `GET /api/audit` with filters on
   actor, camera, category, and time. **Still open** — the dashboard has no
   audit screen.
2. ~~**No storage/health JSON endpoint.**~~ **Closed** by `GET /api/system`
   (§9). Storage pressure and free bytes remain Prometheus-only; the capability
   surface that the UI actually needed to branch on is now JSON.
3. ~~**No browser auth path.**~~ **Closed** by ADR 009 Decision 6: the reverse
   proxy authenticates and asserts the principal. §2.
4. ~~**No WebSocket or SSE.**~~ **Decided, not built**: poll-only at a 5s fleet
   interval, which is what the dashboard does. A push channel remains
   unnecessary at 10–20 cameras and would be a new failure domain.
5. ~~**No live-view integration path documented.**~~ **Closed by ADR 010.**
   `livekit-client` 2.21.0 is vendored at `apps/server/ui/vendor/`, served from
   `/ui/livekit-client.js`, and loaded on the first "Watch live" press. The
   camera detail screen connects to the room and attaches the subscribed video
   track. The token is used to connect and is never rendered. Live viewing
   remains development-only, as the endpoint always was.
6. **No camera thumbnails or preview stills.** Nothing generates them.
7. **No `ExportRecording` download endpoint** distinct from streaming playback.
   If the product wants "download this clip as a file", that is a new endpoint.

---

## 8. What the design must not do

Collected, because each is a way to make the product claim something untrue:

- Present fixture detections as real.
- Show a person count, dwell time, or heatmap. Heatmap semantics are explicitly
  undecided (`ROADMAP.md` Phase 0).
- Imply frame-accurate playback trimming.
- Show a capacity or camera-count headroom figure. No measurement supports one.
- Distinguish "not found" from "not permitted".
- Explain *why* a `401` happened.
- Render a `503` from a dependency as "recording has stopped".
- Store a bearer token anywhere persistent.
- Show a secret setting's value, or a masked input implying it holds one.

---

## 9. `GET /api/system` — ask the server what it can actually do

The three rules below were prose in this document, which meant an implementer
had to read and remember them. They are now values the API returns, so a client
can enforce them and a reviewer can check them. Authenticated like every other
read; requires `ViewSystemCapabilities` (Viewer and above).

```json
{
  "detection": {
    "model_name": "deterministic-fake",
    "model_version": "...",
    "is_production_model": false,
    "results_are_fixture": true
  },
  "playback": {
    "accuracy": "segment-aligned",
    "seekable": false,
    "transport": "chunked-stream"
  },
  "surfaces": {
    "recording_index": true, "viewer_tokens": false, "settings": true,
    "identities": true, "access_audit": true
  }
}
```

`detection` is not a constant. It is read from the inference implementation that
is actually compiled in, so it cannot drift from reality and cannot be edited to
claim something the binary does not do.

### Rule 1 — detection is a fixture, so no screen shows a count

**If `results_are_fixture` is true, no detection-derived number reaches a
screen.** Not a person count, not "3 people detected", not an event badge, not a
sparkline. The fixture's output is correctly shaped, correctly timestamped and
per-camera — it looks exactly like real data, which is precisely why this is the
easiest dishonest thing to ship by accident.

What may be shown: that a detection pipeline exists and is running, labelled as
a fixture. Build the surface behind this flag so that a real model turns it on
by itself.

### Rule 2 — 404 and "not permitted" are the same answer

The server returns a byte-identical `404 not_found` for a camera that does not
exist and a camera the caller may not see. This is deliberate: any observable
difference lets a viewer scoped to one camera enumerate the rest of the
installation — a floor plan of a building under surveillance, handed to someone
entitled to one lens.

So the UI **must never** say "you do not have access to camera-07". It does not
know camera-07 exists. The correct copy is "no such camera", and the correct
navigation model is that a scoped user's world contains only the cameras
`GET /api/cameras` returned. Never construct a camera URL from anything else —
not a bookmark, not a hand-typed ID, not an ID recovered from an old response.

### Rule 3 — playback is a stream, not a file

The response carries `Accept-Ranges: none`, `Transfer-Encoding: chunked`, and
`X-Playback-Accuracy: segment-aligned`. All three are true and none is
negotiable by the client.

- **No scrubber**, because there is nothing to seek to. A range request is
  refused, and a `<video>` element told the source accepts no ranges will not
  offer one.
- **No total-duration display** during playback. There is no `Content-Length`;
  the length is not known until the last segment has been read.
- **Boundaries are approximate, and say so.** The requested instants come back
  in `X-Requested-Start` / `X-Requested-End`; the media begins at the enclosing
  segment's keyframe. Show the requested window and label it "segment-aligned",
  rather than drawing a timeline that implies the cut landed where asked.
- **A time picker selects a window, not a frame.** Minute granularity matches
  what the index can honour; second granularity promises precision the server
  does not have.

---

## 10. What was built

`apps/server/ui/dashboard.html`, one self-contained file compiled into the
binary with `include_str!` and served at `GET /` and `GET /ui`.

One file, no build step, and no external request of any kind. The deployment
target is a self-hosted unit that may sit on a segmented network, so a CDN
script or webfont would fail closed in exactly the environment this product is
for. There is also no npm toolchain in this repository, and introducing one to
render five screens would be a build-and-supply-chain dependency bought for very
little.

### Why the shell is unauthenticated

It joins `/live`, `/ready`, and `/metrics` as a route reached without an
`Authorized`, on the same ADR 009 Decision 2 grounds: it is a compile-time
constant, byte-identical for every caller, and names no camera, no principal,
and no time range. It carries no data — every value it displays arrives later
through an authenticated call.

It also *has* to be reachable by a caller this system will refuse. The proxy
authenticates people this server may have no principal for, or may have
suspended; if the shell required authorization, that person would meet a raw
JSON `401` instead of the screen explaining that signing in again cannot help.

### Screens

| Route | Screen | Notes |
|---|---|---|
| `#/` | Fleet | §4.1 rollup, 5s poll, keeps the last good list behind a staleness marker |
| `#/camera/{id}` | Camera detail | §4.2 subsystem order; Start/Stop/Remove |
| `#/recordings/{id}` | Search + playback | §4.3, coverage strip, no scrubber |
| `#/settings` | Settings | §4.4, read-only |
| `#/system` | Capabilities | §9 |

### Not built, deliberately

- **Audit log** — gap 1: no endpoint exists.
- **Live video** — now built; see ADR 010. "Watch live" on the camera detail
  screen loads the vendored SDK from this server, mints a viewer token through
  the real endpoint, joins the room, and attaches the subscribed track. It is a
  real player, not a placeholder: if no publisher is in the room it says
  "Connected — waiting for video" rather than showing a black rectangle. Viewer
  access is still development-only and the page still says so.
- **Settings editing** — `PUT /api/settings` applies a whole batch
  transactionally or rejects it naming one offending key. A per-field editor
  would misrepresent that; a batch editor is worth building on its own.
- **Identity management** — the token plaintext is returned exactly once, and
  the once-only handling deserves its own design pass.
- **Any detection number** — Rule 1, enforced in code rather than remembered.

### What the tests do and do not cover

Ten tests in `apps/server/src/health_http.rs` assert properties of the shipped
bytes: served at `/` and `/ui` as HTML, no external resource, no credential
handling (`localStorage`, `sessionStorage`, `document.cookie`, `Authorization`,
`<form`, password inputs), no `controls` attribute on the player, no `autoplay`,
`missingCamera` free of permission vocabulary, no response cacheable by a
browser, every playback boundary carried from a recorded timestamp rather than a
derived one, a gapped range never offered as one stream, and detection counts
unreachable until the fixture flag has been consulted — including the
fail-closed `!state.system ||`, whose loss would show fixture numbers whenever
`/api/system` fails to load.

Each was mutation-verified: the mutation was applied, the named test failed, the
file was restored.

**These are assertions about an asset, not about an interface.** There is no
JavaScript engine in this workspace and no automated browser test, so nothing in
`make check` is evidence that a screen renders or that a click works. That
evidence comes from §11 instead, and it is a manual result, not a repeatable one.

## 11. What running it against a webcam changed

On 2026-08-15 the dashboard was driven in a browser against the full local loop:
FaceTime HD Camera → FFmpeg (H.264 baseline, 1280×720, 15 fps) → MediaMTX RTSP →
Retina ingestion → segmented fMP4 → PostgreSQL index → this page, through a
stand-in reverse proxy asserting an authenticated principal. Sixty segments
accumulated across four genuine coverage gaps (two server restarts and one
webcam death), which is what made the playback paths testable at all.

Three defects were found this way. None of them was reachable from the byte-level
tests, and all three are now covered by one:

1. **A dead player on a gapped range.** The server correctly answered `409
   recording_range_not_contiguous` when asked to join footage across a gap; the
   page rendered a silent `<video>` that simply never started — indistinguishable
   from footage that does not exist. Playback now splits the range into
   contiguous runs *before* a URL is built, and a gapped range is presented as a
   list of playable runs with a banner, never as one stream.
2. **A stale dashboard after upgrade.** The page is a compile-time constant, so
   a browser that cached it kept serving the previous interface across a server
   upgrade with nothing to indicate it. `response()` now sends
   `Cache-Control: no-store` on every response it builds — the JSON is
   per-principal and describes footage, and does not belong in a shared or
   on-disk cache either.
3. **Microsecond truncation in playback boundaries.** Segment times are recorded
   to microseconds (`10:38:43.011609Z`). A boundary rebuilt through `Date.parse`
   lost everything below the millisecond and asked for `…011Z`, putting the
   request 609 µs in front of the first frame — which the server correctly called
   a gap and refused. *Every* "play this run" answered 409. The recorded strings
   are now carried through untouched and only ever sent verbatim.

Verified afterwards in the browser, against that live index: six contiguous runs
listed for a four-gap range with no video element offered; the run beginning
`10:38:43.011609Z` requested at full precision, `readyState 4`, 1280×720,
`currentTime` advancing, no media error. The same range truncated to
milliseconds still returns 409 — the bug is reproducible, and the fix is what
distinguishes them.

Two properties were measured rather than assumed. `seekable` is a single
zero-width range and both a forward and a backward seek collapsed back to the
start, so "it cannot be seeked" is accurate and not merely the absence of a
scrubber. `duration` grows as the stream arrives and never describes the
requested range, which is why no duration is displayed.

What this does *not* establish: one browser (Chromium), one camera, one
operator, one afternoon. There is no automated browser test, no second browser,
no layout verification, and no measurement of behaviour under many cameras or
concurrent viewers.
