# Decisions and Open Questions — 2026-08-10 19:31 +06

## Accepted decisions

- Fragmented MP4 is the H.264 MVP recording container.
- Active files are visibly partial and cannot be indexed/served as completed.
- Completion ordering is finalize/sync/validate/atomic rename, followed by completed metadata persistence.
- PostgreSQL/file divergence is handled through explicit lifecycle and reconciliation, not claimed cross-resource atomicity.
- Classic MP4 is rejected for continuous recording.
- Matroska remains a fallback subject to representative-stream/playback evidence.

## Remaining technical gates

- Real camera fragmented-MP4 stream copy and timestamp behavior.
- Production filesystem sync, machine restart, and power-loss qualification.
- Browser/time-range playback and fragment concatenation behavior.
- LiveKit H.264 publication.
- Rust ingestion recovery and bounded distribution semantics.

Open product/hardware/security decisions remain unchanged from the previous snapshot.

