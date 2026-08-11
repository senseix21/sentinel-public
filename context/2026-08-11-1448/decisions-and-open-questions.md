# Decisions and Open Questions — 2026-08-11 14:48 +06

## Decisions

- Use `watch` semantics for latest camera lifecycle health.
- Queue health observers are read-only and non-owning.
- A health reader does not keep a failed supervisor alive.

## Open

- HTTP liveness/readiness representation and authentication boundary.
- Metrics text/export format.
- Runnable server configuration and forced-process harness.
