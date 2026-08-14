# ADR 008: Persistent Configuration, Authorization Seam, and Audit Durability

Status: accepted for Phase 7 implementation; authorization identity model deferred to Phase 8

Date: 2026-08-14

Supersedes the "Open details" section of ADR 003, which left the persistence
transaction boundary, the operator-editable surface, and post-persistence
rollback behavior unresolved.

## Context

ADR 003 established that configuration is layered, typed, and validated as a
transaction, and that settings declare when they apply. It deliberately left
four questions open. Phase 7 cannot proceed without three of them, because the
answers determine schema, task ownership, and API shape, and all three become
expensive to change once persistence and endpoints exist on top of them.

Known facts about the tree as of this date:

- `crates/config/src/settings.rs` validates 34 fields for one camera in a single
  transaction, computes a coupled pipeline memory bound, and rejects credentials
  embedded in the RTSP endpoint. It has no notion of which fields an operator may
  change at runtime.
- `crates/config/src/repository.rs` and `crates/config/src/validation.rs` are
  one-line stubs. `crates/database/src/settings.rs` is a one-line stub.
- All deployment values load from environment variables in
  `apps/server/src/runtime_config.rs`. `config/default.toml` and
  `config/example.toml` are documentation, not inputs.
- `crates/database/src/recordings.rs` establishes the repository pattern:
  a bounded pool, `RepositoryError` classified into retryable and terminal
  categories, forward migrations owned by the repository, and multi-statement
  writes inside an explicit `pool.begin()` transaction.
- The event bus is bounded and in-process, with per-class overflow policy and
  no durability. ADR 002 deferred persistence and replay to explicit adapters.
- PostgreSQL is deployed per unit, not centrally (confirmed 2026-08-14). A unit's
  database is a local process on the same machine as the server.
- No authentication or authorization exists anywhere. The API is loopback-only.

The fourth open question — the production secret store and rotation mechanism —
remains open and is not decided here. Phase 7 introduces `Secret<T>` as the
in-process representation so that redaction is structural rather than
remembered; where secrets ultimately live is a Phase 8 deployment decision.

## Precedence, restated and made precise

ADR 003 fixed the ordering. Phase 7 makes it concrete, inserting the file layer
that ADR 003 described only as "versioned defaults":

```text
compiled defaults  <  config file  <  persisted settings  <  environment
```

An API change writes the persisted layer. It is not a fifth layer above the
environment. This has a consequence that must be reported rather than hidden: if
the environment pins a setting, an API change to that setting is validated,
persisted, and audited, but does **not** become effective. The change report
names that outcome explicitly as `PersistedButShadowed`, carrying the origin that
wins. Reporting success for a change with no effect is the failure mode this
avoids — an operator who changes a value and sees nothing happen needs to be told
why, in the response, not by reading the environment of a running process.

## Decision 1: A single authorization chokepoint, and a mutating surface that is off by default

Every mutating configuration request passes through one function:

```rust
fn authorize(request: &RequestContext, action: Action) -> Result<Authorized, Denied>;
```

`Authorized` is an unforgeable token: it has private fields, no public
constructor, and is produced only by `authorize`. Every mutating handler
requires one by value. A handler that skips authorization does not compile.

In Phase 7 the implementation grants when the peer is loopback and the mutating
surface is enabled. In Phase 8 it grows an identity model. The seam does not
move; only the body does.

Independently, the entire mutating surface is gated by one configuration flag
that defaults to **off**. When disabled, mutating endpoints return `404`, not
`403` — a disabled surface should not advertise itself. Read endpoints remain
available, because observing effective configuration is how an operator
diagnoses the system and carries no write risk.

Two controls rather than one, because they answer different questions. The flag
answers "is this deployment allowed to be reconfigured over HTTP at all", which
is a deployment posture. `authorize` answers "may this caller perform this
action", which is a request property. Collapsing them would mean either that
enabling the API for one operator enables it for every caller, or that turning
the feature off requires an identity model that does not yet exist.

Rejected: implementing authorization inside each handler. That is the design
that produces the finding "endpoint X forgot the check", and it cannot be proven
absent by inspection at the rate this surface will grow.

Rejected: defaulting the mutating surface on because the listener is
loopback-only. Loopback is not a security boundary on a multi-user host, and the
next phase binds beyond loopback. A default that is safe only under the current
listener configuration is a latent defect.

