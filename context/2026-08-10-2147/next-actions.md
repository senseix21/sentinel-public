# Next Actions — 2026-08-10 21:47 +06

1. Investigate the narrowest supported Rust LiveKit encoded-track API and write the publisher contract before adding implementation dependencies.
2. Build tests for H.264 access-unit assembly covering SPS/PPS, IDR, multiple slices, malformed input, and timestamp discontinuity.
3. Add a bounded freshness-oriented publisher queue with observable depth, lag, and drops only after the access-unit contract is accepted.
4. Test browser rendering and glass-to-glass latency with the webcam.
5. Retain representative ONVIF Profile T High Profile qualification as a Phase 0 exit gate.
