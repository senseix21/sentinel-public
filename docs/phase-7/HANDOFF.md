# Phase 7 Handoff — Persistent Configuration, Rules, ROI, and Analytics

Mission: make operational behavior safely configurable, and add derived business
behavior without coupling it to the producers of that behavior.

Roughly half of Phase 7 cannot be started. Items depending on confirmed product
semantics — ROI coordinates, heatmap meaning, tracking need — are blocked on
decision gates that remain open. This handoff separates what is buildable today
from what is not, and does not pretend the blocked half is merely unstarted.

## Read first

1. `AGENTS.md` — governing engineering policy. Its "Current Repository Reality"
   section was corrected on 2026-08-14 and is accurate as of that date.
2. `CLAUDE.md` — verified commands, real crate layout, working conventions.
3. `ROADMAP.md` Phase 7 — the 18 authoritative items this handoff expands.
4. `context/README.md` → the latest snapshot — current position and blockers.
5. `docs/adr/003-configuration.md` — the accepted configuration decision this
   phase extends.
6. `docs/phase-6/HANDOFF.md` — the format this document follows.

## Verified starting point

Verified on 2026-08-14, all commands rerun:

- `make check` passes: `cargo fmt --all --check`, `cargo clippy --workspace
  --all-targets --all-features -- -D warnings`, `cargo test --workspace
  --all-features` (126 passed, 0 failed, 20 ignored), release build.
- Phases 0 through 6 are implemented. Phase 6 is committed as `9d960f5` on
  branch `phase-6-detection-and-events`.
- Configuration today is **environment variables only**, validated through
  domain types in `crates/config/src/settings.rs` and applied at startup only.
  `apps/server/src/runtime_config.rs` holds the runtime-observable subset.
- There is no configuration file loading, no persistence, no runtime mutation,
  and no settings API.
- The HTTP surface is `apps/server/src/health_http.rs`, loopback-bound. There is
  no authentication or authorization anywhere in the system.
- PostgreSQL is available via `crates/database` with `cameras` and
  `recording_segments` tables. There is no settings table.
- The domain event bus (`crates/events`) publishes camera, recording, storage,
  detector, and person facts with a `DeliveryClass` per event. There is no
  durable event log; ADR 002 defers persistence and replay to explicit adapters.
- No CI exists. Phase 1 requires it and all 13 of its boxes remain unchecked.

## Product decision gates

Blocked. Do not implement past these without written confirmation:

- ROI polygon coordinate system, validation rules, versioning, and how a region
  associates with a camera.
- Heatmap and occupancy semantics. `AGENTS.md` forbids implementing a heatmap
  before its meaning is agreed — density, occupancy, movement, and dwell are
  different products.
- Whether analytics require identity over time, which is the only justification
  for adding tracking.
- Which rule conditions and actions are actually required. A generic rule engine
  built before its conditions are known is the speculative abstraction
  `AGENTS.md` warns against.

## Sequencing problem to settle before starting

ROADMAP item 4 requires **authenticated** settings APIs. Authentication is
Phase 8 and does not exist; the API is loopback-only.

Plain deferral is the wrong answer: Phase 8 would then retrofit authentication
onto an existing API surface, which is where auth bugs live — the forgotten
endpoint, the handler that skipped the middleware. Pulling Phase 8 forward is
also wrong: authentication done properly needs an identity model, token
lifetime, rotation, and the TLS that arrives with Phase 8.

**Decision: build the seam, not the implementation.** Every mutating settings
endpoint routes through one `authorize(request, action) -> Result<(), Denied>`
chokepoint. Today it allows loopback. In Phase 8 that single function becomes
real and every endpoint inherits it. The deferral is structural in code rather
than a note in a document.

**Plus one real control:** mutating endpoints are opt-in via configuration,
defaulting to off, so a deployed unit exposes no mutable settings unless
someone deliberately enables them. Loopback is a weak boundary on a device
people can reach over SSH or that runs containers; a default-off toggle is not.

Record this in an ADR before writing code — it changes the API contract.

## Contracts to define

### Configuration bootstrap boundary

A setting persisted in PostgreSQL cannot supply the PostgreSQL connection
string. Split the space explicitly:

- **Bootstrap settings** — database URL, bind addresses, recording root,
  credentials. Sourced from file and environment only. Restart-scoped.
- **Operational settings** — thresholds, intervals, retention watermarks,
  per-camera behavior. Persistable and runtime-mutable.

Define which settings are which before implementing precedence. Getting this
wrong produces a system that cannot start when its database is unavailable.

### Behavior when the settings store is unavailable