## Decision 2: Last-known-good local cache with a transient `Stale` state, and defaults are never silently substituted

Persisted settings load after bootstrap. On a successful load the resolved
operational settings are written to a local last-known-good cache file, replacing
any prior contents by atomic rename. Every successful load rewrites it.

When the settings store is unavailable at startup, the server loads the cache and
enters `ConfigurationFreshness::Stale`. It does **not** fall back to defaults.

This is the load-bearing part of the decision. Several operational settings are
destructive when they revert: retention age and storage watermarks delete
recordings. A unit that silently reverted a deliberately-raised retention window
to its default because PostgreSQL was slow to start would delete footage that
policy says to keep, and would do so quietly, at startup, when nobody is looking.
Starting with stale-but-real configuration is strictly safer than starting with
fresh-but-wrong configuration.

`Stale` is transient, not terminal. A background task retries the local store on
a bounded interval and leaves `Stale` on the first successful load, without a
restart. The dominant cause of `Stale` is a startup race with a local PostgreSQL
that has not finished starting, so the retry must outlast typical service start
ordering — the default retry budget therefore exceeds a minute of wall clock —
while never blocking the server from coming up. Media preservation does not wait
for configuration freshness.

Cache states and behavior:

| Store | Cache | Result |
|---|---|---|
| Available | any | `Fresh`; cache rewritten |
| Unavailable | valid | `Stale`; cached values active; retry |
| Unavailable | corrupt or unreadable | `Stale`; bootstrap-only operation; retry; corruption surfaced as a distinct metric and log, and the corrupt file is preserved, not overwritten, until a successful load replaces it |
| Unavailable | absent | `Stale`; bootstrap-only operation; retry |

"Bootstrap-only operation" means the process runs on values resolvable without
the store — defaults, file, and environment. This is not a silent substitution:
freshness is `Stale`, readiness reports it, and the metric that exposes the age
of the last successful load has no value to report. The distinction from the
rejected design is that the operator can see it.

`Stale` degrades readiness but not liveness, and never stops recording. It is
visible in `/ready`, in the settings read endpoint, and in metrics, so that a
unit running on a stale cache can be identified without reading logs.

Rejected: refusing to start without the store. That makes a local database a
hard dependency of media preservation, inverting the confirmed subsystem
priority in which recording outranks everything else.

Rejected: treating the cache as authoritative and the store as a mirror.
PostgreSQL is per-unit and local, so the store is reachable in the ordinary case
and is the natural place for audit history and concurrency control. The cache is
a fallback for a startup race, not a primary path. Were PostgreSQL central and
remote, this decision would invert, which is why it had to be confirmed first.

## Decision 3: The audit record commits in the same transaction as the settings change

A settings change and its audit record are written in one `pool.begin()`
transaction and commit together. If the audit write fails, the settings change
does not commit. If the settings write fails, no audit record appears.

An audit trail with gaps is worse than no audit trail, because it invites the
inference that an unlogged change did not happen. Same-transaction commit makes
"changed but unlogged" unrepresentable rather than rare.

The bus notification in P7.7 is published **after** the transaction commits, and
is explicitly not the record. The bus is bounded, in-process, and lossy by
design; ADR 002 states it provides no durability. Choosing a `DeliveryClass` for
configuration events is a question about consumer freshness, not about whether
the change is remembered. A dropped notification is a consumer that is briefly
behind; the audit table is unaffected. Publication failure or overflow must not
roll back a committed change — the change happened, and a notification is not
evidence.

Audit rows record the actor, the action, the setting key, the origin, the
outcome, and the old and new values. Secret-valued settings must not appear.
Storing the value would defeat `Secret<T>` at the one place most likely to be
exported. Redacting to `old: "***", new: "***"` produces a record that cannot
show whether anything changed.

Secrets are therefore recorded as a **SHA-256 fingerprint** of the value. A
fingerprint reveals that a value changed, and that it changed back to a
previously used value — both of which are audit-relevant — without revealing the
value. Fingerprints are salted per deployment so that they are not comparable
across units and are not subject to precomputation against a small candidate
space; the salt is a bootstrap setting and is itself a `Secret`.

## Decision 4: Runtime application failure after a committed change does not roll back the commit

ADR 003 left open what happens when a change validates and persists, and then
fails to apply to the running system. Validation is pure and transactional and
runs before the commit, so a failure at application time is an environmental
fault — a resource that could not be acquired, an executable that has since
disappeared — not an invalid value.

