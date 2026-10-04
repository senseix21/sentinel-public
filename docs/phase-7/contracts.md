# Phase 7 Contracts — Persistent Configuration

Companion to `docs/adr/008-persistent-configuration.md`. The ADR decides; this
file specifies. Where the two disagree, the ADR wins and this file is wrong.

Written 2026-08-14 against the tree at `c71f952`.

## 1. Classification (P7.1)

Every setting is classified along four axes. Classification is exhaustive by
construction: `SettingId` is an enum, and each axis is a `const fn` with an
exhaustive `match`. Adding a variant without classifying it does not compile.

### Axes

**Class** — who may change it.

| Class | Meaning |
|---|---|
| `Bootstrap` | Resolvable before the settings store exists. Never editable through the API. Changed by editing the file or environment and restarting. |
| `Operational` | Editable through the API, persisted, audited. |

Bootstrap is not a synonym for "restart-applied". It means the value is needed
in order to reach the store, or it governs the mechanism by which settings are
changed. A setting that can lock out the interface used to change it belongs
here — hence `api.mutating_surface_enabled` is `Bootstrap` even though changing
it at runtime is technically feasible.

**Boundary** — when a change becomes effective.

| Boundary | Meaning |
|---|---|
| `Immediate` | Read at each use. Effective on the next use, with no coordination. |
| `NextSegment` | Effective when the recorder rolls to a new segment. Never mid-segment. |
| `NextReconnect` | Effective when the camera session is next established. Never mid-session. |
| `PipelineRestart` | Requires tearing down and rebuilding that camera's pipeline. Operator-triggered; never automatic, because it interrupts recording. |
| `ProcessRestart` | Requires restarting the server process. |

A change whose boundary is `PipelineRestart` is persisted and reported, and the
operator decides when to trigger the restart. Applying it silently would drop
recording to satisfy a configuration change, inverting the confirmed subsystem
priority.

**Secrecy** — `Public` or `Secret`. `Secret` values are `Secret<T>`, never
serialized in reads, never logged, and appear in audit rows only as a salted
fingerprint.

**Scope** — `Global` or `PerCamera(CameraId)`.

### 1.1 Global bootstrap settings

| Key | Type | Boundary | Secrecy | Source today |
|---|---|---|---|---|
| `server.health_bind` | `SocketAddr` | `ProcessRestart` | Public | `SURVEILLANCE_HEALTH_BIND` |
| `storage.recording_root` | `PathBuf` | `ProcessRestart` | Public | `SURVEILLANCE_RECORDING_ROOT` |
| `database.url` | `Secret<String>` | `ProcessRestart` | **Secret** | `SURVEILLANCE_DATABASE_URL` |
| `database.max_connections` | `u32` | `ProcessRestart` | Public | `SURVEILLANCE_DATABASE_MAX_CONNECTIONS` |
| `database.acquire_timeout` | `Duration` | `ProcessRestart` | Public | `SURVEILLANCE_DATABASE_ACQUIRE_TIMEOUT_SECONDS` |
| `supervision.maximum_cameras` | `NonZeroUsize` | `ProcessRestart` | Public | `SURVEILLANCE_MAXIMUM_CAMERAS` |
| `supervision.command_queue_items` | `NonZeroUsize` | `ProcessRestart` | Public | `SURVEILLANCE_COMMAND_QUEUE_ITEMS` |
| `api.mutating_surface_enabled` | `bool` | `ProcessRestart` | Public | **new**, default `false` |
| `audit.fingerprint_salt` | `Secret<String>` | `ProcessRestart` | **Secret** | **new** |
| `config.cache_path` | `PathBuf` | `ProcessRestart` | Public | **new** |
| `config.stale_retry_interval` | `Duration` | `ProcessRestart` | Public | **new**, default 5 s |
| `config.stale_retry_budget` | `Duration` | `ProcessRestart` | Public | **new**, default 300 s |

`storage.recording_root` is bootstrap because moving it orphans every indexed
recording. Relocating storage is a migration, not a settings change.

`config.stale_retry_budget` defaults to 300 s deliberately: ADR 008 requires the
retry to outlast typical service start ordering, and a local PostgreSQL that is
slow to accept connections is the dominant cause of `Stale`.

