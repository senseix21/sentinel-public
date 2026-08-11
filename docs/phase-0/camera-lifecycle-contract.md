# Camera Lifecycle Contract

Date: 2026-08-10

## State and ownership

The camera lifecycle uses separate owning Rust types:

```text
DisconnectedCamera
        |
        v
ConnectingCamera --success + session--> ConnectedCamera<Session>
        |                                      |
        | retryable failure                    | session loss
        v                                      v
ReconnectingCamera --retry time--> ConnectingCamera

Connecting/Connected/Reconnecting --request--> StoppedCamera
Connecting --terminal failure---------------> StoppedCamera
```

Only `ConnectedCamera<S>` contains `S`, making a connected state without an
active upstream session unrepresentable. Leaving connected state consumes the
value and drops the session exactly once.

Legal transitions are methods on the originating state type. There is no broad
`transition(new_state)` API capable of inventing illegal combinations.

## Identity and time

- Each connection attempt receives a cumulative nonzero attempt number.
- Each successful session receives a cumulative nonzero connection epoch.
- A failed attempt does not advance the successful epoch.
- Source timestamps are comparable only within the resulting connection epoch,
  as defined by the timestamp contract.
- Connected UTC is observational metadata; it does not replace media time.

## Failure classification

Retryable failures are network unavailability, timeout, EOF, server
unavailability, and transport-protocol failure. Authentication rejection,
invalid configuration, and unsupported media are terminal and require operator
or configuration action. The future adapter must map errors explicitly; unknown
errors must not be retried forever by default.

Reconnect delay is a validated nonzero value. Backoff calculation and jitter are
separate policy work and are not embedded in lifecycle state.

## Observability

Every state can produce a credential-free `ConnectionSnapshot` containing:

- camera identity;
- explicit disconnected/connecting/connected/reconnecting/stopped state;
- attempt, epoch/connected time, retry cause/delay, or stop cause as applicable;
- saturating attempts, successful connections, reconnects, clean stops, and
  terminal-failure counters.

The snapshot is the future health/metrics boundary; it never exposes session or
credential objects.
