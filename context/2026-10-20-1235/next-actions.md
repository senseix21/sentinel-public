# Next actions

1. Reproduce the consumer stall after RTSP epoch change with the saved scene
   source. Trace decoder/publication reset, queue draining and cancellation.
   Both inference and live counters stall while recording resumes. Correct
   health reporting as part of that fix; add a focused regression.
2. Investigate the older sustained RecorderBackpressure incident separately.
3. Triage cargo-deny advisory findings by actual dependency/runtime/build usage;
   update dependencies with focused checks, not policy exceptions. Install/run
   cargo-audit and the complete security fixture.
4. Resolve plain macOS release SQLx LINKEDIT failure. Until then label
   CARGO_PROFILE_RELEASE_STRIP=none explicitly.
5. Obtain physical webcam/Firefox, accuracy, storage and 10/20-camera evidence.

README carries the exact startup/testing walkthrough and observed results.
Keep local evidence under target/verification-2026-10-20/ until these failures
are understood; it is ignored build output and will not survive cargo clean.