### 1.2 Global operational settings

| Key | Type | Boundary | Secrecy | Source today |
|---|---|---|---|---|
| `supervision.command_timeout` | `Duration` | `Immediate` | Public | `SURVEILLANCE_COMMAND_TIMEOUT_SECONDS` |
| `api.max_request_bytes` | `NonZeroUsize` | `Immediate` | Public | hardcoded `health_http.rs:31` |
| `api.io_timeout` | `Duration` | `Immediate` | Public | hardcoded `health_http.rs:32` |
| `retention.max_age` | `Duration` | `Immediate` | Public | `SURVEILLANCE_RETENTION_DAYS` |
| `retention.protected_recent` | `Duration` | `Immediate` | Public | `SURVEILLANCE_RETENTION_PROTECTED_HOURS` |
| `retention.batch_files` | `NonZeroUsize` | `Immediate` | Public | `SURVEILLANCE_RETENTION_BATCH_FILES` |
| `retention.batch_bytes` | `NonZeroU64` | `Immediate` | Public | `SURVEILLANCE_RETENTION_BATCH_BYTES` |
| `retention.interval` | `Duration` | `Immediate` | Public | `SURVEILLANCE_RETENTION_INTERVAL_SECONDS` |
| `storage.watermark_warning` | `u8` | `Immediate` | Public | hardcoded `runtime_config.rs:94` |
| `storage.watermark_critical` | `u8` | `Immediate` | Public | hardcoded `runtime_config.rs:94` |
| `storage.watermark_emergency` | `u8` | `Immediate` | Public | hardcoded `runtime_config.rs:94` |
| `storage.watermark_hysteresis` | `u8` | `Immediate` | Public | hardcoded `runtime_config.rs:94` |
| `reconciliation.queue_items` | `NonZeroUsize` | `ProcessRestart` | Public | hardcoded `main.rs:78` |
| `reconciliation.interval` | `Duration` | `Immediate` | Public | hardcoded `main.rs:79` |
| `reconciliation.backoff` | `Duration` | `Immediate` | Public | hardcoded `main.rs:80` |
| `metadata.queue_items` | `NonZeroUsize` | `ProcessRestart` | Public | hardcoded `main.rs:106` |
| `metadata.flush_interval` | `Duration` | `Immediate` | Public | hardcoded `main.rs:107` |
| `metadata.retry_backoff` | `Duration` | `Immediate` | Public | hardcoded `main.rs:108` |
| `playback.ffmpeg` | `PathBuf` | `Immediate` | Public | `SURVEILLANCE_FFMPEG` |
| `playback.probe_timeout` | `Duration` | `Immediate` | Public | hardcoded `main.rs:117` |
| `livekit.viewer_token_ttl` | `Duration` | `Immediate` | Public | hardcoded `main.rs:56` |

The four `storage.watermark_*` keys are individually editable but jointly
validated: strictly increasing, below 100, with hysteresis smaller than the first
transition gap. A single-key update that breaks the joint invariant is rejected
whole. This is the clearest case for P7.4's transactional application, and the
one the failure matrix exercises.

The queue-capacity settings are `ProcessRestart` because the queues are
constructed once at startup with a fixed capacity. Making them `Immediate` would
require a resizable bounded queue, which is a media-plane change with its own
overload semantics, not a configuration feature.

### 1.3 Per-camera operational settings

Scoped by `CameraId`. Metric labels use the setting key only, never the value;
camera cardinality is already bounded by `supervision.maximum_cameras`.