Splitting the space answers where settings come from. It does not answer what
happens to operational settings when PostgreSQL is unreachable, which is a
normal operating condition for units at sites with unknown connectivity.

Refusing to start is wrong — recording already survives a database outage.
Falling back to defaults is worse than it looks: a retention watermark
reverting to its default during a database flap would start deleting
recordings. That is a data-loss bug wearing a configuration costume.

**Decision: a local last-known-good cache.** Every successful settings load
writes the resolved operational set to a file on local disk. If the store is
unreachable at startup, load that file instead and mark configuration state
`Stale` with the timestamp of the last successful load. Behavior is preserved
across outages, staleness is observable, and nothing silently reverts.

**PostgreSQL is per-unit** (confirmed 2026-08-14). That makes the cache a
genuine fallback rather than the primary path: a local store is unavailable
during a startup race or after corruption, not during a network partition.

Two consequences for the design. `Stale` should be **transient with retry** —
the server keeps attempting to reach the local store and leaves `Stale` on the
first successful load, rather than treating it as a steady operating mode. And
because the cache and the database share a disk, a disk failure takes both;
the cache defends against ordering and corruption, not against storage loss.

Two consequences outside this phase, recorded so they are not discovered later.
PostgreSQL competes with camera decoders, recording buffers, and the TensorRT
engine for unified memory on a Jetson, so it is an input to the detection
sizing rule rather than a free background cost. And per-unit storage means
there is no fleet-wide configuration view — changing a setting across sites is
a distribution problem this phase does not solve.

### Precedence and origin

Precedence order is defaults → configuration file → persisted settings →
environment secrets → API changes. Every effective value must carry its origin,
so an operator can answer "why is this value what it is" without reading code.

### Application boundary

Every setting declares whether a change applies immediately, at a safe boundary
(segment rotation, reconnect, next sample), or only after restart. This is a
property of the setting, not of the update mechanism, and must be reported back
to the caller of an update.

### Audit durability

ROADMAP item 4 says configuration changes publish *audit events*. The event bus
has bounded queues with per-class overflow policies and, per ADR 002, no durable
log. An audit trail that can be dropped under load or lost on restart is not an
audit trail.

**Decision: the audit record writes to PostgreSQL in the same transaction as
the settings change.** Either both land or neither does, which also gives the
settings generation counter and its audit row an atomic ordering. Publishing to
the bus additionally is fine for live consumers, but the bus is the
notification path, not the record.

Read item 4 as "persist audit records and optionally notify" rather than
"publish audit events", and say so in the ADR — as written it points at a bus
that cannot carry it.

### Secret handling

Redaction as policy gets forgotten. Make it structural: a `Secret<T>` wrapper
whose `Debug`, `Display`, and `Serialize` implementations redact
unconditionally, so exposing a value requires deliberately calling `.expose()`.
Leaking then takes an affirmative act rather than an oversight.

Redacting to `old: "***", new: "***"` also produces a useless record — it
cannot show whether anything changed. Record a **SHA-256 fingerprint** instead,
which reveals that a value changed, and that it changed back to a previously
used value, without revealing the value itself.

## Work packages

Buildable now. Each ends with evidence, not assertion.

### P7.1 — Define the configuration domain

Classify every existing setting as bootstrap or operational, and declare each
one's application boundary. Extend the domain types in
`crates/config/src/settings.rs`; do not introduce parallel primitives.
Output: a table in the ADR, and types that make an unclassified setting
impossible to add.

Introduce `Secret<T>` here, before any setting that holds a credential exists in
the persisted or API paths. Retrofitting redaction after those paths are written
means auditing every one of them.

### P7.2 — Implement layered resolution

Implement the precedence chain with origin tracking. File loading joins the
existing environment path. `config/default.toml` and `config/example.toml`
already exist and should become real inputs rather than documentation.

### P7.3 — Implement change classification and reporting

An update returns which settings applied immediately, which are pending a
boundary, and which require restart. Pending-restart state must be observable —
an operator who changes a value and sees no effect needs to know why.

### P7.4 — Implement transactional application

Related changes apply as a unit. A single invalid field rejects the whole
update with a typed error naming the offending setting. No partial application,
no silent coercion.

### P7.5 — Persist operational settings and audit records

Add settings and audit tables with a repository in `crates/database`, following
the `recording_segments` pattern. A settings change and its audit record commit
in one transaction.

Startup loads persisted settings after bootstrap. On store unavailability, load
the local last-known-good cache and enter `Stale`; never fall back to defaults.
Every successful load rewrites that cache.

