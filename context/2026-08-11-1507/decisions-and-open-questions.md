# Decisions and Open Questions — 2026-08-11 15:07 +06

## Decisions

- The actual server binary is part of the mandatory external gate.
- Do not mark forced-shutdown recovery complete using a fabricated empty marker
  file; the partial must contain real recoverable media evidence.
- Graceful qualification uses direct SIGINT to the compiled Rust process.

## Open

- Adapt muxer output to incrementally persist a fragmented-MP4 partial while
  preserving bounded memory and atomic durable publication.
- Define whether an interrupted valid fragment set is playable, repairable, or
  retained only as failure evidence.
- Configured-versus-negotiated dimension comparison and HTTP metric export.