| Key | Type | Boundary | Secrecy |
|---|---|---|---|
| `camera.rtsp.endpoint` | `RtspEndpoint` | `NextReconnect` | Public |
| `camera.rtsp.username` | `Secret<String>` | `NextReconnect` | **Secret** |
| `camera.rtsp.password` | `Secret<String>` | `NextReconnect` | **Secret** |
| `camera.rtsp.connect_timeout` | `Duration` | `NextReconnect` | Public |
| `camera.rtsp.max_access_unit_bytes` | `usize` | `PipelineRestart` | Public |
| `camera.rtsp.max_nals_per_access_unit` | `usize` | `PipelineRestart` | Public |
| `camera.reconnect.initial` | `Duration` | `NextReconnect` | Public |
| `camera.reconnect.maximum` | `Duration` | `NextReconnect` | Public |
| `camera.reconnect.jitter_basis_points` | `u16` | `NextReconnect` | Public |
| `camera.reconnect.teardown_timeout` | `Duration` | `NextReconnect` | Public |
| `camera.reconnect.stable_session_reset` | `Duration` | `NextReconnect` | Public |
| `camera.video.width` | `u32` | `PipelineRestart` | Public |
| `camera.video.height` | `u32` | `PipelineRestart` | Public |
| `camera.queue.recorder_items` | `usize` | `PipelineRestart` | Public |
| `camera.queue.live_items` | `usize` | `PipelineRestart` | Public |
| `camera.queue.lifecycle_event_items` | `usize` | `PipelineRestart` | Public |
| `camera.recording.segment_duration` | `Duration` | `NextSegment` | Public |
| `camera.recording.fragment_duration` | `Duration` | `NextSegment` | Public |
| `camera.recording.max_segment_bytes` | `usize` | `NextSegment` | Public |
| `camera.recording.max_fragment_bytes` | `usize` | `NextSegment` | Public |
| `camera.recording.max_segment_samples` | `usize` | `NextSegment` | Public |
| `camera.recording.clock_drift_tolerance` | `Duration` | `Immediate` | Public |
| `camera.recording.ffprobe` | `PathBuf` | `Immediate` | Public |
| `camera.recording.reconciliation_scan_entries` | `usize` | `Immediate` | Public |
| `camera.memory.max_pipeline_bytes` | `usize` | `PipelineRestart` | Public |
| `camera.detection.enabled` | `bool` | `PipelineRestart` | Public |
| `camera.detection.queue_items` | `usize` | `PipelineRestart` | Public |
| `camera.detection.sample_fps` | `u16` | `PipelineRestart` | Public |
| `camera.detection.max_gap` | `Duration` | `PipelineRestart` | Public |
| `camera.detection.model_path` | `PathBuf` | `PipelineRestart` | Public |
| `camera.detection.model_sha256` | `String` | `PipelineRestart` | Public |
| `camera.detection.input_width` | `u32` | `PipelineRestart` | Public |
| `camera.detection.input_height` | `u32` | `PipelineRestart` | Public |
| `camera.detection.execution_providers` | `String` | `PipelineRestart` | Public |
| `camera.detection.intra_op_threads` | `u16` | `PipelineRestart` | Public |
| `camera.detection.inter_op_threads` | `u16` | `PipelineRestart` | Public |
| `camera.detection.score_threshold_percent` | `u8` | `PipelineRestart` | Public |
| `camera.detection.nms_iou_percent` | `u8` | `PipelineRestart` | Public |
| `camera.detection.motion_threshold_percent` | `u8` | `PipelineRestart` | Public |
| `camera.detection.gate_cooldown` | `Duration` | `PipelineRestart` | Public |
| `camera.detection.gate_heartbeat` | `Duration` | `PipelineRestart` | Public |
| `camera.detection.hwaccel` | `String` | `PipelineRestart` | Public |
| `camera.detection.decoded_frame_max_bytes` | `usize` | `PipelineRestart` | Public |
| `camera.detection.decoder` | `PathBuf` | `PipelineRestart` | Public |
| `camera.detection.decode_timeout` | `Duration` | `PipelineRestart` | Public |
| `camera.livekit.endpoint` | `String` | `PipelineRestart` | Public |
| `camera.livekit.api_key` | `Secret<String>` | `PipelineRestart` | **Secret** |
| `camera.livekit.api_secret` | `Secret<String>` | `PipelineRestart` | **Secret** |
| `camera.livekit.deployment_namespace` | `String` | `PipelineRestart` | Public |
| `camera.livekit.queue_items` | `usize` | `PipelineRestart` | Public |
| `camera.livekit.max_lag` | `Duration` | `PipelineRestart` | Public |
| `camera.livekit.max_access_unit_bytes` | `usize` | `PipelineRestart` | Public |
| `camera.livekit.shutdown_timeout` | `Duration` | `PipelineRestart` | Public |
| `camera.livekit.reconnect_initial` | `Duration` | `PipelineRestart` | Public |
| `camera.livekit.reconnect_maximum` | `Duration` | `PipelineRestart` | Public |
| `camera.livekit.reconnect_stable_reset` | `Duration` | `PipelineRestart` | Public |
| `camera.livekit.decoder` | `PathBuf` | `PipelineRestart` | Public |
| `camera.livekit.max_publish_height` | `u32` | `PipelineRestart` | Public |
| `camera.livekit.viewer_idle_grace` | `Duration` | `PipelineRestart` | Public |

