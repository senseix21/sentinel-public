# Repository Status — 2026-10-01 21:35 +06

## Verified facts

- Branch: `feature/ffmpeg-webcam-source`, open as pull request #10 against
  `main`. Base commit before this session: `12175b4`.
- Five commits were created from a previously dirty worktree, split by arc:
  `d80ab3b` direct physical-camera qualification harness, `70e8be4`
  source-neutral FFmpeg source with MediaMTX retired, `d78702f` recording video
  subresource and playback UI, `a2a5542` rustls 0.23.45, `70de120` FFmpeg child
  teardown on cancellation.
- `make check` passed on the final tree: 504 tests passed, 0 failed, 30 ignored
  across 42 targets; Clippy with warnings denied; release workspace build;
  `cargo deny` reported advisories, bans, licences, and sources ok; `cargo
  audit` scanned 461 crates.
- `cargo audit` emits two warnings that predate this branch and do not fail the
  gate: `number_prefix` 0.4.0 unmaintained (RUSTSEC-2025-0119, through
  `indicatif` and `muxide`) and `chacha20` 0.10.1 yanked (through `rand` and
  `quinn-proto`).
- A cancelled `FfmpegH264Session::start` leaked its child process. The error
  arms called `terminate_child`, but a dropped future takes no arm, so a
  supervisor shutdown landing inside the startup window left ffmpeg alive
  reparented to pid 1. Measured: spawned 17:48:49, shutdown requested 17:48:55,
  process still running 42 minutes later holding the capture device. Fixed with
  `kill_on_drop`; a hermetic regression test cancels startup and requires the
  process to be gone, and it fails on `70e8be4`.
- `make run` cannot complete on this host. The camera device opens and answers
  format queries, but no frames arrive: a bare
  `ffmpeg -f avfoundation -i 0 -frames:v 5` was killed at 30 seconds having
  produced nothing, with no competing process and the device free. The server
  reports `failure=Timeout` and `access_units=0`. No TCC denial appears in the
  system log, which matches a consent request that cannot be displayed.
- Each stack bring-up runs a bounded retention pass that deletes real
  recordings: 100 segments and 1.35 GB on the first attempt, 100 segments and
  626 MB on the second, at `pressure: Warning` with 96 GB free of 500 GB.
- A fresh clone builds and passes `make verify` with no inherited artifacts:
  504 tests passed, 0 failed, across 42 targets. A cold build is not hermetic,
  though: `webrtc-sys` downloads a prebuilt WebRTC archive from GitHub releases
  in its build script and hard-unwraps the HTTP result. That fetch failed three
  times on 2026-10-01 — once with `operation timed out` during the first gate
  run and once with `Connection reset by peer (os error 54)` during the fresh
  clone — and succeeded on retry each time. Pull-request CI therefore warms that
  download in a separate retried step so a transient fetch is not reported as a
  failing gate; the gate itself still runs once.
- Phase 1 now has CI: `.github/workflows/ci.yml` runs `make verify` on
  `macos-latest` for every push and pull request, and
  `.github/workflows/audit.yml` runs `make audit` daily and on demand.
  `make check` is unchanged in meaning; it is now `verify` plus `audit`.

## The prototype

Carried forward unchanged from the earlier snapshots so this remains the latest
record of it. The original `cam_rtsp` prototype survives as `src/`, `examples/`,
and `tests/`. It is outside the workspace, is not compiled, and is not shipped.
Its known defects are the reason the migration exists: a single `default_stream`
identity; independent connections per feature from the same source; one
continuous recording file that ignores the configured directory; a GStreamer bus
loop that can stop draining after its first idle timeout; fabricated metadata and
one-hour durations; filesystem-scanning clip selection with keyframe-imprecise
seeking; placeholder configuration endpoints; and substantial dead or parallel
paths in `StreamProcessor` and `FileRecorder`. The MediaMTX code it also
contained was removed this session by ADR 013. Read it for proven RTSP,
GStreamer, FFmpeg, and H.265 mechanics only; none of its behavior is current.

## Still unverified

- Physical webcam capture on any platform, including this macOS host, because
  the camera consent prompt cannot be shown to a non-interactive process.
- Linux V4L2 and Windows DirectShow capture.
- Mixed RTSP and FFmpeg multi-camera topologies.
- Replacement external RTSP authentication and reconnect fixtures for the
  deleted proxy harness.
- Recording-video slice 5: deterministic fMP4 input, FFprobe decode of the
  full, 5-second, and 10-second variants, duration variance, startup latency,
  CPU and RSS, bytes delivered, cancellation cleanup, and concurrent retrieval.
- 10/20-camera capacity, viewer latency, and target deployment hardware.
