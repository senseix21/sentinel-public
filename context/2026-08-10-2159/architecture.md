# Architecture Context — 2026-08-10 21:59 +06

The RTSP depayloader owns RTP fragmentation/aggregation reconstruction and supplies validated H.264 NAL units with a timestamp and explicit end-of-access-unit marker.

The media boundary groups all slices for one presentation unit. `H264AccessUnit` always contains picture data and reports whether it is an IDR keyframe and whether both SPS and PPS are inline.

Discontinuities are explicit resets, not silently accepted timestamp changes. Immutable NAL payloads use `Arc<[u8]>` for ownership transfer without claiming measured zero-copy performance.

The LiveKit adapter remains absent because official Rust encoded ingest is unreleased. The media contract does not depend on its draft API.
