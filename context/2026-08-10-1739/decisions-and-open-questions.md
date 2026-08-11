# Decisions and Open Questions — 2026-08-10 17:39 +06

## Evidence-backed decisions

- Storage sizing is modeled from measured/planned bitrate, camera count, and retention; resolution/codec alone are not capacity inputs.
- The 259.2 TB figure is valid only for 200 continuously recorded 4 Mbps cameras over 30 days.
- Timestamp normalization and container choice are codec-sensitive and remain architectural work.
- The local Mac is a development/spike host, not qualification hardware.
- Synthetic files validate segment mechanics only; they do not validate RTSP cameras, LiveKit, or capacity.

## Open technical gates

- Representative H.264/H.265 RTSP streams, profiles, GOPs, PTS/DTS, reconnect, and caps-change behavior.
- Clean H.265 segmentation policy and container choice.
- Local MediaMTX or alternative RTSP fixture server installation/operation.
- GStreamer runtime qualification.
- LiveKit ingest and codec conversion behavior.
- Durable bounded metadata reconciliation during PostgreSQL outage.

## Open product gates

Erik must still confirm camera specifications, production hardware, audio/ONVIF/PTZ, detection model/hardware/freshness, heatmap semantics, playback accuracy, viewer concurrency/live latency, availability/data-loss targets, disk-full behavior, security/identity, and p90 objectives.

