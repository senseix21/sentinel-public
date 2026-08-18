# ADR 009: Identity, Trust Boundary, and the Secret Store

Status: accepted for Phase 8 implementation; deployment target and packaging deferred

Date: 2026-08-15

Closes the identity question ADR 008 deferred to Phase 8, and the production
secret store question left open by ADR 003 and deferred again by ADR 008.

## Context

`docs/phase-8/HANDOFF.md` names five decisions that must precede code. Three of
them interlock — the credential type constrains the proxy design, and the proxy
design constrains where secrets must live — so they are decided together here.
The remaining two (deployment target, availability objectives) are recorded at
the end: one is answered, one is not.

Known facts about the tree as of this date:

- `apps/server/src/authorize.rs` is the single decision point for mutating
  configuration. `Authorized` has a private field and no public constructor, so
  a handler that skips the check does not compile. `Action` has two variants and
  no camera dimension. The body grants on loopback.
- `Actor::loopback()` in `crates/config/src/repository.rs` is a placeholder
  identity. `settings_audit.actor` exists and is bounded to 128 bytes.
- `Secret<T>` and `Fingerprint` in `crates/config/src/secret.rs` redact
  `Debug`/`Display`/`Serialize` and fingerprint with a salted SHA-256. Neither
  prejudges a store.
- `EnvironmentConfigError::HealthBindNotLoopback` in
  `apps/server/src/runtime_config.rs` refuses any non-loopback listener address.
- The HTTP surface is a hand-rolled dispatcher in
  `apps/server/src/health_http.rs`. Reads (`/api/cameras`, recording search,
  `GET /api/settings`, `/metrics`) are unauthenticated.
- `crates/database/src/settings.rs` establishes the repository pattern: bounded
  pool, `RepositoryError` classified retryable/terminal, forward migrations
  owned by the repository, multi-statement writes inside `pool.begin()`.
- PostgreSQL is per unit, not central (ADR 008).

## Decision 1 — Bearer tokens for both human and service identities

A `Principal` is either a human operator or a service identity. Both
authenticate with an opaque bearer token presented as
`Authorization: Bearer <token>`.

The token is a 256-bit random value rendered as URL-safe base64 with a
`srv_` prefix and a short public identifier segment. Only the identifier is
indexed; the secret half is verified against an Argon2id hash. The store never
holds a usable token.

Rejected alternatives:

- **Session cookies, humans only.** Would require a second mechanism for service
  identities and for any non-browser client. Two authentication paths is two
  chances to get it wrong.
- **mTLS client certificates.** The strongest binding, and it removes the secret
  from the request entirely. Rejected because nothing in this repository has a
  certificate lifecycle, and issuance/renewal/revocation would be new
  operational surface in the same phase that introduces authentication at all.

Reversal criteria: adopt mTLS if a certificate authority becomes part of the
confirmed deployment, or if a threat model revision rules bearer tokens
insufficient against request-log capture. `Principal` is deliberately
independent of the credential type so the verifier can be replaced without
touching authorization.

### Why Argon2id and not SHA-256

A bearer token is high-entropy, so an offline attacker cannot brute-force it and
a fast hash would arguably be adequate. Argon2id is used anyway because the same
table is intended to hold operator passwords later, and a store that is correct
only while its inputs stay high-entropy is a latent defect. Parameters are
recorded per row so they can be raised without invalidating existing rows.

## Decision 2 — All API reads authenticated; `/metrics` open on a separate bind

Recording search reveals when a site was occupied, and camera health reveals
which cameras are blind. Both are authenticated.

`/metrics` stays unauthenticated, because the collector in a single-machine
deployment is an unauthenticated scraper on a private network, and requiring a
token there means putting a token in the scrape config. It is instead bound to
its own listener address so that it can be kept off the routable interface
independently of the API. `/live` and `/ready` follow `/metrics`: an orchestrator
health probe that needs a credential fails for the wrong reasons.

This makes exactly three unauthenticated endpoints, all of which are either
constant or aggregate, and none of which name a recording or a time range.

Reversal criteria: authenticate `/metrics` if the collector moves off the unit
or the metric set grows a series that identifies a person or an event time.

