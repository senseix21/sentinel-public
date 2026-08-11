# Next Actions — 2026-08-10 21:35 +06

1. Build a deterministic 15 FPS H.264 publisher-throughput harness that independently measures produced frames, consumed frames, wall time, queue depth, and drops.
2. Compare the CLI/socket adapter with the narrowest viable native packet-publishing API; do not add it to production crates until its ownership and cancellation contract is understood.
3. After real-time throughput passes, test actual browser rendering and end-to-end latency using the webcam source.
4. Retain representative ONVIF Profile T H.264 camera qualification as a Phase 0 exit gate.

Do not proceed to Phase 1 on the basis of track publication alone.
