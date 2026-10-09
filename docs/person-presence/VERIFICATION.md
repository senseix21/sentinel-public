# Person presence qualification — 2026-10-09

Local development evidence only. The user corrected their earlier response: they
had not performed the physical webcam turn-away test. The tests below were run
by the coding agent with real ONNX models, not inferred from that response.

## Real-model and browser evidence

Both models use a 70% threshold. Real-model external tests passed for front-facing,
head-obscured, stationary repeated, blank and actual back-facing inputs.
The back-facing input isolates the left subject of COCO128 image 000000000077;
the full image contains another subject with a visible profile and is unsuitable
for asserting zero faces. These fixtures do not establish population accuracy.

A marked H.264 scene loop went through MediaMTX RTSP, the single Rust ingest,
detection decoder, real model lanes, domain events, LiveKit and Chromium:

| Decoded scene | People observed | Faces |
| --- | ---: | ---: |
| Front-facing public bus image | 3 | 2 |
| Back-facing cropped public COCO image | 1 | 0 |
| Empty frame after expiry | 0 | 0 |

The browser determined the scene from a decoded color marker and inspected the
actual overlay and fresh non-fixture packets. Subjects differ between scenes;
this does not prove continuous identity association for a physical person turning.
Recording completed at least one segment with zero segment failures during this
controlled run. MediaMTX reported exactly one surveillance RTSP reader.
Evidence: `.local/presence-scenes-browser.log`, `presence-scenes-metrics.txt`,
`presence-scenes-sessions.json`. Reproduction helpers:
`scripts/dev/build-presence-scenes.sh`, `scripts/dev/check-presence-scenes.cjs`.

An intentionally invalid but correctly hash-pinned face artifact then exercised
startup failure. Person lane state was Running (2), face Terminal (4). Two browser
tabs continued decoding for 16 seconds, received real available person snapshots,
and received no face packets. Recording completed one segment with zero failures.
Evidence: `.local/presence-failure-browser.log`, `presence-failure-metrics.txt`.
Use the normal browser helper with `SENTINEL_EXPECT_PRESENCE=true` and
`SENTINEL_EXPECT_FACE_UNAVAILABLE=true` for this case.

## Engineering checks

- Workspace all-feature tests: 587 passed, zero failed, 38 ignored in the saved
  run. Two real-model external tests were subsequently executed explicitly and
  passed; ignored tests remain separately qualified, not counted as workspace passes.
- Workspace all-target/all-feature Clippy with warnings denied: passed.
- Formatting: passed. UI: 46 passed. Python development helpers: 21 passed.
- All-feature release build: passed with `CARGO_PROFILE_RELEASE_STRIP=none`.
  Plain stripped macOS release remains affected by a SQLx proc-macro dylib
  LINKEDIT alignment problem; the workaround is not proof that plain release passes.
- Dependency audit gate could not run: cargo-deny is missing; cargo-audit is also
  unavailable. Consequently `make check` is not claimed green.

## Reliability and production gates

A longer development fixture run stopped ingestion with RecorderBackpressure
(one full recorder queue rejection after 5,005 access units and nine completed
segments). LiveKit closed-engine warnings preceded it. The cause is unresolved;
controlled short-run isolation does not establish sustained recording reliability.
Evidence: `.local/presence-long-run-server.log`. Investigate this before claiming
production readiness; do not conceal it by increasing queue capacity.

Physical webcam turn-away, Firefox, representative accuracy, total ORT/FFmpeg
memory, target hardware and 10/20-camera loads remain unverified. Hung foreign
inference cannot be forcibly cancelled in process. Tracker association uses a
provisional IoU baseline, not a validated motion model or unique-person counter.
The specification remains a target contract where its production acceptance
criteria exceed these local measurements. No enterprise-readiness claim is made.