## Decision 3 — Reverse proxy terminates TLS, with proof of origin

The server does not terminate TLS. A reverse proxy does, and the server binds a
private address behind it.

A "trusted header" design that omits proof of origin is an authentication
bypass, so the proof is mandatory and has two independent parts:

1. **Peer allowlist.** The connection's peer address must be in a configured
   allowlist of proxy addresses.
2. **Shared secret header.** The request must carry
   `X-Forwarded-Proof: <secret>`, compared in constant time against a configured
   `Secret<String>`.

Both must hold before any forwarded header is read. If either fails, the
forwarded client address is discarded — not defaulted — and the request is
treated as coming from the peer directly.

Rejected alternative: **direct TLS in the server.** It keeps `authorize` reading
the peer address and removes the proxy dependency, but it puts certificate
renewal and hot reload inside the process that owns active recordings. A reload
bug then costs footage. Deferred, not refused: `TrustBoundary` is an enum with
room for a `DirectTls` variant.

Reversal criteria: implement direct TLS if the confirmed deployment has no
proxy, or if the proxy is found to be the availability bottleneck.

### The unlock condition for `HealthBindNotLoopback`

The non-loopback bind is refused unless *all* of:

- a trust boundary is configured (`TrustBoundary::ReverseProxy` with a non-empty
  peer allowlist and a proof secret), and
- an authentication provider is configured with at least one enabled principal.

This is enforced as a validation rule, not documentation. A non-loopback bind
without both is a startup error naming which condition failed.

## Decision 4 — Permission-enforced secrets file, with environment override

Secrets live in a file whose mode must be `0600` and whose owner must be the
running user. The loader refuses to read a wider-permissioned file and reports
the observed mode. Environment variables override individual keys so a container
can inject without a file.

This unblocks per-camera credentials: the file is keyed, so
`camera.<id>.rtsp_password` is expressible where the flat environment layer was
not.

Rejected alternatives:

- **Encrypted in PostgreSQL with a key from the environment.** Makes credentials
  ordinary rows and API-editable, but introduces a key-encryption key with the
  same storage problem one level down, and makes a database outage a camera
  authentication outage. Recording preservation outranks configuration
  convenience.
- **External secret manager.** Correct for multi-host, but no such
  infrastructure is confirmed, and an untestable integration is not a control.

Reversal criteria: move to an external manager when more than one unit shares a
credential, or when rotation must be driven by something other than an operator
on the host.

Unavailability behavior follows the Phase 7 rule: degrade visibly, never
silently substitute. A camera whose credential is missing fails to start with a
named error and a metric; it does not fall back to an unauthenticated connect.

## Decision 5 — Availability objectives

Confirmed 2026-08-15: **RPO is one segment** (at most the in-flight 30–60
seconds may be lost) and **RTO is 15 minutes** from the operator beginning
recovery.

Consequences that the runbooks encode: the database is backed up nightly with
WAL archiving; loss of the index falls back to the existing filesystem
reconciliation scan rather than to a restore; and no synchronous cross-host
durability is required, which is what keeps a single-machine deployment viable.

## Decision 6 — Browser authentication is asserted by the proxy

Confirmed 2026-08-15. Decision 1 gave this system opaque bearer tokens, which
work well for a script and badly for a browser: a token has to be stored
somewhere, and every place a browser can store one is worse than the last for a
system that streams footage of a physical space.

### Options

1. **Token in `localStorage`.** Simple, no server change. Readable by any script
   that achieves XSS — and a single successful XSS then yields a long-lived
   credential for live cameras and the recording archive, exfiltrable in one
   request. Rejected.
2. **Session cookie issued by this server.** `HttpOnly` puts the credential out
   of JavaScript's reach, which is a real improvement. It also obliges this
   system to own session storage, expiry, rotation, logout, and CSRF defence on
   every mutating route — a second authentication system beside the token one,
   and the larger half. Rejected on scope, not on security.
3. **The reverse proxy authenticates and asserts the principal.** Rejected
   forms of this are notorious, which is the whole risk: header-asserted
   identity is only as good as the guarantee that the header came from the
   proxy. This system already has that guarantee. **Chosen.**