`Stale` is transient: retry the local store on a bounded interval and leave
`Stale` on the first successful load. The dominant cause is a startup race with
a local PostgreSQL that is not ready yet, so the retry must outlast typical
service start ordering without blocking the server from coming up.

### P7.6 — Expose settings APIs

Read and update endpoints in `apps/server/src/health_http.rs`, following the
existing camera-scoped resource pattern. Secret-safe reads via `Secret<T>`.

Every mutating endpoint routes through the single `authorize` chokepoint, and
the mutating surface is opt-in via configuration, defaulting to off.

### P7.7 — Notify configuration changes on the bus

A new event payload on the existing bus, published *after* the transaction in
P7.5 commits. This is the notification path for live consumers; the durable
record is the audit table. Choosing a `DeliveryClass` here is about consumer
freshness, not about durability, which the bus does not provide.

### P7.8 — Remove hardcoded operational values

Sweep core pipeline logic for values that are expected to change, and move them
into the configuration domain. This closes exit gate 15 and is the item most
likely to be quietly skipped, because it produces no visible feature.

## Deferred work packages

Do not start. Each requires a product decision gate above.

- **P7.9 — Rules as event consumers.** Typed conditions and actions, with alert,
  dashboard, and webhook policy kept out of detection code. The mechanism is
  implementable; the conditions are not known.
- **P7.10 — ROI definition and validation.** Blocked on coordinate semantics.
- **P7.11 — Analytics and heatmap.** Blocked on confirmed meaning.
- **P7.12 — Tracking.** Only if confirmed analytics require identity over time.

## Required failure matrix

At minimum qualify:

- invalid value supplied at each precedence layer;
- conflicting values across layers, verifying the declared precedence wins;
- multi-field update where exactly one field is invalid;
- concurrent updates to overlapping and non-overlapping settings;
- update arriving during shutdown;
- PostgreSQL unavailable at startup, and lost while running;
- store unavailable with a valid cache, with a corrupt cache, and with no cache,
  verifying `Stale` state and that defaults are never silently substituted;
- store unavailable at startup and reachable shortly after, verifying `Stale`
  clears on the first successful load without a restart;
- cache rewritten on every successful load, and survives forced process exit;
- settings change succeeds but the audit write fails, verifying neither commits;
- mutating endpoints disabled by default, and rejected when disabled;
- every mutating endpoint reaches the `authorize` chokepoint, proven by test
  rather than by inspection;
- persisted settings row corrupt or schema-mismatched;
- restart with persisted settings present, absent, and partially present;
- boundary-scoped change applied while a camera is streaming and recording;
- restart-scoped change, verifying it does not apply early;
- secret-valued setting read through every path including audit and logs;
- audit event admission failure under bus overflow.

For every case record expected configuration state, effective values, reported
application boundary, recording and camera behavior, readiness, metrics, and
audit outcome. Do not infer transactional safety from code structure alone.

## Observability requirements

Bounded label cardinality only — setting identity and outcome category, never
values. Expose at least:

- current configuration generation and last-applied timestamp;
- effective value origin per setting group;
- count of settings pending a boundary and pending restart;
- rejected update count by rejection reason;
- persistence availability, configuration freshness state, and the age of the
  last successful load — a unit running on a stale cache must be visible without
  reading logs;
- whether the mutating API surface is enabled;
- audit write and notification outcomes, counted separately.

## Exit gates

From `ROADMAP.md`, unchanged:

- No expected-to-change operational value is hardcoded in core pipeline logic.
- Invalid configuration cannot become active.
- ROI, rule, and analytics behavior matches confirmed product semantics.
  **Cannot close while the product gates are open — expect `[!]`.**
- Action failure does not corrupt event production or media processing.

## Validation commands

```
make check
cargo test -p surveillance-config --all-features
cargo test -p surveillance-database --all-features
scripts/phase4/postgres-fixture.sh
```

A new `scripts/phase7/config-fixture.sh` should qualify precedence, restart
persistence, rollback, and PostgreSQL-outage degradation against a real
database. Record its output in `docs/phase-7/local-evidence-<date>.md` following
the Phase 6 evidence format.

## Explicit first implementation action

Do not write code first. Record the three decisions above in an ADR — the
authorize seam with a default-off mutating surface, the last-known-good cache
with a `Stale` state, and audit persistence in the settings transaction — then
produce the P7.1 classification table: every existing setting labeled bootstrap
or operational, with its application boundary.

Confirm whether PostgreSQL is per-unit or central before writing the ADR. It
changes whether the cache is a fallback or the primary path.

That table is the input to every other work package, and it is cheap to change
now and expensive to change after persistence and APIs are built on top of it.