The persisted row therefore stays. Operator intent is recorded. The previously
effective value remains in force, the setting is marked `ApplyFailed` with a
typed failure category, and that state is visible in the read endpoint and in
metrics. Application is retried at the next boundary for boundary-scoped
settings.

Rejected: automatically reverting the persisted row. It discards a deliberate
operator decision on the strength of a possibly transient fault, and it makes
the durable record disagree with what the operator was told they had done.

Rejected: reporting success and leaving the discrepancy implicit. A setting whose
stored value and effective value differ is exactly the condition an operator
needs surfaced, which is why the read endpoint reports stored and effective
values separately rather than collapsing them into one number.

## Consequences

- The settings schema needs an actor column that Phase 7 cannot populate
  meaningfully. It records the loopback origin and a placeholder identity, and
  Phase 8 replaces the placeholder without a migration of shape.
- Every operational setting acquires a durable identity — its key string — that
  appears in the database, the API, metrics labels, and audit rows. Renaming one
  later is a migration, not a refactor. This is the cost of the P7.1 table being
  the input to everything else, and the reason to get it right before code.
- Bootstrap settings cannot be changed through the API at all. An operator who
  wants a new listen address edits the file and restarts. This is intentional:
  a setting that can lock out the interface used to change it does not belong on
  that interface.
- The cache file becomes a new piece of on-disk state with its own failure modes
  (corrupt, absent, stale, unwritable). Each needs a test, and unwritable must
  not fail the load that produced it.

## Acceptance test inventory

Every case names its expected configuration state, effective values, reported
application boundary, recording and camera behavior, readiness, metrics, and
audit outcome. Transactional safety is not inferred from code structure.

1. Invalid value supplied at each precedence layer.
2. Conflicting values across layers; the declared precedence wins.
3. Multi-field update where exactly one field is invalid.
4. Concurrent updates to overlapping and to non-overlapping settings.
5. Update arriving during shutdown.
6. PostgreSQL unavailable at startup, and lost while running.
7. Store unavailable with a valid cache, a corrupt cache, and no cache.
8. Store unavailable at startup then reachable; `Stale` clears without restart.
9. Cache rewritten on every successful load; survives forced process exit.
10. Settings change succeeds but audit write fails; neither commits.
11. Mutating endpoints disabled by default, and rejected when disabled.
12. Every mutating endpoint reaches `authorize`, proven by test, not inspection.
13. Persisted settings row corrupt or schema-mismatched.
14. Restart with persisted settings present, absent, and partially present.
15. Boundary-scoped change applied while a camera is streaming and recording.
16. Restart-scoped change does not apply early.
17. Secret-valued setting read through every path including audit and logs.
18. Audit event admission failure under bus overflow.

## Alternatives

- **Authorization as middleware rather than a typed token.** Rejected: middleware
  is bypassed by any handler registered outside it, and the hand-rolled `route()`
  function in `apps/server/src/health_http.rs` has no middleware layer to attach
  to. A token required by the handler signature is checked by the compiler.
- **Optimistic concurrency by row version rather than serialized transactions.**
  Deferred, not rejected. The expected write rate is an operator occasionally
  changing a value; contention is not the problem to solve first. The transaction
  boundary decided here is compatible with adding versioning later.
- **Writing audit records to the event bus only.** Rejected under ADR 002: the
  bus has no durability, so this produces exactly the gapped trail described
  above.
- **Encrypting rather than fingerprinting secret values in audit rows.**
  Rejected: it makes audit export a decryption-key problem and preserves a
  recoverable copy of every secret ever set, which is a larger liability than
  the question it answers.

## Reversal criteria

- Revisit the cache-as-fallback decision if PostgreSQL becomes central and
  remote. The store would then be routinely unreachable rather than
  exceptionally, and the cache would become a primary path needing its own
  write-back and conflict semantics.
- Revisit same-transaction audit if audit volume grows to where it contends with
  settings writes. The measurement that would justify it is settings-write
  latency attributable to audit insertion, not an expectation of it.
- Revisit the default-off mutating surface when Phase 8 lands an identity model
  and a deployment where the API is the intended administration path.
- Revisit fingerprinting if a confirmed requirement demands showing an operator
  what a secret was, which would require a key-management decision first.
