# Architecture Context — 2026-08-11 05:37 +06

`RtspSourceConfig` keeps a validated credential-free `RtspEndpoint`, separately
owned redacted credentials, a nonzero total handshake timeout, and explicit
access-unit allocation limits. Embedded URL credentials, queries, and fragments
are rejected to reduce accidental secret exposure.

`connect_h264` performs DESCRIBE, selects one H.264 video stream, requests
interleaved TCP, SETUP, and PLAY. The returned `RetinaH264Session` is the singular
upstream owner. It emits complete `H264AccessUnit` values or explicit packet-loss
discontinuities; it does not own reconnect, distribution, or recording policy.

Retina emits four-byte length-prefixed H.264 access units. The adapter validates
the byte/NAL bounds, maps the RTP-derived timestamp without inventing wall time,
and feeds the established media-plane assembler. Rust 1.88 is now the declared
MSRV due to Retina 0.4.19.

