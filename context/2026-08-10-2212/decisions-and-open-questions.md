# Decisions and Open Questions — 2026-08-10 22:12 +06

## Decisions

- Preserve recorder backlog on saturation and surface new-item rejection.
- Preserve live freshness by evicting that queue's oldest retained media.
- Keep consumer queues independent and publication nonblocking.
- Allow subscribers to drain retained items after producer closure.
- Release and count retained media when a consumer closes.
- Keep async/runtime concerns outside the first deterministic contract.

## Open questions

- Measured item and byte capacities for representative camera workloads.
- Whether the production queue needs a simultaneous byte and item limit.
- Recorder saturation escalation: pause, camera reconnect, degraded operation,
  or terminal pipeline failure.
- Cancellation-safe async notification mechanism and task ownership.
- How configuration updates apply to queue-capacity changes.
