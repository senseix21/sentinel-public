# Repository status — 2026-09-01 10:49 +06

Base: `12175b4`. Worktree changes in this slice are uncommitted.

## Verified facts

- The production Rust runtime already connects its Retina owner directly to the
  configured RTSP endpoint. No MediaMTX production dependency existed in the
  camera, distributor, recorder, detector, or LiveKit code, so no media-path
  rewrite was justified.
- `scripts/phase9/direct-camera-qualification.sh` now owns a one-camera release-
  server run without starting MediaMTX. It refuses credential-bearing endpoints
  and a local MediaMTX process, requires one server-owned TCP peer on the RTSP
  port, samples health/metrics/resources, gracefully joins the server, probes and
  fully decodes durable segments, and stores private evidence under `umask 077`.
- `scripts/phase9/direct-camera-report.py` applies a camera-scoped metric gate.
  It requires connected media flow, at least two segments completed during the
  window, zero terminal/teardown/recording failures, and zero recorder queue
  rejection. Three regression tests cover a healthy window, recording failure,
  and disconnected/empty media; another camera's counters are ignored.
- The detailed governing SPEC, staged PLAN, and evidence template live under
  `docs/direct-camera-rtsp/`. Architecture, roadmap, Makefile, and the camera-
  offline runbook link to them.
- The first authenticated external fixture run exposed an existing fixture bug:
  its owned FFmpeg publisher sometimes survived `SIGINT`, so the recovery
  publisher could not claim the MediaMTX path. The controller now escalates
  through bounded TERM/KILL and proves the old publisher is absent before the
  next epoch.

## Validation

- Final `make check`: passed after the fixture shutdown correction. This covered
  shell syntax, 13 Python tests, 27 Node tests, formatting, Clippy with warnings
  denied, all non-ignored workspace Rust tests, release build, `cargo deny`, and
  `cargo audit`.
- `scripts/phase0/retina-rtsp-fixture.sh`: passed after the correction — five
  authenticated/rejection/saturation/ownership/reconnect RTSP tests and the real
  three-segment fMP4 test.
- `scripts/phase3/multi-camera-fixture.sh`: passed its synthetic ten-camera and
  two-camera failure-isolation workload.
- Dependency audit retained two allowed transitive warnings: unmaintained
  `number_prefix 0.4.0` and yanked `chacha20 0.10.1`.

## Not verified

- No representative physical camera or target server was available.
- The direct-camera harness has not been executed against a private camera
  endpoint; it is tested as code and by its metric parser, not as camera evidence.
- Camera reboot/network interruption/credential rotation authority, sustained
  soak objectives, and 10/20-camera target-hardware workloads remain external
  gates.
- H.265, audio, ONVIF, and PTZ remain outside the implemented H.264 MVP slice.
