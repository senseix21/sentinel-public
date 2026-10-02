# Recording Video API and UI Plan

Status: slices 0-4 implemented; slice 5 (real media and measurements) and
slice 6 (validation and handoff) not started. Checkboxes below are ticked from
evidence, with `[~]` where only part of the item holds and the remainder named.  
Date: 2026-09-05  
Scope: recording-specific full-video retrieval with optional leading duration  
Branch: `feature/ffmpeg-webcam-source`

## 1. Objective

Add a recording-specific media endpoint and expose it in the Playback UI:

```http
GET /api/cameras/{camera_id}/recordings/{recording_id}/video
GET /api/cameras/{camera_id}/recordings/{recording_id}/video?duration_seconds=5
```

Contract:

- omitted `duration_seconds` returns the complete completed recording;
- present `duration_seconds=N` returns the first `N` seconds of that recording;
- the request is camera-scoped, authenticated, authorized, and export-audited;
- no filesystem path is exposed;
- active, failed, missing, inconsistent, or out-of-scope recordings are never
  returned;
- the existing time-range playback endpoint remains compatible.

This is a media export operation, not a metadata read and not a domain event.

## 2. Current implementation

The repository currently provides:

| Route | Current behavior |
|---|---|
| `GET /api/cameras/{camera}/recordings?start=&end=` | Searches completed indexed segments |
| `GET /api/cameras/{camera}/recordings/{recording}` | Returns metadata and a time-range playback link |
| `GET /api/cameras/{camera}/recordings/playback?start=&end=` | Streams all segments covering a mandatory UTC range |

Current time-range playback:

- requires both `start` and `end`;
- queries PostgreSQL by overlap;
- builds a `PlaybackPlan` of up to 1,000 contiguous segments;
- starts one owned FFmpeg stream-copy process;
- emits chunked fragmented MP4;
- is segment-aligned, not frame-accurate or seekable;
- has bounded request, process startup, output buffer, and client-stall behavior;
- reaches `Action::ExportRecording` and writes an export audit fact.

The UI searches a UTC interval, merges adjacent segments into runs, and plays a
run through the time-range endpoint. It has no recording-specific full/short
video control.

## 3. Decisions and assumptions

### 3.1 New route, not a breaking rewrite

Add `/recordings/{recording_id}/video`. Do not reinterpret
`/recordings/playback`, and do not make its existing `end` optional. The two
resources have different identities:

- time-range playback answers “show footage from this camera and UTC range”;
- recording video answers “return this indexed recording, optionally shortened
  from its beginning.”

### 3.2 Duration input

Use one optional query field:

```text
duration_seconds=<positive integer>
```

Initial validation:

- decimal integer only;
- minimum `1` second;
- maximum `60` seconds, matching the currently validated maximum segment size;
- at most one occurrence;
- unknown query fields are rejected;
- a duration greater than the indexed recording duration is rejected rather
  than silently clamped, because a successful response must not promise more
  media than exists.

If product requirements later allow recordings longer than 60 seconds, replace
the literal ceiling with a versioned configuration value and a checked maximum
derived from indexed duration. Do not accept floating-point seconds in the first
slice.

### 3.3 No offset in the first slice

`offset_seconds` is deliberately excluded. The requested behavior is “the first
5/10/x seconds” or “the whole recording.” Arbitrary offsets reintroduce the
keyframe/start-accuracy decision and should be a separately accepted extension.

### 3.4 Accuracy policy

The recommended first implementation remains H.264 stream-copy. FFmpeg starts
at the recording's first sample and applies the requested output duration. It
does not decode or re-encode.

Consequences:

- the clip starts at the recording start;
- output is duration-bounded in media time but not claimed frame-exact;
- the final packet/frame boundary may differ slightly from the requested whole
  second;
- the response continues to declare `Accept-Ranges: none`;
- no exact-duration claim is allowed until a fixture measures actual output.

If the product owner requires exactly 5.000 seconds or an arbitrary precise
offset, create an ADR for a transcoding mode, its CPU/GPU budget, concurrency
limit, cancellation behavior, and priority below recording.

### 3.5 Full-video behavior

When duration is absent, stream the entire indexed recording through the same
safe remux boundary. Do not open a user-supplied path or return the storage
locator. Keeping one response pipeline preserves authorization, audit,
cancellation, content type, and error behavior.

An optimized direct-file response with `Content-Length` or byte ranges is a
future option only after its path validation, retention race, audit byte count,
and browser behavior are designed explicitly.