Amended 2026-08-16: `camera.livekit.cli_path`, `camera.livekit.cli_version` and
`camera.livekit.ipc_write_timeout` were removed with the child-process publisher
they described (ADR 011). The settings cache treats a persisted key it cannot
classify as corruption, so the rows are deleted — and audited — by migration
`0005_retire_livekit_cli_settings.sql` rather than left behind.

Amended 2026-08-17: `camera.livekit.decoder` and `camera.livekit.max_publish_height`
were added with the transcoding publisher (ADR 012). Both are additions, so no
migration is needed — an absent key falls back to its versioned default, and the
cache only rejects a key it cannot classify.
`max_publish_height` is the one knob that trades published resolution for CPU;
`0` means publish the camera's own height.

Amended 2026-08-19: `camera.livekit.viewer_idle_grace` keeps the existing
published track and decoder warm for a bounded interval after the final viewer
leaves. The default is five seconds and zero restores immediate teardown. It
absorbs short browser navigation gaps without adding a queue, decoder, or
upstream camera connection.

`camera_id` is not in the table. It is the scope key, not a setting; changing it
would create a different camera, which is a lifecycle operation the existing
`/api/cameras` surface already covers.

The detection settings are `PipelineRestart` because ADR 003 already decided
they "apply on pipeline start/restart" and "never mutate an active decoder or
queue in place". Phase 7 does not reopen that.

### 1.4 Coupled validation groups

Several settings cannot be validated independently. A change touching any member
of a group re-validates the whole group against current effective values.

| Group | Members | Invariant |
|---|---|---|
| Storage watermarks | the four `storage.watermark_*` keys | strictly increasing, `< 100`, hysteresis below the first gap |
| Retention | `retention.max_age`, `retention.protected_recent` | protected window strictly shorter than max age |
| Camera memory budget | all `camera.queue.*`, `camera.detection.*`, `camera.video.*`, `camera.rtsp.max_access_unit_bytes`, `camera.recording.max_fragment_bytes`, `camera.memory.max_pipeline_bytes` | computed bound `<=` budget, per the existing `OneCameraConfig::validate` arithmetic |
| Decoded frame budget | `camera.detection.input_width`, `camera.detection.input_height`, `camera.detection.decoded_frame_max_bytes` | `decoded_frame_max_bytes >= input_width * input_height * 3` |
| Reconnect policy | `camera.reconnect.initial`, `camera.reconnect.maximum` | initial `<=` maximum |

The camera memory group is why per-field validation is insufficient: raising
`camera.queue.recorder_items` alone can breach `camera.memory.max_pipeline_bytes`
even though the field itself is a legal `usize`. `OneCameraConfig::validate`
already computes this; P7.4 routes group changes through it rather than
duplicating the arithmetic.

### 1.5 Not settings

`api_concurrent_requests = 1` appears in `config/default.toml` but is not
configurable and is not enforced by a limiter. The HTTP accept loop in
`apps/server/src/health_http.rs` handles connections serially, so the value
describes current behavior accurately but is not a knob. P7.8 converts it to a
comment documenting the invariant rather than promoting it to a setting, because
making it a setting would imply a concurrency mechanism that does not exist.

## 2. Types

### 2.1 `Secret<T>`

```rust
pub struct Secret<T>(T);

impl<T> Secret<T> {
    pub const fn new(value: T) -> Self;
    pub fn expose(&self) -> &T;          // named to be greppable
    pub fn into_inner(self) -> T;
}
```

