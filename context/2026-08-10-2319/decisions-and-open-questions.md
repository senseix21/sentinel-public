# Decisions and Open Questions — 2026-08-10 23:19 +06

## Decisions

- Encode legal lifecycle transitions through state-specific owning types.
- Make connected-without-session unrepresentable.
- Advance connection epoch only after successful session establishment.
- Separate retryable transport failures from terminal configuration/media
  failures.
- Use Retina 0.4.19 provisionally for the H.264 RTSP/TCP spike and preserve a
  GStreamer reversal path.

## Open questions

- Retina error-to-domain classification coverage and diagnostic redaction.
- Exact Retina frame/access-unit and timestamp mapping.
- Session teardown completion under cancellation and timeout.
- Representative camera quirks not covered by synthetic MediaMTX.
- Backoff jitter implementation and maximum-attempt/operator policy.
