# Architecture Context — 2026-08-10 19:40 +06

The single-ingest topology remains mandatory. One camera session produces encoded packets for bounded recorder and live-publication consumers; a LiveKit consumer must never open its own RTSP session.

For compatible H.264, the live path can adapt Annex-B/AVCC framing and packetize RTP without decoding pixels. Transcoding requires a documented codec/client boundary and isolated capacity.

The LiveKit CLI Unix-socket publisher is experiment infrastructure, not the production adapter. The production boundary must specify bounded buffering, freshness-oriented overflow, cancellation, restart/backoff, codec configuration/keyframes, and metrics.

Recording remains preservation-oriented fragmented MP4 with explicit partial/final lifecycle. Live publication remains latency-oriented and independently degradable.
