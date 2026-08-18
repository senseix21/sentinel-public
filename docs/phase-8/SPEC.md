# Phase 8 Specification — Security, Operability, and Production Readiness

Written 2026-08-15 against `e65a318`. Authority order: `ROADMAP.md` Phase 8 for
scope, ADR 009 for the decisions, `docs/phase-8/HANDOFF.md` for the seams.

This file defines contracts and observable acceptance criteria. `PLAN.md` orders
the work and carries status.

## Decisions this specification assumes

Recorded in ADR 009, confirmed by the product owner 2026-08-15:

| # | Decision |
|---|---|
| 1 | Opaque bearer tokens; human operators and service identities; Argon2id at rest |
| 2 | All API reads authenticated; `/live`, `/ready`, `/metrics` open on a separate bind |
| 3 | Reverse proxy terminates TLS; origin proved by peer allowlist **and** shared-secret header |
| 4 | Secrets in a `0600` file, environment override per key |
| 5 | RPO one segment, RTO 15 minutes |
| — | Deployment target **not confirmed** — packaging and target-specific runbooks blocked |

## P8.1 — Threat model

**Deliverable** `docs/phase-8/threat-model.md`.

Seven trust boundaries: camera↔server, client↔API, server↔database,
server↔filesystem, server↔LiveKit, dashboard↔server, operator↔deployment.

Per boundary, name: what crosses it, who is trusted, what an attacker holding
that position can do, which control stops them, and where that control lives in
the tree. A boundary whose control is "none today" says so and links the package
that adds one.

**Acceptance** Every subsequent package traces to at least one threat. Any threat
with no mitigating package is listed as accepted risk with a reason.

## P8.2 — Identity and authentication

**New crate** `crates/auth`. It owns identity; it does not own settings,
persistence, or HTTP.

### Contracts

```text
PrincipalId      validated, bounded, stable; renders into Actor's 128-byte bound
PrincipalKind    Human | Service
Principal        id, kind, display name, enabled flag, role, camera scope
TokenId          the public half; indexed; safe to log
TokenSecret      Secret<String>; the private half; never stored, never logged
IssuedToken      TokenId + TokenSecret + expiry; produced once, at creation
StoredToken      TokenId, principal, Argon2id hash + params, expiry, revoked_at
Authentication   Authenticated(Principal) | Anonymous | Rejected(AuthFailure)
AuthFailure      Malformed | UnknownToken | BadSecret | Expired | Revoked | PrincipalDisabled
```

`TokenStore` is a trait with a Postgres implementation in
`crates/database/src/auth.rs` and an in-memory implementation for tests.
Migration `0003_auth.sql`.

### Required behavior

- Token verification is constant-time on the secret comparison and does not
  short-circuit on principal lookup failure in a way that leaks existence
  through timing. An unknown `TokenId` still performs a dummy Argon2id
  verification.
- `AuthFailure` is *not* rendered to the client. The HTTP layer collapses every
  variant to `401` with a constant body. The variant goes to the audit trail and
  a labelled metric.
- Expiry is mandatory. A token without an expiry cannot be constructed.
- Revocation is immediate: `revoked_at` is checked on every verification, not
  cached.
- `Actor` content becomes `principal:<kind>:<id>`, replacing `Actor::loopback()`.

**Acceptance** Unit tests for each `AuthFailure` variant; a test that the
plaintext token appears in no `Debug`, `Display`, `Serialize`, or log output; a
test that verification of an unknown token and of a wrong secret both perform
hashing work; round-trip issue→verify→revoke→verify against a real database in
the fixture.

## P8.3 — Camera-scoped authorization

### Contracts

```text
Action           one variant per protected operation, each carrying its scope
ActionScope      Global | Camera(CameraId)
Role             Viewer | Operator | Administrator
CameraScope      All | Only(bounded set of CameraId)
```

`authorize(request, action) -> Result<Authorized, Denied>` keeps its signature.
Its body becomes: surface check → shutdown check → authentication → role
permits action → camera scope contains the action's camera.

`Denied` gains `Unauthenticated` (renders `401`) distinct from `Unauthorized`
(renders `403`). `SurfaceDisabled` still renders `404` and is still checked
first.

### Required behavior

- Read endpoints authenticate through the same chokepoint. `Authorized` is
  required to *read* camera health, camera list, recording search, recording
  detail, settings, and viewer tokens.
- A principal scoped to camera A receives `404`, not `403`, for camera B — a
  `403` confirms the camera exists.
- Role/action mapping is a table in one place, tested exhaustively over the
  product of roles and actions.

**Acceptance** The Phase 7 test `every_mutating_settings_request_reaches_the_
chokepoint` grows into a table-driven test over the **full protected route set**,
so that a route added without a check fails. An exhaustive role×action matrix
test. A cross-camera access test asserting `404`.

## P8.4 — Trust boundary

### Contracts

```text
TrustBoundary        Loopback | ReverseProxy { peers, proof } | (DirectTls reserved)
ForwardedClient      the client address, present only when origin was proved
ProofOutcome         Proved | PeerNotAllowed | ProofMissing | ProofMismatch
```

### Required behavior

- Forwarded headers are read **only** after both proofs pass. On failure the
  forwarded value is discarded and the peer address is used; it is never
  defaulted to something permissive.
- The proof secret is compared in constant time.
- `X-Forwarded-For` with multiple hops takes the **rightmost** entry the proxy
  appended, not the leftmost, which is attacker-controlled.
- `HealthBindNotLoopback` relaxes only when a trust boundary is configured *and*
  an authentication provider with at least one enabled principal exists.
  Otherwise startup fails naming the missing condition.

