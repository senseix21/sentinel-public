# Decisions and Open Questions — 2026-08-10 21:59 +06

## Decisions

- Do not adopt an unreleased LiveKit Rust SDK branch.
- Do not infer presentation boundaries from arbitrary Annex-B NAL ordering.
- Do not introduce a publisher trait until a production adapter and behaviorally equivalent test boundary exist.
- Keep H.264 access-unit assembly in `media`; RTP FU-A/STAP-A depayloading remains a camera-ingestion concern.

## Open questions

- Which released SDK/version will provide pre-encoded publication, and with what queue/cancellation contract?
- How will parameter-set cache lifetime interact with reconnect/caps epochs?
- Which representative camera profiles use multi-slice pictures or out-of-band parameter sets?
- What maximum access-unit size should bound ingress validation?
