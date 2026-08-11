# Decisions and Open Questions — 2026-08-10 21:47 +06

## Decisions

- H.264 access units, not NAL units, are the pacing/publication unit.
- Publisher readiness precedes activation of its media subscription.
- Single-slice H.264 is a fixture constraint, not a production camera requirement.
- Sustained webcam publication is established; browser rendering/latency is not.

## Open questions

- Which Rust LiveKit API accepts pre-encoded timestamped H.264 access units with explicit cancellation and feedback handling?
- How will the packet parser assemble multi-slice access units and propagate SPS/PPS and keyframe state?
- What browser, viewer-count, and latency targets define acceptance?
- How does the representative High Profile camera packetize access units?
