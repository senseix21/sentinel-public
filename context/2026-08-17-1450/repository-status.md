# Repository status — 2026-08-17 14:50 +06

Base commit: `e65a318` (`docs: add Phase 8 handoff`). The worktree is dirty with
in-flight Phase 5 live-publication work and is intentionally left that way.
Nothing was committed.

## What changed this session

Live video works **from a cold start**. The dashboard's `<video>` element decodes
and renders the camera. That is the thing the previous snapshot recorded as
unproven.

The qualifier was added 2026-08-17 evening and is not decoration: the live lane
was later measured to stall after a camera reconnect, while health continues to
report `Publishing`. See the evening amendment at the end of this file. Every
live measurement in this snapshot was taken before any reconnect had occurred, so
it describes the cold-start case only.

The route there was ADR 012: stop trying to pass encoded H.264 through
libwebrtc, and decode and re-encode instead — behind a gate so that nothing
decodes while nobody is watching.

## Verified — full gate

`make check`, exit 0:

| Step | Result |
|---|---|
| `cargo fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | clean |
| `cargo test --workspace --all-features` | **43 suites, 508 passed, 0 failed, 34 ignored** |
| `cargo build --workspace --release` | succeeded |
| `make audit` | `cargo deny`: advisories ok, bans ok, licenses ok, sources ok. `cargo audit` ran. |

`AGENTS.md` now records 508 / 34; the previous recorded baseline was 469 / 32 and
the intermediate reality was 492 / 33.

`cargo deny` still warns about duplicate `thiserror` 1.0.69 / 2.0.20. A warning,
not a gate failure, not fixed here.

## Verified — live, on the local stack

`scripts/dev/local-stack.sh`: MediaMTX, a 1280×720@15 H.264 webcam bridge,
`livekit-server`, PostgreSQL, the trust-boundary proxy, and the dashboard. Driven
through an authenticated session in Chromium.

Browser-side, read out of the page:

```
video: 1280x720, paused=false, readyState=4, currentTime advancing,
       getVideoPlaybackQuality().totalVideoFrames = 416
