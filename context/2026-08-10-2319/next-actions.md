# Next Actions — 2026-08-10 23:19 +06

1. Add Retina only inside `camera::rtsp` and implement a validated H.264,
   interleaved-TCP connection spike against deterministic local MediaMTX.
2. Map adapter errors into typed retryable/terminal camera failures without
   exposing credentials.
3. Connect playing-session output to the existing access-unit/distributor seam
   and prove cancellation/session teardown.
4. Implement bounded exponential reconnect backoff with deterministic jitter
   injection and cancellation.
5. Continue the fragmented-MP4 sink and storage-finalization work after the
   ingest representation is confirmed.