## 4. Proposed HTTP contract

### 4.1 Full recording

```http
GET /api/cameras/camera-01/recordings/r-123/video HTTP/1.1
```

Successful response:

```http
HTTP/1.1 200 OK
Content-Type: video/mp4
Transfer-Encoding: chunked
Accept-Ranges: none
X-Playback-Accuracy: recording-start-duration-bounded
X-Recording-Id: r-123
X-Requested-Duration-Seconds: full
Connection: close
```

### 4.2 Leading clip

```http
GET /api/cameras/camera-01/recordings/r-123/video?duration_seconds=5 HTTP/1.1
```

Additional success header:

```http
X-Requested-Duration-Seconds: 5
```

### 4.3 Errors

| Status | Stable code | Meaning |
|---:|---|---|
| `400` | `invalid_recording_duration` | zero, negative, non-integer, duplicate, unknown, or over-limit query |
| `400` | `recording_duration_exceeds_media` | requested duration exceeds indexed recording duration |
| `401` | existing authentication code | caller has no accepted identity |
| `403` | existing authorization code | role cannot export footage |
| `404` | `not_found` | camera is out of scope or route shape is invalid |
| `404` | `recording_not_found` | no completed recording with that camera and ID |
| `409` | `recording_storage_inconsistent` | index points to missing/unsafe storage |
| `409` | `recording_media_unavailable` | FFmpeg cannot open/remux the validated recording |
| `503` | `recording_index_unavailable` | PostgreSQL cannot answer |

Do not distinguish an out-of-scope camera from an unknown camera. Preserve the
existing authorization anti-enumeration behavior.

## 5. Typed contracts

Add narrow types at the playback/API boundary:

```rust
enum RecordingVideoExtent {
    Full,
    Leading(ClipDuration),
}

struct ClipDuration(NonZeroU32);

struct RecordingVideoRequest {
    camera_id: CameraId,
    recording_id: RecordingId,
    extent: RecordingVideoExtent,
}
```

`ClipDuration::new` enforces the supported bound. Query parsing creates a fully
validated `RecordingVideoRequest` before repository or process work begins.

Extend the assembler with a recording-specific entry point rather than adding
booleans to `PlaybackAssembler::start`:

```rust
async fn start_recording(
    &self,
    recording: &CompletedRecording,
    extent: RecordingVideoExtent,
) -> Result<PlaybackStream, PlaybackError>;
```

Both range and recording paths should share private helpers for:

- canonical-root and symlink-safe path validation;
- owned FFmpeg process construction;
- bounded stdin/stdout ownership;
- startup timeout, cancellation, finish, and child reaping.

No broad playback trait is needed until a second real implementation exists.

## 6. Data and ownership flow

```text
Browser request
    |
    v
HTTP parser -> authentication -> camera-scoped export authorization
    |
    v
PostgreSQL find_completed(camera_id, recording_id)
    |
    v
Validated storage locator under recording root
    |
    v
Owned FFmpeg stream-copy process
    |
    v
64 KiB bounded HTTP streaming buffer -> browser video element/download
    |
    v
Export audit: principal, camera, requested extent, outcome, bytes delivered
```

The request owner owns the FFmpeg child. Client disconnect, timeout, server
shutdown, invalid media, or completed output must all join/reap it. The recorder
and camera pipeline are not participants and cannot be cancelled or
backpressured by clip retrieval.

## 7. Implementation slices

### Slice 0 — ADR and executable acceptance contract

- [~] Add an ADR amending ADR 004's reversible playback policy.
  ADR 014 exists and is accepted, but it does not state that it amends ADR 004's reversible playback policy; the amendment is unrecorded.
- [x] Record why a recording subresource is separate from time-range playback.
- [x] Accept the provisional stream-copy accuracy statement and reversal rule.
- [x] Add failing HTTP route/query tests and JS URL tests first.
  Route, query-parsing and JS URL coverage all exist. That they were written first is not evidenced after the fact and is not claimed.

Exit gate:

- route, duration semantics, accuracy, status codes, authorization, and backward
  compatibility are written before implementation.

### Slice 1 — Domain parsing and assembler seam

- [x] Add `ClipDuration`, `RecordingVideoExtent`, and request parsing.
- [x] Reject missing/invalid identifiers before repository access.
- [~] Refactor safe-path/child construction helpers out of range-only assembly.
  `validated_recording_path` serves the recording route; the range route still canonicalises each segment inline inside `start`, so the helper is added rather than shared.
