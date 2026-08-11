# Decisions and Open Questions — 2026-08-10 22:58 +06

## Decisions

- Place the muxer/test substitution boundary at complete H.264 access units.
- Require a sink to synchronously accept into bounded owned state or reject.
- Return ownership of a rejected unit to the supervisor.
- Drain queued units before reporting producer closure.

## Open questions

- Retry versus terminal behavior after sink rejection.
- Async wake-up, cancellation, and task join ownership.
- Concrete fragmented-MP4 sink and its internal buffering bound.
- Segment rotation signaling at a keyframe boundary.
