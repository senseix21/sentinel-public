# Decisions and Open Questions — 2026-08-11 11:35 +06

## Decisions

- Classify RTSP 404 as retryable server unavailability because MediaMTX returns
  it while a configured publisher is temporarily absent.
- Provisionally use muxide 0.2.5 behind a reversible recorder adapter.
- Never open a second RTSP connection for recording or feed encoded video
  through a decoder/encoder.
- Bound muxer bytes, samples, and pending completed output.
- Publish a file only after file sync and atomic rename; probe only the durable
  path and retain stage-classified failures for reconciliation.

## Open questions

- Representative-camera DTS/B-frame behavior and muxide interoperability.
- Media timestamp to UTC mapping, including NTP jumps and reconnect epochs.
- Runtime ownership joining supervisor, recorder, storage, and metadata delivery.
- Reconciliation policy for partial files and durable-but-unindexed files.
- Production limits from measured 4 MP streams and disk performance.

