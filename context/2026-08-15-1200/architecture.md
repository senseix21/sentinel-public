# Architecture — 2026-08-15

Unchanged from 2026-08-14 in the media plane. Phase 7 adds a configuration plane
beside it, touching no packet or frame path.

## Configuration plane

```text
compiled defaults ─┐
config file ───────┤
persisted (PG) ────┼─► ResolvedSettings ─► assemble ─► OneCameraConfig / RetentionPolicy
environment ───────┘        │                              (existing validated types)
                            │
                            ├─► settings API (read)
                            └─► ChangeReport ─► EventBus (notification only)

last-known-good cache ◄── every successful load
```

Precedence is `defaults < file < persisted < environment`. An API change writes
the persisted layer, so an environment-pinned setting reports
`PersistedButShadowed` rather than a bare success.

## Ownership

| Component | Owns |
|---|---|
| `crates/config` | The setting table, values, layers, resolution, change planning, cache, and the store *contract* |
| `crates/database` | The `PostgreSQL` implementation of that contract, and the transaction |
| `apps/server/src/authorize.rs` | The single decision point for mutating requests |
| `apps/server/src/settings_service.rs` | Effective configuration for the process; sequencing of plan → commit → adopt → cache → notify |
| `apps/server/src/health_http.rs` | Translation only; no domain state |

Dependencies point inward: `database` depends on `config`, never the reverse.

## Boundaries that hold

- **No media on the configuration path**, and no configuration on the media
  path. The `ConfigurationChanged` payload carries setting keys and outcomes,
  never values — so a consumer needing a value reads it back through the API,
  and secrets cannot reach the bus structurally.
- **Notification is not the record.** Publication happens after commit and its
  failure is counted separately from the audit write. A dropped notification
  means a consumer is briefly behind.
- **Configuration freshness never blocks media preservation.** `Stale` degrades
  readiness, not liveness, and the server comes up without waiting for the store.
- **Queue capacities remain fixed at construction.** Resizing a bounded queue is
  a media-plane change with its own overload semantics, so those settings are
  restart-scoped rather than pretending to be live.

## What the configuration plane does not do

- No authentication. `authorize` grants on loopback.
- No secret persistence, and no per-camera credentials.
- No live re-reading except `api.max_request_bytes` and `api.io_timeout`. Every
  other operational setting takes effect at a pipeline or process restart, and
  says so in its change report.