Contract:

- `Debug` and `Display` render `Secret(***)`. Never the value.
- `Serialize` emits `"***"`. There is no `Deserialize` that round-trips through
  a public field; secrets enter through explicit constructors only.
- No `Clone` bound leak: `Clone` only where `T: Clone`.
- `PartialEq` is constant-time-independent and intentionally **not** derived;
  comparison happens on fingerprints, not values.
- The accessor is `expose`, not `get` or `value`, so that
  `grep -rn "\.expose()"` enumerates every place a secret is read. That grep is
  a review artifact, and its result is expected to stay small.

`Secret<T>` is introduced in P7.1, before any persisted or API path exists, per
the handoff. Retrofitting redaction after those paths are written means auditing
every one of them.

### 2.2 Fingerprints

```rust
pub struct Fingerprint([u8; 32]);

impl Fingerprint {
    pub fn of(salt: &Secret<String>, value: &[u8]) -> Self;
    pub fn to_hex(&self) -> String;
}
```

SHA-256 over `salt || 0x00 || value`. The separator byte prevents the
concatenation ambiguity where a different `(salt, value)` split produces the same
input. Salted per deployment so fingerprints are not comparable across units and
not subject to precomputation against a small candidate space.

### 2.3 Origin and freshness

```rust
pub enum Origin { Default, File, Persisted, Environment }

pub enum ConfigurationFreshness {
    Fresh { loaded_at: SystemTime },
    Stale { since: SystemTime, cache: CacheState, last_success: Option<SystemTime> },
}

pub enum CacheState { Valid, Corrupt, Absent, Unwritable }
```

`Origin` is tracked per setting, not per layer, because the answer an operator
needs is "why does this setting have this value", one setting at a time.

### 2.4 Change reporting

```rust
pub enum ChangeOutcome {
    Applied,                                   // Immediate boundary, in force now
    PendingBoundary { boundary: ApplicationBoundary },
    PendingRestart,
    PersistedButShadowed { winning_origin: Origin },
    ApplyFailed { category: ApplyFailureCategory },
    Unchanged,                                 // new value equals effective value
}

pub struct ChangeReport {
    pub generation: u64,
    pub outcomes: Vec<(SettingId, ChangeOutcome)>,
}
```

`PersistedButShadowed` is the honest report for a change to a setting the
environment pins: validated, persisted, audited, not effective. `Unchanged` is
distinguished from `Applied` so that an audit trail does not accumulate rows
asserting change where none occurred.

### 2.5 Update rejection

```rust
pub enum UpdateRejection {
    UnknownKey { key: String },
    NotOperational { id: SettingId },          // bootstrap setting via API
    Malformed { id: SettingId, reason: &'static str },
    Invalid { id: SettingId, source: ValidationError },
    GroupInvariant { group: ValidationGroup, source: ValidationError },
    Unauthorized,
    SurfaceDisabled,
    ShuttingDown,
}
```

Every variant names the offending setting or group. A rejection that says only
"invalid configuration" fails the P7.4 contract.

An update is all-or-nothing: the whole batch validates before anything persists.
There is no coercion — an out-of-range value is rejected, never clamped.

## 3. Persistence (P7.5)

### 3.1 Schema

`migrations/0002_settings.sql`, following the CHECK-constraint discipline of
`0001_recordings.sql`.

```sql
CREATE TABLE settings (
    setting_key   TEXT NOT NULL,
    camera_id     TEXT NULL REFERENCES cameras(camera_id),
    value         TEXT NOT NULL,
    updated_at    TIMESTAMPTZ NOT NULL,
    generation    BIGINT NOT NULL,
    PRIMARY KEY (setting_key, camera_id)
);
```

Postgres treats `NULL` as distinct in unique indexes, so global settings — where
`camera_id IS NULL` — need a partial unique index in addition to the primary key
to actually prevent duplicates. Both are created; the primary key alone is not
sufficient and assuming otherwise is the kind of thing that only shows up under
concurrency.

