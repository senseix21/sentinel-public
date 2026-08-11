# Decisions and Open Questions — 2026-08-11 05:37 +06

## Decisions

- Accept Retina 0.4.19 for the deterministic H.264 ingest slice, behind the
  private adapter boundary.
- Use RTSP interleaved TCP for the initial qualification profile.
- Treat 401/403 as terminal authentication failures, 404 as terminal invalid
  configuration, 415 as terminal unsupported media, and otherwise opaque Retina
  connection errors as retryable transport/protocol failures.
- Report packet loss as a discontinuity rather than publishing a potentially
  misleading access unit.
- Raise the workspace MSRV from the earlier unsupported 1.80 claim to Retina's
  actual 1.88 requirement.

## Open questions

- Representative camera interoperability, including vendor RTSP quirks.
- Supervisor cancellation/reconnect behavior and fresh connection epochs.
- Timeout/error classification where Retina exposes no status code.
- Production access-unit byte and NAL-count limits from measured 4 MP streams.
- H.265 adapter policy and representative H.265 timestamp/boundary behavior.
- Credential zeroization and production secret-provider ownership.

