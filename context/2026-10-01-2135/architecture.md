# Architecture — 2026-10-01 21:35 +06

Unchanged from `context/2026-09-05-1116/architecture.md` in its ingest boundary:
the camera owner accepts an exhaustive H.264 source configuration, physical IP
cameras connect directly over Retina RTSP/TCP, local webcams are captured by a
server-owned FFmpeg child emitting bounded Annex-B into the same source
lifecycle, and the distributor, recorder, detector, and LiveKit boundaries are
identical for both source kinds.

Two additions this session.

## Child-process ownership

The spawned FFmpeg process is owned by its `Child` value rather than by the code
paths that observe a failure. `FfmpegSourceConfig::command` sets
`kill_on_drop`, so the process dies at every await point, including cancellation,
where no error arm runs. The explicit `terminate_child` calls remain as the
ordinary path; `kill_on_drop` is the floor beneath them. A session dropped
without `shutdown` no longer leaks either.

## Recording retrieval

Recording media retrieval is a separate resource from time-range playback:
`GET /api/cameras/{camera_id}/recordings/{recording_id}/video`, optionally
bounded by `duration_seconds`. One recording identity maps to one media object,
so an omitted duration has a single meaning. It reuses the existing export
authorization, audit fact, bounded buffer, stall and startup timeouts, and
owned-child lifecycle, and adds no camera connection and no decode or re-encode
path. Accuracy is packet and keyframe bounded, not frame exact (ADR 014).

## Enforcement boundary

The quality gate is now split by determinism rather than by content.
`make verify` is everything whose result depends only on the tree: shell syntax,
the Python suites, the Node UI tests, formatting, Clippy, tests, and the release
build. `make audit` is the part that depends on the outside world, namely the
advisory database. Pull-request CI runs the first, a daily schedule runs the
second, and `make check` still runs both.