```sql
CREATE TABLE settings_audit (
    audit_id      BIGSERIAL PRIMARY KEY,
    occurred_at   TIMESTAMPTZ NOT NULL,
    actor         TEXT NOT NULL,
    action        TEXT NOT NULL,
    setting_key   TEXT NOT NULL,
    camera_id     TEXT NULL,
    origin        TEXT NOT NULL,
    outcome       TEXT NOT NULL,
    old_value     TEXT NULL,
    new_value     TEXT NULL,
    old_fingerprint TEXT NULL,
    new_fingerprint TEXT NULL,
    CHECK ((old_value IS NULL) OR (old_fingerprint IS NULL)),
    CHECK ((new_value IS NULL) OR (new_fingerprint IS NULL))
);
```

The two CHECKs encode the secrecy rule in the schema: a row carries either a
plaintext value or a fingerprint for a given side, never both. A bug that wrote a
secret into `new_value` while also fingerprinting it is rejected by the database
rather than discovered in an export.

`actor` is `"loopback"` in Phase 7. Phase 8 replaces the content, not the shape.

### 3.2 Repository

`crates/database/src/settings.rs`, following `recordings.rs`:

```rust
pub struct PostgresSettingsRepository { pool: PgPool }

impl PostgresSettingsRepository {
    pub fn connect_lazy(config: &DatabaseConfig) -> Self;
    pub fn from_pool(pool: PgPool) -> Self;
    pub async fn migrate(&self) -> Result<(), RepositoryError>;
    pub async fn load_all(&self) -> Result<PersistedSettings, RepositoryError>;
    pub async fn apply(&self, batch: &SettingsBatch, audit: &[AuditRecord])
        -> Result<u64, RepositoryError>;   // returns the new generation
}
```

`apply` opens one `pool.begin()`, writes every settings row and every audit row,
and commits once. Per ADR 008 Decision 3, an audit write failure aborts the whole
transaction. The returned generation is monotonic and identifies the
configuration version in metrics and in the bus event.

Errors reuse the existing `RepositoryError` and its `retryable()` classification
rather than introducing a parallel error taxonomy.

### 3.3 Cache

Written to `config.cache_path` by create-temp-then-`rename` so a torn write is
impossible. Contents are the resolved operational settings plus the generation
and a checksum. On load, a checksum mismatch is `CacheState::Corrupt`.

A corrupt cache file is **preserved, not overwritten**, until a successful store
load replaces it — it is evidence about how the unit got into that state.

Cache write failure does not fail the load that produced it. The settings are
already valid and effective; an unwritable cache degrades the next startup, not
this one, and is surfaced as `CacheState::Unwritable` with its own metric.

## 4. Event (P7.7)

A new `EventPayload` variant, published after the P7.5 transaction commits:

```rust
ConfigurationChanged {
    generation: u64,
    changed: Vec<ChangedSetting>,   // setting key + outcome; never values
}
```

`DeliveryClass::LatestState`, joining `CameraStateChanged`,
`DetectorStateChanged`, and `StoragePressureChanged`. Configuration is state, not
a stream of independently meaningful facts: a consumer that missed generation 7
and receives generation 8 is correct, because 8 supersedes 7. `CriticalLifecycle`
would be wrong — it would imply the bus is where the change is remembered, and
ADR 002 says it provides no durability. The audit table is the record.

`state_key()` returns `(None, "configuration")` so the bus's latest-state
collapsing applies.

**Values never appear on the bus.** A consumer that needs the new value reads it
back through the settings API. This keeps secrets off the bus structurally rather
than by remembering to redact at each publication site.

Publication failure or overflow does not roll back the committed change and does
not fail the API response. The response reports the audit outcome and the
notification outcome separately, which is also how the two metrics are counted.

## 5. API (P7.6)

Extends the hand-rolled `route()` in `apps/server/src/health_http.rs`. No
framework is introduced.

| Method | Path | Auth | Notes |
|---|---|---|---|
| `GET` | `/api/settings` | none | Effective values, origin, boundary, freshness. Secrets rendered `"***"`. |
| `GET` | `/api/settings/{key}` | none | Stored and effective values reported separately. |
| `PUT` | `/api/settings` | `authorize` | Batch update. All-or-nothing. |
| `GET` | `/api/cameras/{id}/settings` | none | Camera-scoped read. |
| `PUT` | `/api/cameras/{id}/settings` | `authorize` | Camera-scoped batch update. |

