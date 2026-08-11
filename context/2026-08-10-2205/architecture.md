# Architecture Context — 2026-08-10 22:05 +06

Camera ingestion owns RFC 6184 payload parsing. Media owns validated NAL/access-unit representations. Dependency direction is `camera -> media`.

The MVP supports non-interleaved single NAL, STAP-A, and FU-A. Interleaved payload formats are rejected until camera evidence requires them.

FU state is singular per depayloader/SSRC stream, bounded, sequence-aware, and cleared on corruption, interruption, loss, or discontinuity. Only complete reconstructed NALs cross into media.