```

Server-side, from `/metrics` on `mac-webcam`:

| | no subscriber | one subscriber |
|---|---|---|
| `surveillance_livekit_state` | 3 (connected) | 3 (connected) |
| `surveillance_livekit_viewers` | 0 | 1 |
| `surveillance_livekit_published_frames_total` | 0 | climbing at 8.85/s |
| `surveillance_livekit_access_units_total` | 0 | climbing at 9.05/s |
| `surveillance_livekit_adapter_drops_total` | climbing | flat |
| `surveillance_livekit_adapter_restarts_total` | 0 | 0 over 60 s |
| `surveillance_livekit_send_failures_total` | 0 | 0 |
| `surveillance_livekit_reconnects_total` | 0 | 0 |

Published-versus-arrived is 98%. The pass-through path died at access unit 9;
this is past that by two orders of magnitude.

## What is *not* verified, stated plainly

- **The live path published 8.85 frames/s from a source that was delivering
  15.00.** Corrected 2026-08-17, later the same day — see the amendment at the
  end of this file. The earlier text here inferred a ~9 FPS webcam from the
  access-unit rate. The recorder's own output disproves that, so the shortfall is
  inside the live path and is unexplained.
- **No latency figure, and none is obtainable through this adapter.** It reports
  `TimestampAuthority::AdapterClock` because B-frame reordering makes the
  frame-to-access-unit mapping unrecoverable. `/metrics` shows
  `surveillance_livekit_media_timestamp_authority 0`. A number needs a
  viewer-side harness or a timestamp-preserving path.
- **One camera, one subscriber.** No CPU-per-viewer measurement, so ADR 012's
  second reversal criterion still has no number behind it. No capacity claim
  follows from anything above.
- **One browser.** Chromium, headless, driven by Playwright. Its
  `totalVideoFrames` rate read far below the server's publish rate; that is not
  explained and is not treated as a server-side fact.
- Representative High Profile cameras, H.265, and time-to-first-picture are
  untouched.

## Code

New:

- `crates/livekit/src/transcode.rs` — `TranscodePublicationAdapter`. Owns a room,
  its event stream, and — only while a remote participant is present — a
  published track, a `NativeVideoSource`, and an FFmpeg decode session. Room
  events are drained inline in `send_access_unit` and the viewer count is
  *re-read* from `room.remote_participants().len()` rather than incremented, so a
  missed event cannot leave permanent drift. No spawned task, no `Room` clone.
- `crates/livekit/build.rs` — `rustc-link-arg-tests=-ObjC` on Apple targets.

Changed:

- `crates/livekit/src/decode.rs` — split write budgets, see below.
- `crates/livekit/src/adapter.rs` — `AdapterObservation` and a defaulted
  `PublicationAdapter::observe`. Three narrow numbers, so a pass-through adapter
  reports zeros and the owner stays generic.
- `crates/livekit/src/owner.rs` — samples `observe()` on every access unit, before
  the outcome is classified, and *stores* rather than accumulates.
- `apps/server/src/pipeline.rs` — selects the transcode adapter.
  `NativePublicationAdapter` is kept, not deleted, so reversal is a one-line
  constructor change.
- `apps/server/src/health_http.rs` — three new series.
- `crates/config`, `docs/phase-7/contracts.md` — two new settings,
  `camera.livekit.decoder` and `camera.livekit.max_publish_height` (77 → 79).

## Three defects found by running it, not by reading it

1. **`-fflags nobuffer` on a raw H.264 pipe produces zero output, exit 0, no
   diagnostic.** Measured against real media on ffmpeg 8.0: 0 bytes versus
   1 152 000 bytes, silent either way. Removed, along with `-flags low_delay`,
   which is only correct for B-frame-free streams. `-flush_packets 1` is the
   low-latency knob that actually works.
2. **`shutdown` killed FFmpeg before it flushed.** Now a bounded `child.wait()`
   under `SHUTDOWN_GRACE` with the kill as a backstop.
3. **Every subscribe cost one `Overloaded` restart.** A single 250 ms write
   budget conflates a decoder that has stopped keeping up with one that has not
   started reading yet — a cold FFmpeg drains nothing, and the first access unit
   of a stream is the largest. The first write now gets
   `SPAWN_WRITE_TIMEOUT` (2 s) and everything after it `WRITE_TIMEOUT` (250 ms);
   one completed write is the proof the decoder is reading. Raising the single
   budget would have blinded the steady-state detector for the whole session.
   Regression test: `a_cold_decoder_gets_the_startup_budget_for_its_first_write`.

Defect 3 was found *because* `surveillance_livekit_adapter_restarts_total` was
added in the same session. The restart path had no log line; it has one now,
carrying the `DecodeError` kind, because the counter says a restart happened and
nothing else said why.

## A previously unrunnable test now runs

`crates/livekit/tests/publisher_fixture.rs` aborted with `SIGABRT` inside
`+[RTCDefaultVideoEncoderFactory supportedCodecs]` — `doesNotRecognizeSelector`,
the classic dropped-Objective-C-category link failure. `apps/server/build.rs`
fixes it with `rustc-link-arg-bins=-ObjC`, which covers the server binary and
nothing else. The new `crates/livekit/build.rs` extends it to this crate's test
binaries, and `livekit_outage_retries_without_blocking_recorder_queue` — Phase 5
failure-isolation evidence — passes for the first time. The test was never
broken; it could not be linked.

## Amendment, 2026-08-17 later the same day — recording verified, and a claim withdrawn

Prompted by a check of whether recording works. It does, and the check falsified
one of this snapshot's own numbers.

### Recording, verified on both sides

Filesystem, `ffprobe -count_frames` over the eight consecutive segments spanning
the live watch (`08-41-49` through `08-45-19` UTC):

| | value |
|---|---|
| segments | 8, contiguous |
| frames | 450 in every segment, 3 600 total |
| duration | 30.000000 s in every segment, 240.000 s total |
| rate | **15.000 fps, in every segment** |
| codec / size | h264 Constrained Baseline, 1280x720, ~5.6 MB each |

Index, `recording_segments` for the same run (15 rows, 14:41:49–14:48:51 +06):
every row `completed`, provenance `recorder`, `fragmented_mp4`/`h264`/1280x720,
`ended_at` of each row exactly equal to `started_at` of the next — no gaps, no
overlaps. `size_bytes` equals `stat` for all 44 of today's completed rows, and
all 44 files are present. The run's final segment is 2.133 s / 33 frames /
444 695 bytes, finalized rather than left partial, and plays: that is the
shutdown cut, recorded honestly rather than padded to 30 s.

**Recording was undisturbed by concurrent decode-and-re-encode.** The live watch
ran inside this window and every segment across it is a full 450-frame, 30.000 s
file. That is direct evidence for the subsystem priority this repository
committed to — preserve recordings above serve live video — rather than an
argument from the design.

### The withdrawn claim

This snapshot and ADR 012 said the webcam delivered about 9 FPS, inferred from
`surveillance_livekit_access_units_total` climbing at 9.05/s. **That inference was
wrong.** The recorder consumed the same stream from the same distributor and got
15.000 fps, measured frame by frame.

So the source was 15 fps and the live path published 8.85/s of it — about 59%.
The missing ~6 frames/s are lost upstream of publication, not at the camera.
Candidates, none of them measured: the live distributor's freshness-preferring
eviction, and the owner's stale-drop before the adapter is offered an access unit.
`surveillance_livekit_adapter_drops_total` was flat during the subscriber window,
which points away from the adapter and toward the distributor, but that is
reasoning, not a measurement. Resolving it needs a run with
`surveillance_camera_live_evictions_total` sampled alongside the publish rate.

This also reframes the open browser-frame-rate question: the shortfall now has a
server-side component that starts before the browser is involved.

### The index's other 191 rows

Not from this run, and worth stating so they are not mistaken for a fault here:

| status | rows | files present | files gone |
|---|---|---|---|
| `completed` | 44 | 44 | 0 |
| `inconsistent` | 178 | 108 | 70 |
| `incomplete` | 13 | 9 | 4 |

The 9 present `incomplete` rows are the nine `.partial.mp4` files — forced-exit
evidence, indexed as such. The 8 rows marked `inconsistent` during today's
startup pass are all rows whose file was deleted between sessions: the probe
fails, and `mark_inconsistent` flags the row rather than deleting it. **No
present, probeable file was newly flagged today** — zero false positives from
this run's reconciliation.

The remaining 170 were flagged on 2026-08-16 across dev sessions that wiped the
database and deleted footage. 108 of those files are still on disk and probe
cleanly now, which surfaces a real operational gap rather than a code defect:
`RecordingStatus::Inconsistent` may transition only to `Quarantined`
(`crates/storage/src/metadata.rs`), and `reconciliation_owner.rs` takes the
`Ok(_) => {}` no-op when a probe later succeeds for an already-flagged row. That
is deliberate — an index that silently re-blesses footage it once disagreed with
is worse — but it means **there is no implemented operator path to re-index a
playable file that was flagged once.** Today that is dev-history residue; on a
real deployment it would be 108 pieces of retrievable footage marked unusable
with no supported way back.

## Amendment, 2026-08-17 evening — playback and the API verified, two new defects

Prompted by a check of whether playback and the API work. Both do. The run also
exposed two defects that reading the code would not have found, and undercut a
hypothesis recorded earlier in this same file.

### Playback, verified by decoding the bytes

`GET /api/cameras/mac-webcam/recordings/playback?start=&end=` over a 60 s window
returned 11 274 711 bytes, assembled from multiple overlapping segments into one
continuous MP4 and streamed chunked.

| | value |
|---|---|
| body | 11 274 711 bytes |
| `ffprobe` | 900 frames, 60.000000 s, h264 Constrained Baseline, 1280x720 |
| `ffmpeg -f null -` | decoded clean, zero errors |
| headers | `Accept-Ranges: none`, `X-Playback-Accuracy: segment-aligned` |

The headers are the honest part: no seeking is offered and accuracy is declared
at segment granularity, in machine-readable form rather than in prose.

### The API surface

| check | result |
|---|---|
| no credential on `:3000` | 401 |
| camera, health, search, detail | 200, expected shapes |
| storage paths or credentials in any response body | none |
| invalid range | 400 |
| unknown recording | 404 `recording_not_found` |
| non-contiguous span | 409 |
| path traversal in the camera id | 400 `invalid_camera_id` |

Search returned 15 rows carrying `playback` URLs. The export audit is exact:
**five `authorization` grants but one `export` row**, whose `byte_count` is
11 274 711 — the bytes that actually left, not the bytes authorised. Playback
metrics agreed: 1 started, 1 completed, 0 failures.

### Defect — a reconnect mid-segment can produce playable footage the index buries

One occurrence in `server.log`:

```
ERROR recording metadata persistence rejected immutable evidence category=Conflict
```

`12-25-22--r-6a82fdb2-24c8630d-d.mp4` is a **decodable 224-frame file** whose
container duration reads `56982.764400` — garbage. Its row is permanently
`incomplete` / provenance `reconciliation`, the API 404s it, and **no metric
counts the rejection**: `surveillance_metadata_rejected_full_total` is 0.

So the failure is silent in all three places an operator would look — the API,
the metrics, and the row's own status. The file is retrievable footage and
nothing offers it. This is the same shape as the `Inconsistent` one-way door
above, reached by a different route, which is why it belongs with it rather than
as an isolated bug.

### Defect — the live lane stalls after a reconnect, and health reports `Publishing`

Sampled over 40 s while media was arriving:

| series | delta |
|---|---|
| `live_offered` | 1523 → 1523, **0** |
| `live_dropped` | 1884 → 2044, **+160** |
| `live_publication_adapter_drops` | 1344 → 1344, **0** |
| `live_published_access_units` | 0 → 0, **0** |
| `recorder_completed_segments` | 21 → 22, +1 |

`live_queue_depth: 4 / capacity: 4` — permanently full,
`surveillance_camera_live_evictions_total` 7075 and climbing. Meanwhile health
reported `live_publication_state: "Publishing"` and `surveillance_livekit_state`
3 (connected).

Two checks narrow it, and both matter because the obvious reading is wrong:

- **It is not an empty stream.** With the camera disconnected later the same
  evening, *no* live counter moved across 30 s — verified by diffing `/metrics`.
  So the 40 s sample was a starved consumer with media genuinely arriving, not a
  quiet pipeline.
- **The owner did not exit.** `server.log` contains zero
  `optional LiveKit publication stopped` lines, which is what
  `apps/server/src/pipeline.rs` logs when the publication future completes. The
  owner is stalled inside its loop, and `adapter_drops` being flat puts the stall
  *upstream of the adapter* — the owner is not even reaching
  `send_access_unit`.

`adapter_drops` was climbing before the reconnect and frozen after, which points
at the reconnect as the trigger. That is where the diagnosis stops: the
mechanism is not identified and no root cause is claimed here. Recording was
unaffected throughout — `recorder_completed_segments` kept advancing — which is
the priority ordering holding again.

**Reproduced deliberately, same evening.** The webcam bridge was restarted, the
camera reconnected, and media flowed again. Sampled over 30 s:

| series | delta |
|---|---|
| `surveillance_camera_live_evictions_total` | 7221 → 7565, **+344** (~11.5/s) |
| `surveillance_livekit_adapter_drops_total` | 1344 → 1344, **0** |
| `surveillance_livekit_access_units_total` | 0 → 0, **0** |
| `surveillance_livekit_published_frames_total` | 0 → 0, **0** |

So it is not a one-off. It reproduces across a second, independently created
camera session, and the eviction rate is close to the 15 fps source — the
distributor is throwing away essentially the whole live lane.

The sharpest version of the anomaly is in `adapter_drops`, and it needs the
earlier evidence table in this file to see: with **no** subscriber that counter
*should climb*, because the adapter is offered every access unit and drops it
(250 and climbing, in the original table). It is now frozen with no subscriber.
That is the signal that the owner is not consuming at all, rather than consuming
and discarding.

`viewers` is 0 throughout, so nothing decodes and no CPU is being burned. Whether
a subscriber arriving would still get a picture was **not** tested, so no claim is
made either way about what a viewer would see.

The operator-facing part is the worst part: live video is dead and every signal
says it is fine.

### A hypothesis from earlier in this file, undercut

The amendment above and ADR 012's correction both suggested the live
distributor's freshness-preserving eviction as the likely home of the 41%
publish shortfall. Pre-reconnect health shows `live_offered: 713,
live_dropped: 12` — **1.7%**, nowhere near enough to account for it. Both texts
labelled that reasoning rather than measurement, so this is not an erratum; it
is the measurement they asked for, and it eliminates the leading candidate
without naming a replacement.

### Not verified

The dashboard's own playback rendering. The Playwright check timed out at 120 s
and was killed. Playback is proven at the API; the UI drawing it is not.

### Environmental, not a defect

MediaMTX tore down the publisher at 18:25:55 +06 because the machine was
CPU-saturated (load 5.64, an unrelated process at 805% CPU). The server
recovered with no operator action: `connection_epochs` 1 → 2, `reconnects: 1`,
`terminal_failures: 0`, segments 12 → 22, new segments at exactly 15.00 fps. The
camera is down as of this writing because the webcam bridge was killed during
testing and not restarted — also not a defect.