- [x] Add `start_recording` with no-duration and duration argument construction.
- [x] Ensure FFmpeg arguments are constructed without a shell.
- [~] Keep stderr handling bounded and ensure child cleanup on every error.
  Child cleanup is covered: `kill_on_drop` plus explicit kill on every failure path. Stderr is sent to `Stdio::null()`, so it is bounded only by being discarded and an assembly failure carries no cause -- the same gap that was fixed on the camera source.

Focused tests:

- full extent emits no duration argument;
- 1, 5, 10, and 60 seconds emit the expected duration argument;
- 0, 61, negative, fractional, duplicate, and unknown fields fail;
- unsafe locator, symlink, missing file, missing pipe, spawn failure, timeout,
  nonzero exit, cancellation, and normal finish are covered.

Exit gate:

- one completed recording can be remuxed fully or duration-bounded without
  decoding, unbounded buffering, path exposure, or orphan processes.

### Slice 2 — Authenticated HTTP vertical slice

- [x] Recognize exactly
  `/api/cameras/{camera}/recordings/{recording}/video`.
- [x] Route through the same context establishment and
  `Action::ExportRecording` chokepoint as time-range playback.
- [x] Resolve recording metadata by both camera ID and recording ID.
- [x] Confirm the durable file is a regular non-symlink beneath the configured
  root immediately before process start.
- [x] Stream fragmented MP4 with explicit honesty headers.
- [x] Count delivered media bytes and write success, refusal, cancellation, and
  failure export-audit outcomes.
  A mid-stream failure previously discarded the byte count with the error, so no export row was written for footage that had already left; the delivered extent now travels inside the failure.
- [x] Reuse configured playback stall/startup bounds and bounded-cardinality
  metrics.
  The startup bound now gates readiness: both routes read the first chunk before returning, which is also what makes `409 recording_media_unavailable` reachable.

HTTP tests:

- anonymous, malformed token, insufficient role, out-of-scope camera;
- unknown camera/recording and cross-camera recording ID;
- missing database, retryable database failure, missing/corrupt file;
- full response and 5/10-second response;
- slow/disconnected client cancels and joins FFmpeg;
- server shutdown does not leave the child alive;
- existing metadata/search/time-range routes remain byte-for-byte compatible.

Exit gate:

- the endpoint is usable end to end and cannot bypass authorization or export
  auditing.

### Slice 3 — Response links and documentation

- [x] Add a stable `video` link to each `RecordingResponse`.
- [x] Keep the existing `playback` link during migration.
- [ ] Document full and duration-bounded curl examples in `docs/API.md`.
  `docs/API.md` carries only the route-table row. There are no curl examples for this route.
- [~] Document accuracy, maximum duration, response headers, and errors.
  The shared playback error table covers `recording_media_unavailable`, and accuracy is stated for the range route. `duration_seconds`, its 1..60 bound, and this route's own headers are not documented.
- [ ] Update client system and architecture guides without claiming exact trim.
  Neither `docs/CLIENT_SYSTEM_GUIDE.md` nor `docs/architecture.md` mentions this route.

Recommended response fragment:

```json
{
  "recording_id": "r-123",
  "video": "/api/cameras/camera-01/recordings/r-123/video",
  "playback": "/api/cameras/camera-01/recordings/playback?start=...&end=..."
}
```

Exit gate:

- clients can discover rather than construct the canonical recording video URL.

### Slice 4 — Playback UI

- [x] Add `recordingVideoPath(cameraId, recordingId, durationSeconds)` to
  `ui/js/playback.js` using `URLSearchParams`/`encodeURIComponent`.
- [x] Add a segment-row action for “Play full recording.”
- [x] Add a duration selector with `Full`, `5 seconds`, `10 seconds`, and a
  validated custom integer input.
- [x] Label the behavior “from recording start.”
- [~] Disable Play while the custom value is invalid or a request is starting.
  An invalid custom duration is reported in the error banner after the click rather than preventing it; the button is never disabled.
- [x] Reuse the existing video element lifecycle: load, pause/resume, stop, and
  disconnect cancellation.
- [~] Present stable server error codes in plain operator language.
  Media failures now reach the error banner in plain language, where they were previously discarded. The server's stable codes are not shown: the response body is consumed by the media element, and re-requesting the URL to read it would issue a second export and a second audit row.
- [x] Keep the existing “Play this run” time-range workflow for multiple
  contiguous segments.
