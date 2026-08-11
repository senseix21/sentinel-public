# Next Actions — 2026-08-10 23:12 +06

1. Select the RTSP session adapter from maintained options and document its
   blocking, cancellation, timestamp, codec-capability, and reconnect behavior.
2. Implement an explicit camera connection lifecycle and legal transitions
   independently of the chosen RTSP library.
3. Spike the H.264 fragmented-MP4 sink with representative access units and
   connect drain/cancellation to the segment lifecycle.
4. Implement symlink-safe partial-file creation and atomic finalization for the
   target OS/filesystem policy.
5. Continue representative camera and deployment qualification when external
   inputs are available.
