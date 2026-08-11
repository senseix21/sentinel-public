# Next Actions — 2026-08-10 21:59 +06

1. Define maximum NAL/access-unit size and count from representative 4 MP/4 Mbps camera evidence or a conservative reversible configuration.
2. Implement the narrow RTP H.264 depayloading contract for single NAL, STAP-A, and FU-A packets with marker/timestamp propagation and malformed-packet tests.
3. Feed depayloaded output into `H264AccessUnitAssembler` in an end-to-end deterministic test.
4. Then define the bounded media distributor and prove a slow live consumer cannot block the recorder path.
5. Continue representative-camera, browser-latency, hardware, and product-decision Phase 0 gates in parallel when inputs are available.