- [x] Do not add a seek/speed control while `Accept-Ranges: none` remains true.

UI tests:

- full URL has no query string;
- 5/10/custom duration URLs encode correctly;
- invalid custom values never initiate playback;
- clicking a row uses that row's camera and recording ID;
- changing selection stops the old media response;
- existing run playback and UTC search remain unchanged;
- empty, loading, error, and successful states render accessibly.

Exit gate:

- an operator can search recordings, choose one row, and play either all or the
  first 5/10/custom seconds without manually constructing a URL.

### Slice 5 — Real media integration and measurements

- [ ] Create deterministic completed fMP4 input with known duration and cadence.
- [ ] Retrieve full, 5-second, and 10-second variants through the authenticated
  HTTP boundary.
- [ ] Probe and fully decode every response with FFprobe/FFmpeg.
- [ ] Measure actual duration variance, startup latency, server/FFmpeg CPU and
  RSS, bytes delivered, and cleanup after client cancellation.
- [ ] Run concurrent retrieval at the confirmed viewer workload; do not infer a
  concurrency claim from one request.
- [ ] Verify recording continues and its queues remain healthy during retrieval.

Exit gate:

- output is playable and decodable;
- measured duration behavior matches the documented accuracy statement;
- no process remains after success, cancellation, or shutdown;
- playback load cannot stop or backpressure recording.

### Slice 6 — Complete repository validation and handoff

- [ ] Update `ROADMAP.md` conservatively.
- [ ] Create a new dated `context/` snapshot with commands and evidence.
- [ ] Run UI, focused HTTP, assembler, database, and authenticated fixture tests.
- [ ] Run `cargo fmt --all --check`.
- [ ] Run workspace Clippy with warnings denied.
- [ ] Run all workspace tests and the release build.
- [ ] Run `cargo deny check` and `cargo audit`.

## 8. Observability

Reuse bounded-cardinality playback metrics. Add only fields/labels with fixed
vocabularies:

- request mode: `full` or `leading_duration`;
- outcome: existing bounded success/cancel/failure categories;
- active playback count;
- started/completed/cancelled/failure totals;
- output bytes and startup/stream duration histograms if the existing metrics
  abstraction supports them without camera/recording labels.

Never use recording ID, camera ID, duration value, path, or query text as a
metric label. Structured traces may include validated camera and recording IDs
but never storage paths or credentials.

## 9. Security and abuse controls

- Preserve camera-scoped authorization and non-enumeration.
- Treat every successful or partial response as an export.
- Bound duration, query bytes, selected recording count (exactly one), FFmpeg
  process lifetime, output buffering, and stalled-client time.
- Use validated repository metadata, never a client path.
- Recheck file type and containment at use time; retention can race retrieval.
- Do not log tokens, cookies, proxy proof, filesystem locators, or raw queries.
- Add a concurrency limit before claiming multi-viewer clip support if load
  measurements show FFmpeg process count can threaten recording priority.

## 10. Compatibility and rollout

- Additive API change: no current route is removed or reinterpreted.
- Existing `playback` JSON field and UI run playback remain.
- New `video` field is additive.
- Rollback removes the new route/link/UI controls without changing stored media
  or database schema; no migration is required.
- If duration-bounded remux proves unreliable, ship full recording retrieval
  alone and leave `duration_seconds` disabled until the accuracy policy is
  resolved. Do not silently return a full recording for a requested clip.

## 11. Definition of done

- Omitting duration returns the complete selected recording.
- `duration_seconds=5`, `10`, and another valid value return playable leading
  clips whose measured behavior matches the documented accuracy.
- Invalid or excessive values fail before FFmpeg starts.
- Camera scope, role checks, and export audit cannot be bypassed.
- No file path is exposed and no unsafe path is opened.
- Every child process and stream is owned, bounded, cancelled, and joined.
- Recording and camera ingestion remain operational during success and failure.
- UI supports full, preset, and custom duration choices.
- Existing search and time-range playback clients continue working.
- Documentation, metrics, tests, roadmap, and context agree with reality.

## 12. Explicit product decision gate

Before claiming exact clip duration, confirm one of these requirements:

1. **Stream-copy duration-bounded (recommended first):** low CPU, starts at the
   recording beginning, final boundary may vary by a frame/packet.
2. **Frame-exact transcoded:** exact boundary, materially higher compute and a
   new concurrency/capacity budget.

The implementation may proceed with option 1 as a reversible default, but the
UI and API documentation must not use “exact” until measured evidence supports
it.
