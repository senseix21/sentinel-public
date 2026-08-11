# Architecture — Continuous Health

```text
camera supervisor --watch(latest snapshot)--> OneCameraHealth
media queues ------read-only observer-------> OneCameraHealth
```

Health is a queryable latest state, not a durable event stream. Lifecycle
history belongs in structured events/metrics. Queue observers share only the
existing bounded state and cannot receive or retain media.
