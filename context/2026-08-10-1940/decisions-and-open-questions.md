# Decisions and Open Questions — 2026-08-10 19:40 +06

## Accepted decisions

- A compatible H.264 encoded packet source may feed recorder and LiveKit consumers without decode/re-encode.
- Bitstream framing conversion and RTP packetization are allowed media adaptation.
- The CLI/socket mechanism is not accepted as the production implementation.
- The Phase 0 LiveKit risk is reduced but remains open pending representative-camera and browser evidence.

## Remaining technical gates

- Representative ONVIF Profile T H.264 High Profile stream evidence.
- Browser rendering and end-to-end live-latency measurement.
- H.265 live-view policy and fallback/transcode behavior.
- Rust publisher API choice, bounded queue contract, restart behavior, and outage isolation.
- Production filesystem/power-loss qualification and Rust ingestion recovery.

Open product, hardware, playback, reliability, and security objectives remain unresolved in `docs/requirements.md` and `ROADMAP.md`.
