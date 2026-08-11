# Architecture Context — 2026-08-10 17:39 +06

The protected topology remains one upstream RTSP session per camera feeding bounded encoded-media distribution. Recorder, detector, and LiveKit are media consumers; only structured facts enter the domain event bus.

Phase 0 evidence reinforces that packet timestamp semantics must be designed before the Rust packet API. H.265 encoding/Matroska segmentation exposed reordered/negative timestamps and non-exact nominal boundaries. The contract therefore needs explicit PTS, DTS, timebase, keyframe, discontinuity, and reconnect semantics rather than a single fabricated timestamp.

Filesystem-backed video plus PostgreSQL metadata remains proposed. Segment lifecycle and bounded reconciliation are part of the correctness boundary, not cleanup details.

No production dependencies or media contracts were added to the Rust scaffold in this increment.

