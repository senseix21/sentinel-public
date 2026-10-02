# Next Actions — 2026-09-05 11:16 +06

1. Select and ADR a deterministic external RTSP test peer, then rebuild auth,
   reconnect, and two-camera failure-isolation fixtures without making it a
   production dependency.
2. Add Linux V4L2 and Windows DirectShow operator smoke procedures and collect
   physical evidence before claiming support.
3. Decide whether webcam/source selection must be persisted and API-managed;
   if yes, add it as a transactional pipeline-replacement vertical slice.
4. Measure parser high-water mark, FFmpeg/server/LiveKit CPU and RSS, bitrate,
   queue drops, and live first-picture/latency under named workloads.
5. Run representative 10- and 20-camera qualification on confirmed target
   hardware before making scale claims.
