# Architecture Context — 2026-08-10 23:19 +06

Camera lifecycle uses `DisconnectedCamera`, `ConnectingCamera`,
`ConnectedCamera<S>`, `ReconnectingCamera`, and `StoppedCamera`. Legal methods
consume the old state. Only connected state owns `S`, so leaving it drops the
upstream session once.

Attempts and successful connection epochs are independent monotonic identities.
Retryable and terminal failures are distinct enums. Credential-free snapshots
carry state-specific details and saturating counters for future health/metrics.

Retina is behind the future `camera::rtsp` boundary and must not leak into these
domain types.
