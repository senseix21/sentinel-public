# Phase 8 Handoff — Security, Operability, and Production Readiness

Written 2026-08-15 against `9a40a80` on `phase-7-persistent-configuration`.

`ROADMAP.md` Phase 8 is the authority on scope. This file adds what the roadmap
cannot: the seams Phase 7 left in place, the decisions that must be made before
code, and the things that are currently true and must stop being true.

Phase 8 is different in kind from Phases 3–7. Those added capability. This one
mostly *removes* comfortable defaults — loopback-only binding, an absent identity
model, unauthenticated reads — and replaces them with things that can fail
closed. Expect to spend more time on decisions and threat modelling than on
lines of code.

## What Phase 7 handed you

### Seams built to be filled, not moved

| Seam | Location | State |
|---|---|---|
| `authorize(request, action)` | `apps/server/src/authorize.rs` | The single decision point for mutating configuration. Grants on loopback. **Only the body changes.** |
| `Authorized` | same | Private field, no public constructor. A handler that skips the check does not compile. |
| `Action` | same | Two variants. Phase 8 grows it per protected operation. |
| `Actor` | `crates/config/src/repository.rs` | `Actor::loopback()` is a placeholder identity. The `settings_audit.actor` column already exists and is bounded to 128 bytes. **Shape does not change; content does.** |
| `Secret<T>`, `Fingerprint` | `crates/config/src/secret.rs` | Redacting `Debug`/`Display`/`Serialize`; salted SHA-256 for audit. |
| `MutatingSurface` | `apps/server/src/authorize.rs` | Deployment posture, default off. |
| `settings_audit` | `migrations/0002_settings.sql` | Configuration changes only. Phase 8 must extend audit to access, camera, export, and authorization changes. |

Read ADR 008 before touching any of these. Its Decision 1 explains why there are
two controls rather than one, and why a disabled surface answers `404`.

### What is currently true and must stop being true

- **`server.health_bind` is validated to reject non-loopback addresses.**
  `EnvironmentConfigError::HealthBindNotLoopback` in
  `apps/server/src/runtime_config.rs` enforces this. It is a deliberate lock that
  Phase 8 must unlock *together with* TLS and authentication, not before.
- **Reads are unauthenticated.** `GET /api/settings`, `/api/cameras`, recording
  search, and `/metrics` are open to anything that reaches the socket. That was
  defensible at loopback-only; it is not defensible once the listener moves.
- **Viewer tokens are labelled development-only** and minted without
  authenticating the requester.
- **`authorize` returns `Denied::Unauthorized` for an unknown peer.** Correct
  today. Behind a reverse proxy every peer becomes the proxy, so the identity
  signal must move to the request, not the socket.

### Phase 7 gaps that are Phase 8's problem

Recorded honestly in `docs/phase-7/local-evidence-2026-08-15.md`:

- **Concurrent settings updates are untested.** The transaction boundary and
  `settings_generation_sequence` make the design defensible; ADR 008 forbids
  calling that proof. Phase 8's failure qualification touches the same store.
- **Secrets are neither persisted nor API-editable**, so cameras cannot have
  different credentials. This is blocked on the decision below and is the single
  most likely thing to block a real ten-camera deployment.
- Audit covers configuration only.
- A boundary-scoped change during live recording is unqualified.

## Decisions required before code

Phase 7's mistake to avoid repeating: the classification table initially declared
seventeen settings `Immediate` whose owners captured them at construction. It
was caught, but only after the table had been written into three documents.
Settle these first.

### 1. Identity model and authorization granularity

`authorize` needs a body. That requires knowing:

- **Who authenticates?** Human operators, service identities, or both. A service
  identity that never rotates is a shared password with better branding.
- **What is the credential?** Session cookie, bearer token, mTLS certificate.
  This decides whether the API can be stateless and whether a reverse proxy can
  terminate identity.
- **Is authorization camera-scoped, or role-based, or both?** The roadmap says
  camera-scoped. `Action` currently has no camera dimension; adding one changes
  every handler signature, so decide before growing the enum.
- **Are reads authenticated?** Not rhetorical. `/metrics` is usually scraped by
  an unauthenticated collector on a private network, and recording search
  exposes when a site was occupied. These may need different answers.

Until this is settled, `authorize` cannot be written, and writing handlers
against a guessed `Action` shape wastes them.

### 2. The trust boundary: TLS directly, or a reverse proxy

Roadmap item 3. The two answers produce different systems:

- **Direct TLS** means certificate lifecycle, renewal, and reload inside the
  server, and `authorize` keeps reading the peer address.
- **Reverse proxy** means the server trusts a header, which is only safe if it
  can prove the request came from the proxy. Every "trusted header" design that
  omits that proof is an authentication bypass.

Decide this before touching `health_bind`, because the unlock is only safe under
one of them.

### 3. Secret storage and rotation

Still open from ADR 003, deferred again by ADR 008. It blocks:

- per-camera credentials (the environment layer has no per-camera syntax);
- secret persistence and API editing;
- credential rotation, which the roadmap requires.

`Secret<T>` is the in-process representation and does not prejudge the store.
Options span environment injection, a file with enforced permissions, an OS
keyring, or an external secret manager. Each has a different failure mode when
the secret is unavailable at startup — and Phase 7 already established the rule
for that case: degrade visibly, never silently substitute.

### 4. Deployment target and model

Roadmap items 8 and 9 cannot start without: the confirmed Linux distribution and
version, whether the unit is a container or a systemd service, the storage
layout, and who performs upgrades. `AGENTS.md` lists these as open decision
gates. A runbook written against a guessed target is worse than no runbook,
because it will be followed.

### 5. Availability objectives and data-loss tolerance

