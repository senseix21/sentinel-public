# SENTINEL HTTP API Reference

This document describes the HTTP API implemented by `apps/server/src/health_http.rs`.
It documents current behavior, not the target architecture or unimplemented
features.

## Contents

- [Conventions](#conventions)
- [Authentication and authorization](#authentication-and-authorization)
- [Endpoint summary](#endpoint-summary)
- [Public endpoints](#public-endpoints)
- [System capabilities](#system-capabilities)
- [Cameras](#cameras)
- [Recordings and video retrieval](#recordings-and-video-retrieval)
- [Live viewer access](#live-viewer-access)
- [Settings](#settings)
- [Identities and API tokens](#identities-and-api-tokens)
- [Errors](#errors)
- [Current API limitations](#current-api-limitations)

## Conventions

### Base URLs

The local development stack exposes two HTTP entry points:

| URL | Intended use |
|---|---|
| `http://127.0.0.1:3001` | Browser access through the development login proxy |
| `http://127.0.0.1:3000` | Direct SENTINEL server access, including bearer-token API clients |

The examples use:

```bash
export SENTINEL_URL=http://127.0.0.1:3000
export SENTINEL_TOKEN='replace-with-issued-token'
export CAMERA_ID=mac-webcam
```

Unless an endpoint says otherwise, requests and responses use JSON:

```http
Content-Type: application/json
```

Ordinary responses include `Cache-Control: no-store` and close the connection
after the response. Playback uses a separate chunked streaming response.

### Identifiers and time ranges

- Camera, recording, principal, and token identifiers are validated domain
  identifiers. Invalid values return `400`.
- Recording times use RFC 3339 timestamps, normally UTC with a `Z` suffix.
- Recording time ranges are half-open: `[start, end)`.
- A recording search range cannot exceed 24 hours.
- URL-encode query parameters in clients. `curl --get --data-urlencode` is the
  safest form.

### Standard error body

Most errors have this shape:

```json
{
  "error": "machine_readable_code"
}
```

Some settings validation errors also include a safe description:

```json
{
  "error": "group_invariant",
  "detail": "description naming the invalid setting or group"
}
```

Internal errors, credentials, filesystem locators, and detailed authentication
failures are not returned.

## Authentication and authorization

### Public endpoints

Only the following operational endpoints are intentionally unauthenticated:

- `GET /live`
- `GET /ready`
- `GET /metrics`

The dashboard shell and its static assets are also public, but contain no
camera, recording, principal, or configuration data. Dashboard data is loaded
later through authenticated API calls.

### Bearer tokens

Non-browser API clients should send an opaque bearer token:

```http
Authorization: Bearer <token>
```

Example:

```bash
curl -fsS \
  -H "Authorization: Bearer $SENTINEL_TOKEN" \
  "$SENTINEL_URL/api/cameras"
```

Tokens are stored as Argon2id hashes. A newly minted plaintext token is returned
once and cannot be recovered later.

### Browser/proxy identity

In the browser flow, an identity-aware reverse proxy authenticates the human.
The server accepts `X-Authenticated-Principal` only after the proxy proves its
origin using both:

- an allowed transport peer; and
- a constant-time shared proof in `X-Forwarded-Proof`.

Only after that proof succeeds may the server read
`X-Authenticated-Principal` or trust the rightmost address in
`X-Forwarded-For`. These headers are deployment integration details and must
not be supplied directly by ordinary clients.

### Roles

Roles are cumulative:

| Role | Permissions |
|---|---|
| `viewer` | List scoped cameras, read camera state, search recording metadata, retrieve playback, obtain a viewer token, read settings and system capabilities |
| `operator` | Viewer permissions plus add, start, stop, and remove cameras |
| `administrator` | Operator permissions plus read/manage identities and update settings |

Every camera-scoped action also checks the principal's camera scope. Access to
an out-of-scope camera returns `404`, not `403`, so callers cannot enumerate
cameras they are not allowed to see.

Configuration and identity mutation additionally require the deployment's
mutating API surface to be enabled. When it is disabled, the route returns
`404` as though it does not exist.

## Endpoint summary

| Method | Path | Minimum role | Purpose |
|---|---|---:|---|
| `GET` | `/live` | Public | Process liveness |
| `GET` | `/ready` | Public | Application dependency readiness |
| `GET` | `/metrics` | Public | Prometheus metrics |
| `GET` | `/api/system` | Viewer | Runtime capabilities and wired surfaces |
| `GET` | `/api/cameras` | Viewer | List visible cameras |
| `POST` | `/api/cameras` | Operator | Add a camera |
| `GET` | `/api/cameras/{camera_id}` | Viewer | Camera state and health |
| `GET` | `/api/cameras/{camera_id}/health` | Viewer | Alias for camera state and health |
| `POST` | `/api/cameras/{camera_id}/start` | Operator | Start the camera pipeline |
| `POST` | `/api/cameras/{camera_id}/stop` | Operator | Stop the camera pipeline |
| `DELETE` | `/api/cameras/{camera_id}` | Operator | Stop, join, and remove the camera |
| `GET` | `/api/cameras/{camera_id}/recordings` | Viewer | Search completed recordings |
| `GET` | `/api/cameras/{camera_id}/recordings/{recording_id}` | Viewer | Get completed recording metadata |
| `GET` | `/api/cameras/{camera_id}/recordings/playback` | Viewer | Stream MP4 footage for a time range |
| `GET` | `/api/cameras/{camera_id}/recordings/{recording_id}/video` | Viewer | Stream a full recording or a leading duration |
| `POST` | `/api/cameras/{camera_id}/viewer-token` | Viewer | Mint development LiveKit viewer access |
| `GET` | `/api/settings` | Viewer | List resolved global settings |
| `GET` | `/api/settings/{setting_key}` | Viewer | Read stored and effective setting state |
| `PUT` | `/api/settings` | Administrator | Transactionally update operational settings |
| `GET` | `/api/identities` | Administrator | List principals |
| `POST` | `/api/identities` | Administrator | Create or replace a principal |
| `GET` | `/api/identities/{principal_id}/tokens` | Administrator | List token metadata |
| `POST` | `/api/identities/{principal_id}/tokens` | Administrator | Mint a token |
| `DELETE` | `/api/identities/{principal_id}/tokens/{token_id}` | Administrator | Revoke a token |
| `POST` | `/api/identities/{principal_id}/disable` | Administrator | Disable a principal |
| `POST` | `/api/identities/{principal_id}/enable` | Administrator | Enable a principal |

## Public endpoints

### `GET /live`

Reports process liveness. It does not assert that cameras or dependencies are
healthy.

```bash
curl -i "$SENTINEL_URL/live"
```

Successful response:

```http
HTTP/1.1 200 OK
Content-Type: text/plain; charset=utf-8

live
```

### `GET /ready`

Reports dependency readiness and camera degradation. Use this for operational
readiness, not `/live`. The exact body reflects the active camera registry,
recording repository, metadata writer, and Phase 4 state.

```bash
curl -fsS "$SENTINEL_URL/ready"
```

### `GET /metrics`

Returns Prometheus text exposition with bounded-cardinality metrics for camera
connections, queues, recording, storage, playback, detection, LiveKit,
configuration, authentication, and API behavior.

```bash
curl -fsS "$SENTINEL_URL/metrics"
```

Content type:

```text
text/plain; version=0.0.4; charset=utf-8
```

## System capabilities

### `GET /api/system`

Returns machine-readable capability honesty. Clients should inspect this before
offering production detection results or seekable playback controls.

```bash
curl -fsS \
  -H "Authorization: Bearer $SENTINEL_TOKEN" \
  "$SENTINEL_URL/api/system"
```

Example response:

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
    "recording_index": true,
    "viewer_tokens": true,
    "settings": true,
    "identities": true,
    "access_audit": true
  }
}
```

Surface booleans report collaborators that were successfully wired into the
running process, not merely configuration intent.

## Cameras

### Camera response

Camera list, detail, and health resources use a flat credential-free response:

```json
{
  "camera_id": "mac-webcam",
  "pipeline_state": "running",
  "connection_state": "connected",
  "connection_attempts": 1,
  "connection_epochs": 1,
  "reconnects": 0,
  "terminal_failures": 0,
  "last_connected_unix_ms": 1787731200000,
  "last_media_unix_ms": 1787731201000,
  "last_actionable_failure": null,
  "recorder_queue_depth": 0,
  "recorder_queue_capacity": 256,
  "recorder_rejected": 0,
  "live_queue_depth": 0,
  "live_queue_capacity": 64,
  "live_offered": 1200,
  "live_dropped": 0,
  "detection_enabled": false,
  "detection_state": "Disabled",
  "detection_queue_depth": 0,
  "detection_queue_capacity": 2,
  "detection_dropped": 0,
  "detection_inferred": 0,
  "detections_published": 0,
  "recorder_completed_segments": 12,
  "recorder_failed_segments": 0,
  "live_publication_enabled": true,
  "live_publication_state": "Published",
  "live_publication_failure": null,
  "live_publication_timestamp_authority": "adapter-clock",
  "live_published_access_units": 1200,
  "live_published_bytes": 4567890,
  "live_publication_adapter_drops": 0
}
```

Counter values are examples; clients must not assume the sample values or queue
capacities.

### `GET /api/cameras`

Lists only cameras visible to the authenticated principal.

```bash
curl -fsS \
  -H "Authorization: Bearer $SENTINEL_TOKEN" \
  "$SENTINEL_URL/api/cameras"
```

Response: a JSON array of camera responses.

### `POST /api/cameras`

Adds a validated H.264 MVP camera configuration to the supervisor.

```bash
curl -fsS -X POST \
  -H "Authorization: Bearer $SENTINEL_TOKEN" \
  -H 'Content-Type: application/json' \
  "$SENTINEL_URL/api/cameras" \
  --data '{
    "camera_id": "loading-bay",
    "rtsp_endpoint": "rtsp://10.0.0.50:554/stream1",
    "username": "camera-user",
    "password": "replace-me",
    "width": 1920,
    "height": 1080,
    "detection_enabled": false,
    "live_publication_enabled": true
  }'
```

Fields:

| Field | Type | Required | Behavior |
|---|---|---:|---|
| `camera_id` | string | Yes | Validated unique camera identifier |
| `rtsp_endpoint` | string | Yes | Validated RTSP endpoint |
| `username` | string or null | No | Must be supplied together with `password` |
| `password` | string or null | No | Must be supplied together with `username` |
| `width` | unsigned integer | Yes | Expected video width |
| `height` | unsigned integer | Yes | Expected video height |
| `detection_enabled` | boolean | No | Defaults to `true` |
| `live_publication_enabled` | boolean or null | No | Defaults to whether this deployment has LiveKit configured |

Success:

```http
HTTP/1.1 201 Created

{"status":"ok"}
```

Notable failures include `incomplete_camera_credentials`,
`invalid_camera_configuration`, `live_publication_not_configured`,
`camera_already_exists`, and `camera_capacity_reached`.

The API request contains camera credentials, so use only an encrypted/trusted
transport in production. Credentials are not returned by camera resources.

### `GET /api/cameras/{camera_id}`

### `GET /api/cameras/{camera_id}/health`

Both routes return the same camera response described above.

```bash
curl -fsS \
  -H "Authorization: Bearer $SENTINEL_TOKEN" \
  "$SENTINEL_URL/api/cameras/$CAMERA_ID/health"
```

### `POST /api/cameras/{camera_id}/start`

Starts the camera pipeline. Success returns:

```json
{"status":"ok"}
```

### `POST /api/cameras/{camera_id}/stop`

Stops the camera pipeline. Success returns:

```json
{"status":"ok"}
```

### `DELETE /api/cameras/{camera_id}`

Cancels, joins, and removes the camera from the supervisor.

```bash
curl -i -X DELETE \
  -H "Authorization: Bearer $SENTINEL_TOKEN" \
  "$SENTINEL_URL/api/cameras/$CAMERA_ID"
```

Success is `204 No Content`. This removes the supervised runtime camera; it is
not a recording-deletion endpoint.

## Recordings and video retrieval

Only completed, indexed recordings are returned. Storage paths are never
exposed. If indexed media is absent or no longer a regular file, the server
marks it inconsistent and refuses to describe or stream it as healthy footage.

### Recording object

```json
{
  "recording_id": "r-6a8148e8-a9a1907-2",
  "camera_id": "mac-webcam",
  "started_at": "2026-08-26T08:00:00Z",
  "ended_at": "2026-08-26T08:00:30Z",
  "codec": "h264",
  "width": 1280,
  "height": 720,
  "size_bytes": 5242880,
  "status": "completed",
  "playback": "/api/cameras/mac-webcam/recordings/playback?start=2026-08-26T08:00:00Z&end=2026-08-26T08:00:30Z"
}
```

The `playback` field is the supported media-retrieval link for that recording.
There is no endpoint that exposes its internal filesystem locator.

### Recording folder selection

`GET /api/recording-folders` (Administrator) lists supported folders beneath
approved browse roots. Each result has an opaque `id` and relative `label`;
absolute server paths are not returned. Arbitrary paths are never accepted.

`POST /api/recording-folders/index` (Administrator, mutating surface enabled)
accepts `{"id":"<folder-id>"}` and returns **202** with
`{"source_id":"archive-1"}`. Unknown fields are rejected. One owned worker
indexes a folder at a time; **409 `index_busy`** means retry after the current
job finishes. Only established camera/UTC segment layouts are supported.

`GET /api/recording-sources` (Viewer) lists registered sources, `state`, and
camera-scoped coverage (`camera_id`, `first_recorded_at`, `last_recorded_at`,
`recordings`). Empty sources outside a restricted camera scope are hidden.
States include `indexing`, `ready`, `partial`, `failed`, `interrupted` and
`unavailable`. Source indexes and roots persist; media files remain read-only.

Archive searches, details, range playback and single-recording video use the
same query contracts below, with this explicit prefix:

```text
/api/recording-sources/{source_id}/cameras/{camera_id}/recordings
/api/recording-sources/{source_id}/cameras/{camera_id}/recordings/{recording_id}
/api/recording-sources/{source_id}/cameras/{camera_id}/recordings/playback
/api/recording-sources/{source_id}/cameras/{camera_id}/recordings/{recording_id}/video
```

Camera-scoped authorization applies before source lookup. Video export audit
rows include `recording_source_id` so duplicate IDs in different folders remain
distinguishable. Existing `/api/cameras/...` URLs continue to use `primary`.
Archive sources are excluded from the recorder's retention and reconciliation
owners. See [ADR 020](adr/020-playback-recording-folders.md).

### `GET /api/cameras/{camera_id}/recordings`

Returns completed segments overlapping the requested range, ordered by start
time and recording identifier.

Query parameters:

| Parameter | Required | Description |
|---|---:|---|
| `start` | Yes | RFC 3339 range start |
| `end` | Yes | RFC 3339 range end; must be later than `start` |
| `limit` | No | Valid bounded result count; defaults to `100` |
| `cursor` | No | Opaque `next_cursor` from the preceding response |

```bash
curl -fsS -G \
  -H "Authorization: Bearer $SENTINEL_TOKEN" \
  "$SENTINEL_URL/api/cameras/$CAMERA_ID/recordings" \
  --data-urlencode 'start=2026-08-26T08:00:00Z' \
  --data-urlencode 'end=2026-08-26T09:00:00Z' \
  --data-urlencode 'limit=100'
```

Response:

```json
{
  "recordings": [
    {
      "recording_id": "r-6a8148e8-a9a1907-2",
      "camera_id": "mac-webcam",
      "started_at": "2026-08-26T08:00:00Z",
      "ended_at": "2026-08-26T08:00:30Z",
      "codec": "h264",
      "width": 1280,
      "height": 720,
      "size_bytes": 5242880,
      "status": "completed",
      "playback": "/api/cameras/mac-webcam/recordings/playback?start=2026-08-26T08:00:00Z&end=2026-08-26T08:00:30Z"
    }
  ],
  "next_cursor": null
}
```

If `next_cursor` is non-null, pass it unchanged in the next request with the
same camera and time range.

### `GET /api/cameras/{camera_id}/recordings/{recording_id}`

Returns one completed recording's metadata and playback link.

```bash
curl -fsS \
  -H "Authorization: Bearer $SENTINEL_TOKEN" \
  "$SENTINEL_URL/api/cameras/$CAMERA_ID/recordings/r-6a8148e8-a9a1907-2"
```

This is a metadata endpoint. It does not return media bytes.

### `GET /api/cameras/{camera_id}/recordings/playback`

Retrieves footage for an RFC 3339 time range. The server finds every completed
segment overlapping `[start, end)`, validates that the selected range is
contiguous under the playback plan, assembles MP4 output, and streams it.

```bash
curl -fS -G \
  -H "Authorization: Bearer $SENTINEL_TOKEN" \
  "$SENTINEL_URL/api/cameras/$CAMERA_ID/recordings/playback" \
  --data-urlencode 'start=2026-08-26T08:00:00Z' \
  --data-urlencode 'end=2026-08-26T08:01:00Z' \
  --output footage.mp4
```

Success headers:

```http
HTTP/1.1 200 OK
Content-Type: video/mp4
Transfer-Encoding: chunked
Accept-Ranges: none
X-Playback-Accuracy: segment-aligned
X-Requested-Start: 2026-08-26T08:00:00Z
X-Requested-End: 2026-08-26T08:01:00Z
Connection: close
```

Playback contracts:

- The response may begin before `start` and end after `end` because whole
  segments and their keyframes define the returned boundary.
- The synthesized response is not byte-range seekable.
- There is no `Content-Length`; the server does not know it until assembly
  completes.
- The current implementation is H.264 stream-copy oriented and does not promise
  frame-exact arbitrary boundaries.
- A single recording can be retrieved by following its exact `playback` link.
- The endpoint supports at most 1,000 selected completed segments per request.
- Playback is audited with principal, camera, requested range, outcome, and
  exported media byte count.

Notable failures:

| Status | Error | Meaning |
|---:|---|---|
| `400` | `invalid_recording_range` | Missing, malformed, reversed, or excessive range |
| `404` | `not_found` | Camera is outside scope or route is absent |
| `409` | `recording_range_not_contiguous` | Indexed segments cannot form the requested plan |
| `409` | `recording_media_unavailable` | Assembly could not access or prepare the media |
| `503` | `recording_index_unavailable` | PostgreSQL recording index is unavailable |

## Live viewer access

### `POST /api/cameras/{camera_id}/viewer-token`

Returns short-lived, subscribe-only LiveKit access for the camera's room.

```bash
curl -fsS -X POST \
  -H "Authorization: Bearer $SENTINEL_TOKEN" \
  "$SENTINEL_URL/api/cameras/$CAMERA_ID/viewer-token"
```

Response:

```json
{
  "endpoint": "ws://127.0.0.1:7880",
  "room": "camera-mac-webcam",
  "token": "eyJ...",
  "expires_in_seconds": 300,
  "development_only": true
}
```

The returned token is a LiveKit credential, not a SENTINEL API bearer token.
The current issuer is development-only. Live media remains WebRTC through
LiveKit; the HTTP playback API serves recorded media.

## Settings

Settings reads require a viewer. Updates require an administrator and an
enabled mutating surface. Values in update requests are strings so duration,
count, path, and enum settings share one wire format.

### `GET /api/settings`

Returns all resolved unscoped settings and their origins:

```json
{
  "generation": 12,
  "freshness": "fresh",
  "cache": "current",
  "settings": [
    {
      "key": "camera.recording.segment_duration",
      "value": "30s",
      "origin": "default",
      "boundary": "next_segment",
      "class": "operational",
      "secret": false
    }
  ]
}
```

Origins reflect the precedence `defaults < file < persisted < environment`.
Secret values render as `***`.

### `GET /api/settings/{setting_key}`

Returns stored and effective values separately:

```json
{
  "key": "camera.recording.segment_duration",
  "effective": "30s",
  "origin": "default",
  "stored": null,
  "boundary": "next_segment",
  "editable": true,
  "secret": false
}
```

### `PUT /api/settings`

Applies a multi-setting update transactionally. A failed change rejects the
whole batch.

```bash
curl -fsS -X PUT \
  -H "Authorization: Bearer $SENTINEL_TOKEN" \
  -H 'Content-Type: application/json' \
  "$SENTINEL_URL/api/settings" \
  --data '{
    "changes": {
      "camera.recording.segment_duration": "45s"
    }
  }'
```

Camera-scoped setting keys use `setting.name@camera-id`:

```json
{
  "changes": {
    "camera.livekit.viewer_idle_grace@loading-bay": "5s"
  }
}
```

Success:

```json
{
  "generation": 13,
  "outcomes": [
    {
      "key": "camera.recording.segment_duration",
      "camera_id": null,
      "outcome": "pending_next_segment"
    }
  ]
}
```

The exact outcome describes whether the value is effective immediately,
pending a safe boundary or restart, or persisted but shadowed by a higher
precedence source.

## Identities and API tokens

Identity reads and mutations require an administrator. Mutations also require
the deployment's mutating surface to be enabled.

### `GET /api/identities`

Returns principals without secret material:

```json
[
  {
    "principal_id": "operations",
    "kind": "human",
    "display_name": "Operations Team",
    "role": "operator",
    "scope": ["loading-bay", "entrance"],
    "enabled": true,
    "live_tokens": 2
  }
]
```

`scope` is either the string `"all"` or an array of camera identifiers.

### `POST /api/identities`

Creates or replaces a principal:

```bash
curl -fsS -X POST \
  -H "Authorization: Bearer $SENTINEL_TOKEN" \
  -H 'Content-Type: application/json' \
  "$SENTINEL_URL/api/identities" \
  --data '{
    "principal_id": "playback-service",
    "kind": "service",
    "display_name": "Playback integration",
    "role": "viewer",
    "cameras": ["loading-bay"],
    "enabled": true
  }'
```

Fields:

| Field | Required | Default |
|---|---:|---|
| `principal_id` | Yes | — |
| `role` | Yes | —; one of `viewer`, `operator`, `administrator` |
| `kind` | No | `human`; may be `human` or `service` |
| `display_name` | No | `principal_id` |
| `cameras` | No | All cameras; an explicit empty array grants no camera scope |
| `enabled` | No | `true` |

Success:

```http
HTTP/1.1 201 Created

{"principal_id":"playback-service"}
```

### `GET /api/identities/{principal_id}/tokens`

Lists token metadata only:

```json
[
  {
    "token_id": "token-id",
    "label": "integration",
    "expires_at_unix": 1795507200,
    "revoked_at_unix": null
  }
]
```

### `POST /api/identities/{principal_id}/tokens`

Mints an opaque bearer token:

```bash
curl -fsS -X POST \
  -H "Authorization: Bearer $SENTINEL_TOKEN" \
  -H 'Content-Type: application/json' \
  "$SENTINEL_URL/api/identities/playback-service/tokens" \
  --data '{"label":"video-exporter","lifetime_days":90}'
```

`lifetime_days` defaults to `90` and must be between 1 and 3,650.

Response:

```json
{
  "token_id": "token-id",
  "token": "plaintext-token-returned-once",
  "expires_at_unix": 1795507200
}
```

Store the `token` immediately. The server stores only its Argon2id hash and
cannot return it again.

### `DELETE /api/identities/{principal_id}/tokens/{token_id}`

Revokes a token. Success is `204 No Content`. Missing and already-revoked token
identifiers both return `404 token_not_found`.

### `POST /api/identities/{principal_id}/disable`

Disables a principal and returns:

```json
{"principal_id":"playback-service","enabled":false}
```

### `POST /api/identities/{principal_id}/enable`

Enables a principal and returns:

```json
{"principal_id":"playback-service","enabled":true}
```

## Errors

Common cross-cutting responses:

| Status | Error | Meaning |
|---:|---|---|
| `400` | `invalid_request` | Malformed HTTP request or invalid UTF-8 body/head |
| `400` | endpoint-specific validation code | Identifier, body, configuration, or range is invalid |
| `401` | `unauthenticated` | Missing, malformed, unknown, expired, revoked, or otherwise invalid authentication |
| `403` | `unauthorized` | Authenticated principal lacks the required role |
| `404` | `not_found` | Resource absent, camera outside scope, or protected surface disabled |
| `405` | `method_not_allowed` | Path exists in a route family but method is unsupported |
| `409` | endpoint-specific conflict | Existing camera, invalid lifecycle transition, inconsistent media, or violated setting group |
| `503` | `shutting_down` | A mutating request was refused while draining |
| `503` | dependency-specific code | Required repository, supervisor, token issuer, or other dependency is unavailable |

Authentication failures deliberately share the same external `401` body. The
specific failure category is retained in audit/metrics rather than disclosed.

The HTTP adapter reads a bounded request body. The default maximum complete
request size is 16 KiB and is configurable through `api.max_request_bytes`.
Slow request/response I/O is bounded by `api.io_timeout`; playback uses its own
larger stall timeout so a normally paced media consumer can continue streaming.

## Current API limitations

- The server uses a purpose-built HTTP adapter in `apps/server`; the empty
  `crates/api` stub is not the deployed API implementation.
- There is no OpenAPI document or generated client yet.
- There is no direct raw storage-path or raw-segment download endpoint.
  Recording metadata supplies a supported time-range playback link instead.
- Playback is segment-aligned, chunked, and non-seekable. It is not frame-exact
  and does not support HTTP byte-range requests.
- Playback accepts a time range, not a `recording_id`; use the `playback` URL in
  the recording object to retrieve exactly one indexed segment's interval.
- There is no recording deletion API.
- Detection events do not currently have a public production API, and the only
  compiled inference backend is deterministic fake inference.
- ROI, rules, alerts, analytics, heatmaps, and worker APIs do not exist.
- The viewer-token endpoint is explicitly development-only.
- The development login proxy is not a production identity provider and has no
  TLS, MFA, account recovery, or durable session store.
- API serialization and gRPC are not treated as scale bottlenecks without Phase
  9 measurements. HTTP/JSON remains the external control API, HTTP streams
  recorded playback, and LiveKit/WebRTC carries live video.

## Source of truth

The implementation is in
[`apps/server/src/health_http.rs`](../apps/server/src/health_http.rs), with the
authorization contract in
[`apps/server/src/authorize.rs`](../apps/server/src/authorize.rs). Security and
trust-boundary decisions are recorded in
[`docs/adr/009-identity-trust-boundary-and-secret-store.md`](adr/009-identity-trust-boundary-and-secret-store.md).

When this document and code disagree, the code and accepted ADRs are
authoritative; update this document in the same change that modifies a route.