### Decision

The reverse proxy authenticates the human by whatever means its operator already
runs — OIDC, SAML, mTLS, an existing SSO — and asserts the result in
`X-Authenticated-Principal`. This server reads that header **only** after
`ProofOutcome::proves_origin()` holds, which is the peer allowlist *and* the
constant-time shared-secret header from Decision 2. Precedence at the
chokepoint is: a bearer token if presented, otherwise a proved assertion,
otherwise the loopback development identity.

The proxy asserts **who**, never **what they may do**. The name is looked up in
the local `principals` table and the role and camera scope come from there. Two
consequences follow, and both are tested: a principal the proxy authenticates
but this system does not hold is refused (`unknown_principal`), and a principal
this system has suspended is refused (`principal_disabled`) even though the
proxy still authenticates them. Without the second, suspension here would be
decorative and revocation would mean editing proxy configuration.

No hashing occurs on this path, so it takes no permit from the Argon2id
semaphore of Decision 1 and cannot contribute to that queue.

### Trade-offs accepted

The proxy configuration becomes security-critical: an operator who allowlists a
peer they do not control, or who lets the proof secret leak, has handed out
identity assertion. This is recorded in the threat model rather than mitigated
in code, because no code on this side can distinguish a correctly-proved
assertion from a correctly-proved lie. What this system does instead is make the
proof hard to satisfy accidentally — two independent conditions, neither of
which a client can set — and refuse a routable listener without it.

We also accept that there is no logout on this side. Sessions belong to the
proxy, which is the component that knows what a session is.

### Reversal criteria

Revisit if a deployment has no reverse proxy and still needs browser access —
then option 2 becomes necessary and this decision does not obstruct it: the
chokepoint takes a `PresentedCredentials`, and a cookie-derived principal would
be a third arm of the same precedence, not a rewrite. Revisit also if the
identity a proxy can assert ever needs to carry authorization with it; that
would be a different decision about where policy lives, and this one should be
reopened rather than extended.

### Development login clarification (2026-08-18)

The loopback-only proxy used by `make run` owns a deliberately small development
login: one environment/per-run password and a bounded in-memory session. This
does not reverse Decision 6. The proxy still owns authentication and sessions;
the Rust server still accepts only a proved principal assertion and owns all
authorization. The fixture has no TLS, account recovery, MFA, durable sessions,
or packaging and refuses to bind beyond loopback. It must not be deployed.

No reusable password is committed. `SENTINEL_DEV_PASSWORD` may provide one for
a run; otherwise the launcher generates one, prints it once, and keeps it only
in process memory. Production still requires the deployment's identity-aware
TLS proxy. If the product instead requires standalone local human accounts,
this ADR must be reopened before adding password or session ownership to the
surveillance server.

## Deferred — Deployment target

**Not confirmed.** The Linux distribution, whether the unit is a container or a
systemd service, the storage layout, and who performs upgrades all remain open.

`AGENTS.md` requires that this either be obtained or given a reversible
documented default. It is neither guessed nor defaulted here, because a runbook
written against a guessed target is worse than no runbook: it will be followed.

Consequently roadmap items 8 (packaging) and the target-specific runbooks in
item 9 are recorded `[!]` blocked. The target-independent runbooks — disk-full,
camera-offline, database-outage, LiveKit-outage — do not depend on it and are
written.

## Consequences

- A new `crates/auth` owns principal identity, credential verification, and
  token lifecycle. Ownership is singular: `config` owns settings, `auth` owns
  identity, `database` owns persistence, `api`/server owns translation.
- `Action` grows a camera dimension. Every mutating handler takes `Authorized`
  by value, so the compiler enumerates the sites.
- `Actor` keeps its shape and bound; its content becomes the authenticated
  principal instead of `loopback`.
- Audit extends beyond configuration to access, camera lifecycle, recording
  export, and authorization changes, reusing the `settings_audit` shape.
- The threat model (`docs/phase-8/threat-model.md`) is the prioritisation input
  for all of the above and is written first.
