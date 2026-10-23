# Next actions

1. Review the focused recovery changes on fix/rtsp-consumer-recovery. Local RTSP
   recovery regressions and three real two-viewer replays now pass. Rust server/
   detection teardown is prompt; whole-stack shutdown still varied up to 26
   seconds while waiting on LiveKit. No production claim follows.
2. Investigate the two immutable recording-metadata conflicts after prolonged
   source outages in target/verification-2026-10-23/stack/logs/server.log.
   Trace recorder metadata versus reconciliation evidence; preserve the files
   and database until the cause is known.
3. Investigate the older sustained RecorderBackpressure incident independently.
4. Triage dependency advisories, install/run cargo-audit and the security fixture;
   fix plain stripped macOS SQLx release rather than silently relying on the
   explicitly labeled CARGO_PROFILE_RELEASE_STRIP=none workaround.
5. Obtain physical webcam/Firefox, representative accuracy, target filesystem
   and 10/20-camera qualification evidence. Real LiveKit-server restart remains
   a different gate from the camera-source outage qualified here.

Use the README recovery section to reproduce the automated fixture. It replaces
and then stops the fixture publisher; restart your publisher for continued use.
