# Architecture — One-Camera Runtime Ownership

```text
OneCamera owner
  |-- camera supervisor (sole RTSP session and distributor producer)
  `-- segment recorder (sole recorder subscription and durable store)
```

Either child ending stops the pipeline; the sibling is never detached. The
distributor is dropped before recorder drain so closure is explicit.

Startup reconciliation is discovery, not repair. Partial files are never
completed by filename inference; durable candidates need probe and repository
work in Phase 4.