**Acceptance** Tests for each `ProofOutcome`; a spoofed `X-Forwarded-For` from a
non-allowlisted peer does not become the client identity; a non-loopback bind
without authentication is refused at startup; a non-loopback bind with both is
accepted.

## P8.5 — Secret storage and rotation

### Contracts

```text
SecretKey        namespaced, validated: `camera.<id>.rtsp_password`, `livekit.api_secret`, ...
SecretStore      get(&SecretKey) -> Option<&Secret<String>>
SecretSource     File { path, mode } | Environment { var }
SecretLoadError  PermissionsTooOpen { observed_mode } | NotOwned | Unreadable | Malformed | ...
```

### Required behavior

- File mode must be exactly `0600`; a wider mode is refused with the observed
  mode in the error. Ownership must match the running user.
- Environment overrides a file key; the resolved source is recorded per key so
  `GET /api/settings` can report *where* a secret came from without reporting
  what it is.
- A missing required secret is a named startup failure and a metric, never a
  silent fallback to unauthenticated.
- Rotation: replace the file and signal reload; in-flight camera sessions keep
  their existing credential until their next reconnect, which is the documented
  application boundary.
- Removes the Phase 7 restriction that secrets are neither persisted nor
  API-editable. Secret settings become editable; responses return the
  `Fingerprint` and source, never the value.

**Acceptance** Permission-refusal test with an actual `0644` file; override
precedence test; per-camera credential test with two cameras holding different
passwords; a test that no secret value appears in the settings API response,
audit row, or log; rotation test that a reload changes the fingerprint.

## P8.6 — Audit beyond configuration

Extends `settings_audit`'s shape to `access_audit` (migration `0004_audit.sql`):
authentication success/failure, authorization denial, camera lifecycle
operations, recording export, and principal/token changes.

### Required behavior

- Same-transaction commit wherever a transaction exists, matching Phase 7.
- Sensitive values are fingerprinted, never stored.
- Recording export records principal, camera, exact time range, byte count, and
  outcome. It is the operation that removes footage from the system's control.
- An audit write failure for a *mutating* operation fails the operation. An
  audit write failure for a *read* is counted and does not fail the read —
  stated explicitly because the two are different risk trades.

**Acceptance** A test per audited operation asserting a row with the right
principal; a fingerprint-not-value test; a test that a failed audit write rolls
back the mutation it accompanied.

## P8.7 — Health, metrics, dashboards, alerts

Liveness/readiness/per-camera degradation already exist. This package adds:

- `surveillance_auth_*` and `surveillance_authz_*` series, bounded cardinality —
  labelled by outcome and role, **never** by principal ID or token.
- `docs/phase-8/dashboards/` Grafana JSON and `docs/phase-8/alerts.yml`
  Prometheus rules for: camera down, recording stalled, disk critical, database
  unavailable, authentication failure spike, queue saturation.
- A rule-evaluation test that feeds synthetic series to the alert expressions and
  asserts each fires. An alert that has only been written is not tested.

**Acceptance** Each critical alert has a test that makes it fire and a test that
it stays silent under nominal series. Cardinality test over the metric families.

## P8.8 — Packaging — **BLOCKED**

Deployment target unconfirmed (ADR 009). Roadmap item stays `[!]` with the
reason. Not guessed.

## P8.9 — Runbooks — **PARTIAL**

Written, target-independent: disk-full, camera-offline, database-outage,
LiveKit-outage.

Blocked on the deployment target: deployment, upgrade, rollback, backup/restore.
Their RPO/RTO inputs are settled (Decision 5) but the procedures are not
writable without knowing the unit.

**Acceptance** Each written runbook is executed against the local fixture and the
result recorded. A runbook that has only been read is untested and is labelled
so.

## P8.10 — Dependency and input hardening

- `cargo deny` configuration (`deny.toml`) covering advisories, licenses, and
  duplicate versions; `cargo audit` added to `make check`.
- Documented system-library patching policy for FFmpeg and the RTSP stack.
- Fuzz targets for the parsers that consume attacker-influenced bytes: RFC 6184
  H.264 depayloading in `crates/camera`, and the HTTP request-line, header,
  range, and time-query parsing in `apps/server`.

**Acceptance** `cargo deny check` passes in the gate. Each fuzz target runs a
bounded corpus in CI without panic; findings become regression tests.

## Failure matrix

Ten cases. For each: expected telemetry, operator-visible symptom, recovery
action, and whether the runbook alone sufficed.

1. Camera disconnect and flapping
2. Network partition
3. Detector and LiveKit process failure
4. PostgreSQL outage and recovery
5. Disk warning, critical, full, write error
6. Abrupt restart during segment finalization
7. Invalid/expired credentials and unauthorized cross-camera access
8. Graceful `SIGTERM` under active recording and in-flight API requests
9. Concurrent settings updates, overlapping and non-overlapping *(carried from Phase 7)*
10. Boundary-scoped configuration change during live recording *(carried from Phase 7)*

Cases 1–6 have partial Phase 4–6 evidence. That evidence is **not** re-reported
as Phase 8's: the question here is whether an operator can diagnose and recover
from documented telemetry, which is a different claim from whether the system
survives.

## Validation

```bash
make check
cargo deny check
scripts/phase4/postgres-fixture.sh
scripts/phase7/config-fixture.sh
scripts/phase8/security-fixture.sh    # new
```

Evidence in `docs/phase-8/local-evidence-<date>.md`, Phase 7 format: commands
with actual results, a failure-matrix table naming what is *not* covered, and
assumptions stated separately from measurements. Anything that cannot run is
reported as unverified with the reason, never omitted.
