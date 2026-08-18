# LiveKit load run — 10 cameras, 10 viewers, 2026-08-15

Requested workload: ten cameras publishing to LiveKit with ten subscribers
attached, recording CPU, memory, queue drops, publish failures, and latency
percentiles.

**The requested workload could not be run, because the system cannot express
it.** What was run instead, and what it measured, is below. Nothing here is a
capacity claim.

## Headline

Live publication is available to exactly one camera per process: the camera
configured from the environment at startup. Cameras added through
`POST /api/cameras` are created with live publication disabled and there is no
way to enable it.

> Superseded on 2026-08-16, for the code and not for this run.
> `AddCameraRequest` now carries `live_publication_enabled: Option<bool>`, and a
> camera added through the dashboard with it on was observed publishing
> (`docs/phase-9/authenticated-ui-2026-08-16.md`). Everything below still
> describes the binary this run was taken on, and the numbers are unchanged.

- `AddCameraRequest` (`apps/server/src/health_http.rs`) carried `camera_id`,
  `rtsp_endpoint`, `username`, `password`, `width`, `height`, and
  `detection_enabled`. There was no LiveKit field.
- `apps/server/src/pipeline.rs:144` creates a publication only
  `if let Some(livekit) = config.livekit.clone()`, and the add-camera path
  never populates it.
- Measured directly: at the end of the run `surveillance_livekit_enabled` was
  `1` for `camera-01` and `0` for `camera-02` through `camera-10`.

So "10 cameras → LiveKit" is currently "1 camera → LiveKit". The ten
subscribers had one track between them; nine had nothing to subscribe to.
Extending live publication to API-added cameras is a product change, not a
fixture fix, and is not made here.

## The publication that did exist tore itself down about once per second

`crates/livekit/src/adapter.rs` writes each access unit to the `lk` child over
a Unix socket bounded by `ipc_write_timeout`. If the write does not complete in
time, `publish_session` (`crates/livekit/src/owner.rs:298-305`) counts a send
failure and returns `SessionExit::Retryable(Transport)`, which tears down the
whole publication and reconnects with backoff.

`ipc_write_timeout` is **250 ms**, hardcoded at
`apps/server/src/runtime_config.rs:363`. That is below the observed worst-case
time for a single access-unit write to the child, so the publisher destroys and
rebuilds its session continuously.

A/B, one camera, no viewers, identical `testsrc2` 640×360@15 source, identical
`lk` 2.18.2 and livekit-server 1.13.5, 60 s window, only the timeout changed:

| | 250 ms (shipped) | 5000 ms (patched) |
|---|---|---|
| end state | 4 = Backoff | 3 = Publishing |
| connection attempts | 8 | 1 |
| reconnects | 7 | 0 |
| send failures | 8 | 0 |
| access units | 123 | 362 |
| keyframes | 9 | 14 |
| parameter set waits | 105 | 8 |
| bytes published | 1,304,729 | 3,764,601 |

The failure category was `Transport`, never `ChildExited`: the `lk` child stays
alive throughout. This is a write-pacing failure, not a media failure, not a
parameter-set problem, and not `lk` crashing.

The 5000 ms figure is a diagnostic that isolates the cause. It is not a proposed
default — a write timeout that long stops detecting a genuinely wedged child.
Choosing between a larger socket buffer, a drop-rather-than-teardown policy on
write pressure, and a different timeout is a design decision that belongs in an
ADR. The patch was reverted; `crates/livekit/src/adapter.rs` and
`apps/server/src/runtime_config.rs` are unmodified.

## Two defects found alongside it

**The `camera.livekit.*` persistent settings do nothing.**
`CameraLiveKitIpcWriteTimeout`, `CameraLiveKitQueueItems`, and
`CameraLiveKitMaxAccessUnitBytes` appear only in `crates/config/src/defaults.rs`
and `crates/config/src/classification.rs`. They are declared, defaulted,
validated, and audited, but no code reads them into `LiveKitPublicationConfig`.
Every value the publisher uses comes from the hardcoded literals at
`apps/server/src/runtime_config.rs:355-368`. An operator who sets
`camera.livekit.ipc_write_timeout` gets an accepted, audited, persisted change
with no effect.

