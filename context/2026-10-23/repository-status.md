# Repository status — 2026-10-23

Focused recovery work on fix/rtsp-consumer-recovery.

## Changes and failure mechanisms

Detection decoder reset/Drop joined its output reader while the receiver of its
bounded one-frame queue remained alive. When that queue was full, the reader
blocked on send forever even after FFmpeg was killed. Teardown now drops the
receiver first. A deterministic full-queue regression failed on the old code
and passes now. Combined detection exposes Starting/Degraded until frames return,
with decode attempts visible before waiting and a focused health regression.

Live publication reset pacing on timestamp discontinuity, but retained the old
RTP mapper. Backward/new camera timestamps were rejected indefinitely. The owner
now treats connection epoch, backward timestamp/timebase, and forward pacing
resynchronisation as media-run boundaries. It allocates a monotonic run epoch,
clears RTP/parameter-set state and waits for a parameterised keyframe. The
transcoding adapter replaces its decoder when that run changes, keeping the
room and track. Rejections/resynchronisation report Degraded until acceptance.
Regression covers same-epoch reversal and new camera epochs with both backward
and forward timestamps. PublicationAdapter epoch semantics and ADR 012 updated.

scripts/dev/check-rtsp-recovery.cjs is a repeatable, opt-in isolated fixture:
it validates loopback URLs and a local media file, replaces the selected
MediaMTX publisher, and owns/joins the replacement publisher and browser. It
keeps two existing viewers and asserts new camera epoch, inference/publication,
a full new recording segment, no recorder loss, and exactly one upstream reader.
README documents setup; make verify checks script syntax. ROADMAP records local
camera recovery separately from still-open LiveKit-server recovery qualification.

## Verification

Formatting and all-target/all-feature Clippy passed. All-feature workspace tests:
590 passed, 39 ignored. Two ignored real FFmpeg decode tests executed separately
and passed. Default make verify passed (including UI and Python helpers) with
CARGO_PROFILE_RELEASE_STRIP=none. All-feature release passed with that workaround.
Raw logs: target/verification-2026-10-23/. Old-code regression failures are saved
there as live-before-fix.log and detection-before-fix.log.

Apple Silicon macOS; file-backed H.264 1280x720 at 20 FPS through MediaMTX and
one Rust RTSP reader; combined real YOLOX/YuNet CPU inference; PostgreSQL,
LiveKit and Chromium. Three automated recovery cycles passed. The earlier stronger
replay: epoch 4->5, inference 6907->7148, live units 2942->3548, segments 30->32,
zero recorder rejection/failure, both existing viewer frame counters advance,
and exactly one upstream reader. All 33 finalized recordings decoded.
A final clean-state replay under clean/ passed with epoch 1->2, inference
29->270, live units 27->631, segments 0->2 and advancing frames in both viewers.
All three clean recordings decoded and no ERROR occurred in that clean server log.
Rust server/detection joined within a second. Whole launcher shutdown was one
second in the earlier run but 26 seconds in the clean run, waiting on LiveKit.
No test services remain listening. This is local recovery evidence, not capacity
or soak, and does not qualify uniformly prompt shutdown of the entire stack.

Manual execution/approval gaps left the source offline for long intervals.
The first manual browser check timed out on its observation window and is not
counted as passing. Two recording metadata immutable-evidence Conflict errors
occurred after long source-offline gaps; separate indexing/reconciliation
investigation is needed. No metadata/recording policy changes were made here.

Dependency advisory findings, missing cargo-audit, the stripped macOS release
issue, old sustained RecorderBackpressure incident, physical webcam, Firefox,
representative accuracy, and target-hardware capacity remain open. Full make
check is not claimed green.