Also open in `AGENTS.md`. Backup/restore, upgrade, and rollback runbooks all
encode an implicit RPO and RTO. Write them down first, or the runbooks will
encode whatever was convenient.

## Work packages

Buildable once the decisions above are made. Each ends with evidence.

### P8.1 — Threat model

Camera, API, database, filesystem, LiveKit, dashboard, and deployment trust
boundaries. Output is a document naming, per boundary: what crosses it, who is
trusted, what an attacker with that position can do, and which control stops
them. This is not paperwork — it is what makes the remaining packages
prioritisable rather than a checklist.

Do this first. It is the only package with no prerequisite.

### P8.2 — Identity and authentication

Users and/or service identities, credential storage with a real password hash if
passwords are used, session or token lifecycle, and expiry. Fill the `authorize`
body. Replace `Actor::loopback()` with the authenticated identity; the audit
column already exists.

### P8.3 — Camera-scoped authorization

Extend `Action` per protected operation. Every mutating handler already takes
`Authorized` by value, so the compiler enumerates the sites for you. Add the
camera dimension decided in §1.

The Phase 7 test `every_mutating_settings_request_reaches_the_chokepoint` should
grow into a table-driven test over the full protected route set, so a route added
without a check fails.

### P8.4 — TLS or proxy boundary

Per §2. If a proxy, implement and test the proof that a request actually came
from it. If direct, implement certificate reload without dropping active
recordings.

Only after this: relax `HealthBindNotLoopback`, deliberately, with a test that
a non-loopback bind without TLS and authentication is refused.

### P8.5 — Secret storage and rotation

Per §3. Then remove the Phase 7 restriction: make secret settings persistable and
API-editable, which unblocks per-camera credentials. The audit path already
fingerprints rather than storing values, so it does not need to change.

File permissions and rotation procedure are part of this package, not a
follow-up.

### P8.6 — Audit beyond configuration

Extend to access, camera lifecycle, recording export, and authorization changes.
Reuse the `settings_audit` shape: same-transaction commit where a transaction
exists, and a fingerprint rather than a value for anything sensitive.

Recording *export* is the one to think hardest about: it is the operation that
removes footage from the system's control, and it is the one an insider abuses.

### P8.7 — Health, metrics, dashboards, alerts

Liveness, readiness, and per-camera degradation are already distinguished, and
`/ready` already reports configuration `Stale`. Phase 8 adds dashboards and
alerts, and must qualify that each critical alert actually fires — an untested
alert is a belief, not a control.

`surveillance_configuration_*` series landed in `9a40a80`; see
`docs/phase-7/contracts.md` §6 for what they mean.

### P8.8 — Packaging

Per §4. Restart policy, resource limits, and a non-root user with only the
filesystem access it needs.

### P8.9 — Runbooks

Deployment, upgrade, rollback, backup/restore, disk-full, camera-offline,
database-outage, LiveKit-outage. Each must be executed at least once by someone
following it literally. A runbook that has only been read is untested.

### P8.10 — Dependency and input hardening

`cargo audit` / `cargo deny` in the gate, a documented system-library patching
policy, and fuzzing for externally supplied parsers. The highest-value fuzz
targets are the ones parsing attacker-influenced bytes: RTSP/H.264 depayloading
in `crates/camera`, and the range/time/query parsing in the HTTP layer.

## Required failure matrix

Roadmap Phase 8 lists eight. For each, record expected telemetry, operator-
visible symptom, recovery action, and whether the runbook alone was sufficient.

1. Camera disconnect and flapping.
2. Network partition.
3. Detector and LiveKit process failure.
4. PostgreSQL outage and recovery.
5. Disk warning, critical, full, and write error.
6. Abrupt machine/process restart during segment finalization.
7. Invalid or expired credentials, and unauthorized cross-camera access.
8. Graceful `SIGTERM` under active recording and in-flight API requests.

Carry forward from Phase 7, because they touch the same store and the same
listener:

9. Concurrent settings updates, overlapping and non-overlapping.
10. A boundary-scoped configuration change during live recording.

Cases 1 through 6 have partial Phase 4–6 evidence. Do not re-report that evidence
as Phase 8's: the question here is whether an *operator* can diagnose and recover
from documented telemetry, which is a different claim from whether the system
survives.

## Validation

```
make check
scripts/phase4/postgres-fixture.sh
scripts/phase7/config-fixture.sh
scripts/phase8/security-fixture.sh          # new
cargo audit                                  # new to the gate
```

Record output in `docs/phase-8/local-evidence-<date>.md`, in the Phase 7 format:
commands run with actual results, a failure-matrix table naming what is *not*
covered, and assumptions stated separately from measurements.

Anything that cannot run — no target hardware, no representative cameras, no
external secret manager — is reported as unverified with the reason, never
omitted.

## Explicit first action

Do not write code first.

1. Produce the P8.1 threat model. It is the only package with no prerequisite,
   and it tells you which of the decisions below actually matter most.
2. Take decisions §1 (identity), §2 (trust boundary), and §3 (secrets) to the
   product owner together. They interlock: the credential type constrains the
   proxy design, and the proxy design constrains where secrets must live.
3. Record the answers in an ADR before filling the `authorize` body.

Phase 7's `authorize` seam exists precisely so that step 3 is a body change and
not an architecture change. Spending the decisions first is what keeps it that
way.

## Do not

- Do not relax `HealthBindNotLoopback` before TLS and authentication exist.
- Do not describe the settings API as authenticated until it is; the roadmap
  item is `[~]` and should stay that way until P8.2 lands.
- Do not add a "trusted header" identity path without proving the request came
  from the proxy.
- Do not write a runbook against an unconfirmed deployment target.
- Do not mark an alert as tested because it exists.