**The `lk` child's diagnostics are discarded.**
`crates/livekit/src/adapter.rs` sets `stderr(Stdio::null())`, and the
teardown/reconnect cycle emits no log line at INFO. A publisher can flap
indefinitely while the server log stays silent. With stderr inherited, `lk`
reports its join plan, room connection, and track publication — enough to
diagnose this in minutes rather than hours.

## What the run did measure

Ten cameras ingesting and recording concurrently, one of them also attempting to
publish. `scripts/phase9/livekit-load-fixture.sh`, `CAMERAS=10 VIEWERS=10
WINDOW=180 SAMPLE_EVERY=5`, 640×360@15, GOP 30, release build, defaults
unpatched. Artifacts in `/Users/abuhamzah/.cache/tmp/livekit-load.f1Pq6s`.

Measured wall time 187.76 s.

Ingest and recording, over the window:

| | |
|---|---|
| cameras connected at start and end | 10 of 10 |
| `surveillance_camera_reconnects_total` | 0 |
| `surveillance_camera_terminal_failures_total` | 0 |
| `surveillance_recorder_segments_completed_total` | 60 |
| `surveillance_recorder_segments_failed_total` | 0 |
| recorded to disk | 337.5 MiB (15.0 Mib/s aggregate) |

Bounded queues, by consumer, deltas over the window:

| | detection | live | recorder |
|---|---|---|---|
| capacity | 20 | 40 | 320 |
| depth at end | 0 | 4 | 0 |
| accepted | 0 | 2,815 | 28,150 |
| received | 0 | 273 | 28,150 |
| rejected (full) | 0 | 0 | 0 |
| dropped (oldest) | 0 | 2,538 | 0 |

The recorder queue lost nothing: 28,150 accepted, 28,150 received, zero dropped.
The live queue dropped 2,538 of 2,815 — that is the freshness-preferring policy
behaving exactly as designed while its consumer spent most of the window in
backoff, not an independent defect. `surveillance_camera_live_evictions_total`
was 2,538, matching.

CPU, average fraction of one core over the window (100 = one core saturated):

| process | | |
|---|---|---|
| `surveillance-server` (pid 63732) | **4.1%** | 7.65 s CPU |
| livekit-server (pid 63062) | 2.9% | 5.45 s |
| MediaMTX (pid 63073) | 10.3% | 19.29 s |
| 10 × ffmpeg synthetic sources | ~7.0% each | ~70% total |
| 10 × `lk` subscribers | ~0.1% each | |

The ffmpeg sources, MediaMTX, and livekit-server stand in for cameras and an SFU
and are not part of the measured system. The server figure covers ten RTSP
sessions, ten recorders, and one flapping publisher on a MacBookPro15,1
(16 CPUs, 32 GB, macOS 15.7.7).

## Not measured

**Viewer-perceived latency — no p50/p90/p99 is reported.** There is no
end-to-end live latency histogram in this system; the only latency buckets are
the detector's. The requested percentiles cannot be derived from the metrics
that exist, and none are estimated here.

**Memory.** The CPU sampler records cumulative CPU seconds per pid and its
children; it does not snapshot RSS. That was an omission in the fixture, not a
measurement that failed.

**Ten concurrent publications, and viewer scaling.** Blocked by the enablement
gap above. Ten subscribers were started and nine had no track available, so this
run says nothing about ten-viewer behaviour.

## Reproducing

```bash
CAMERAS=10 VIEWERS=10 WINDOW=180 SAMPLE_EVERY=5 \
  bash scripts/phase9/livekit-load-fixture.sh
```

Needs `livekit-server`, `lk`, `mediamtx`, and `ffmpeg` on PATH. The fixture
treats camera connectivity as a precondition and publication state as a measured
outcome — deliberately, since whether publications hold is one of the questions.