Reads are unauthenticated in Phase 7 because the listener is loopback-only and
observing effective configuration carries no write risk; Phase 8 revisits this
with an identity model. Writes always route through `authorize`.

When `api.mutating_surface_enabled` is false, the two `PUT` routes return `404`,
matching an unknown path exactly. A disabled surface does not advertise itself.

### 5.1 The authorization token

```rust
pub struct Authorized(());               // private field, no public constructor

pub fn authorize(request: &RequestContext, action: Action)
    -> Result<Authorized, Denied>;

// every mutating handler:
fn apply_settings_update(auth: Authorized, /* ... */) -> Response;
```

`Authorized` is constructible only inside the module that defines `authorize`.
A handler that skips the check cannot obtain one and does not compile. This is
what makes failure-matrix item 12 — "every mutating endpoint reaches the
chokepoint, proven by test rather than by inspection" — provable: the test
asserts the runtime behavior, and the type system asserts the absence of a
bypass.

## 6. Observability

Bounded cardinality: setting key and outcome category only, never values.

All series are prefixed `surveillance_` to match the existing exposition.
Implemented in `render_configuration_metrics`, asserted by
`configuration_metrics_are_exposed_with_bounded_cardinality`.

| Metric | Type | Labels |
|---|---|---|
| `surveillance_configuration_generation` | gauge | — |
| `surveillance_configuration_freshness` | gauge | — (0 fresh, 1 stale) |
| `surveillance_configuration_store_available` | gauge | — |
| `surveillance_configuration_mutating_surface_enabled` | gauge | — |
| `surveillance_configuration_unreadable_rows` | gauge | — |
| `surveillance_configuration_last_success_age_seconds` | gauge | — |
| `surveillance_configuration_cache_state` | gauge | `state` |
| `surveillance_configuration_origin` | gauge | `origin` |
| `surveillance_configuration_pending` | gauge | `boundary` |
| `surveillance_configuration_applied_total` | counter | — |
| `surveillance_configuration_rejected_total` | counter | — |
| `surveillance_configuration_audit_write_total` | counter | — |
| `surveillance_configuration_audit_write_failed_total` | counter | — |
| `surveillance_configuration_notify_published_total` | counter | — |
| `surveillance_configuration_notify_dropped_total` | counter | — |
| `surveillance_configuration_cache_write_total` | counter | — |
| `surveillance_configuration_cache_write_failed_total` | counter | — |

Two departures from the first draft of this table, both deliberate:

- Outcome is carried by the **series name** rather than an `outcome` label, so a
  dashboard cannot accidentally sum a success and a failure into one number.
- `configuration_rejected_total` has no `reason` label. Breaking rejections down
  by reason needs per-reason counters that do not exist yet; a label that always
  reads `unknown` would be worse than its absence.

`surveillance_configuration_last_success_age_seconds` is **absent**, not zero,
when no load has ever succeeded. Zero would read as "just loaded".

Audit-write and notification outcomes are counted separately and deliberately.
Collapsing them would hide the case ADR 008 cares about: a change that is durably
recorded but whose notification was dropped, which is correct behavior and must
not read as a failure.

`/ready` reports `Stale` as degraded readiness, never as failed liveness.

## 7. What this does not specify

- **Authentication and identity.** Phase 8. `authorize` grants on loopback with a
  placeholder actor until then.
- **Production secret storage and rotation.** Still open in ADR 003. `Secret<T>`
  is the in-process representation only.
- **ROI, rules, analytics, tracking** (P7.9–P7.12). Blocked on product gates:
  coordinate semantics, rule conditions, and the confirmed meaning of a heatmap.
  `crates/database/src/rois.rs` stays a stub, and `ROADMAP.md` records these as
  `[!]`.
- **Optimistic concurrency by row version.** Deferred in ADR 008 with a recorded
  reversal criterion. Phase 7 relies on the transaction boundary.
