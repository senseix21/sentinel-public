# Phase 8 Threat Model

Written 2026-08-15 against `e65a318`. Roadmap Phase 8 instruction 1.

Scope: the single-machine 10-camera unit. PostgreSQL is local to the unit
(ADR 008), the API is currently loopback-only, and the intended end state is a
private-network listener behind a reverse proxy (ADR 009 Decision 3).

This is a prioritisation input, not paperwork. Every Phase 8 package below traces
to at least one threat here, and every threat either names the package that
mitigates it or is recorded as accepted risk with a reason.

## Assets, in priority order

The priority is `AGENTS.md`'s subsystem ordering, which remains an assumption
pending product-owner confirmation.

1. **Recorded footage** — the product. Loss is unrecoverable; disclosure is a
   privacy breach with legal weight.
2. **Camera credentials** — compromise yields live and historical access to the
   physical space, and is reusable against the cameras directly.
3. **Camera availability** — a blind camera during an incident is the failure the
   system exists to prevent.
4. **The recording index** — loss is recoverable by filesystem reconciliation
   (that path exists); disclosure reveals occupancy patterns.
5. **Configuration and audit integrity** — the ability to detect and reconstruct
   what changed.
6. **Live view and detection** — degradable by design.

## Adversaries

| | Position | Realistic capability |
|---|---|---|
| A1 | Unauthenticated on the LAN | Reach the API and RTSP; scan; replay captured traffic |
| A2 | Authenticated low-privilege operator | Everything their role permits, plus whatever the authorization check misses |
| A3 | Compromised camera | Send arbitrary bytes into the RTSP/H.264 parsers as a trusted-by-position peer |
| A4 | Local unprivileged user on the host | Read world-readable files; connect to loopback; read `/proc` |
| A5 | Insider with legitimate access | Export footage, delete recordings, change retention — all "authorized" actions |
| A6 | Network-path attacker | Observe and modify unencrypted traffic between proxy, server, cameras, database |

Explicitly out of scope: a host-root attacker (game over, and no in-process
control is honest against it), physical access to the machine or the cameras,
and supply-chain compromise of the Rust toolchain.

---

## B1 — Camera ↔ server

**Crosses** RTSP/TCP control and interleaved H.264 media, inbound; camera
credentials, outbound.

**Trusted today** The camera, completely. It is a trusted-by-position peer whose
bytes reach `crates/camera`'s RFC 6184 depayloader and access-unit assembler.

| Threat | Adversary | Impact | Control |
|---|---|---|---|
| Malformed RTP/H.264 crashes or corrupts the ingest task | A3 | Camera-scoped outage; possible memory-safety issue at an FFI edge | **Partial.** Bounded depayloading rejects malformed, truncated, oversized, and discontinuous payloads (Phase 0/2 tests), and failure is camera-isolated (Phase 3). **P8.10 adds fuzzing** — that parser is the highest-value target in the tree. |
| Credentials observed on the wire | A6 | Asset 2 compromised | **Accepted risk.** RTSP Basic/Digest over TCP without TLS is what the camera profile supports. Mitigated by segmenting cameras onto their own network — a deployment control, recorded here because no code change fixes it. |
| Compromised camera pivots to the server | A3 | Host compromise | Ingest holds no shell path; the process runs as a non-root user with narrow filesystem access — **specified in P8.8, which is blocked**, so this is currently unenforced. |
| Camera flapping exhausts reconnect resources | A3 | Denial of the camera and its neighbours | **Controlled.** Bounded exponential backoff with jitter, reset only after a stable interval (Phase 3). Failure-matrix case 1 re-qualifies it from the operator's side. |

**Residual** Unencrypted camera credentials. Network segmentation is the only
control, and it is not ours to enforce.

## B2 — Client ↔ API

**Crosses** Every request. Today: unauthenticated, loopback-only.

**Trusted today** Anything that reaches the socket.

| Threat | Adversary | Impact | Control |
|---|---|---|---|
| Unauthenticated read of camera health, recording search, settings | A1, A4 | Occupancy patterns disclosed; blind cameras identified | **P8.3** authenticates all API reads (ADR 009 Decision 2). |
| Unauthenticated mutation — stop a camera, change retention | A1, A4 | Assets 1 and 3 | Partially held today by `MutatingSurface` defaulting off; **P8.2 + P8.3** make it an identity decision rather than a deployment toggle. |
| Operator reaches a camera outside their scope | A2 | Asset 1 | **P8.3** camera-scoped `Action`. Cross-camera denial answers `404`, since `403` confirms existence. |
| Viewer token minted without authenticating the requester | A1 | Live view of any camera | **P8.3.** Token issuance becomes a protected, camera-scoped action. |
| Credential brute force or stuffing | A1 | Asset 2 | **P8.2** Argon2id and mandatory expiry; **P8.7** authentication-failure-spike alert. Rate limiting is *not* implemented — see residual. |
| Malformed request crashes the hand-rolled HTTP parser | A1 | Loss of the API task | Contained: the API is an independently joined sibling and its failure does not cancel recording (Phase 3). **P8.10** fuzzes request-line, header, range, and time-query parsing. |
| Token stolen from a log, proxy access log, or shell history | A1, A4 | Full impersonation | **P8.2** — the token never appears in `Debug`/`Display`/`Serialize`; it is sent in a header, not a query string, so it stays out of access logs. Expiry bounds the window. |

**Residual** No rate limiting on authentication. Argon2id makes online guessing
expensive and the alert makes it visible, but a determined A1 gets unlimited
attempts. Recorded as accepted for Phase 8; the mitigation is small and belongs
with a considered lockout policy rather than bolted on here.

## B3 — Server ↔ database

**Crosses** Settings, audit rows, recording index, principals and token hashes.

**Trusted today** Local PostgreSQL over loopback; credentials from the
environment.

| Threat | Adversary | Impact | Control |
|---|---|---|---|
| Read of the token table | A4 | Impersonation if hashes are weak | **P8.2** Argon2id with per-row parameters. Reading the table yields no usable token. |
| Read of camera credentials from the database | A4 | Asset 2 | **P8.5** keeps secrets out of the database entirely — file-backed, `0600`. This is a reason for Decision 4 beyond convenience. |
| Audit tampering to hide an action | A5 | Asset 5 | **Partial.** `settings_audit` commits in the same transaction as the change, so an action cannot be committed without its record. A5 with database access can still delete rows afterwards; append-only enforcement and off-host shipping are **accepted risk** for Phase 8. |
| Database outage stops recording | A1 causing load, or operational | Asset 1 | **Controlled.** Media preservation does not depend on the index; reconciliation reconstructs it (Phase 4). Failure-matrix case 4. |
| SQL injection | A2 | Assets 3, 5 | Parameterised queries throughout `crates/database`. No dynamic SQL construction exists. |

**Residual** Audit is tamper-*evident within a transaction*, not tamper-proof
against an adversary with database write access.

## B4 — Server ↔ filesystem

**Crosses** Recording segments, the secrets file, configuration.

| Threat | Adversary | Impact | Control |
|---|---|---|---|
| World-readable footage | A4 | Asset 1 disclosed | Storage-owned paths are contained beneath the configured root and traversal is rejected (Phase 2). Directory *permissions* are a P8.8 concern and therefore **unenforced today**. |
| World-readable secrets file | A4 | Asset 2 | **P8.5** refuses to load a file wider than `0600` and reports the observed mode. |
| Path traversal via a camera ID or query parameter | A1, A2 | Arbitrary read/write | **Controlled.** `RecordingLayout` rejects paths outside its root before any I/O; `CameraId` is validated. |
| Disk exhaustion stops recording | any | Asset 1 | **Controlled.** Watermarks with hysteresis and retention from indexed metadata (Phase 4). Failure-matrix case 5; runbook in P8.9. |

## B5 — Server ↔ LiveKit

| Threat | Adversary | Impact | Control |
|---|---|---|---|
| LiveKit API secret disclosed | A4 | Anyone can mint viewer tokens | **P8.5** file-backed secret. |
| Viewer token grants more than intended | A2 | Live view of other cameras | Tokens are already subscribe-only and camera-scoped by room; **P8.3** authenticates issuance. |
| LiveKit outage cascades into recording | operational | Asset 1 | **Controlled by design** — independent consumer (Phase 5). Failure-matrix case 3. |

## B6 — Dashboard/metrics ↔ server

| Threat | Adversary | Impact | Control |
|---|---|---|---|
| `/metrics` read by anyone on the network | A1 | Camera count, uptime, failure rates, occupancy inferable from detection counters | **Accepted, bounded.** ADR 009 Decision 2 keeps `/metrics` open but moves it to a separate bind so it can be kept off the routable interface. Revisit if a series ever identifies a person or an event time. |
| Metric cardinality explosion | A2 | Memory exhaustion of the metrics registry | **Controlled.** Only validated camera IDs and fixed labels (Phase 3). **P8.7** extends the cardinality test to the new auth series; principal ID is deliberately never a label. |

## B7 — Operator ↔ deployment

| Threat | Adversary | Impact | Control |
|---|---|---|---|
| Insider exports footage | A5 | Asset 1 leaves the system's control permanently | **P8.6.** Export audit records principal, camera, exact range, byte count, outcome. Detection, not prevention — the honest description. |
| Insider shortens retention to destroy evidence | A5 | Asset 1 | **P8.6** audits the change with before/after. Retention is boundary-scoped so the change is visible before it takes effect. |
| Spoofed forwarded header grants proxy-level trust | A1 | Full bypass of B2's controls | **P8.4.** Peer allowlist **and** constant-time shared-secret header, both required before any forwarded header is read. This is the single highest-severity design risk in Phase 8: every trusted-header design without proof of origin is an authentication bypass. |
| Non-loopback bind before authentication exists | operator error | Everything in B2 | **P8.4.** The unlock is a validation rule, not documentation: a non-loopback bind without a trust boundary and an enabled principal is a startup error. |
| Upgrade loses data or cannot be rolled back | operational | Assets 1, 3 | **P8.9, partially blocked.** RPO/RTO settled (one segment / 15 minutes); the procedure needs the deployment target. |
| Vulnerable dependency or system library | A1, A3 | Varies | **P8.10.** `cargo deny` in the gate, documented FFmpeg/RTSP patching policy. |
| Misconfigured proxy asserts an identity it should not | operator error | Full impersonation of any provisioned principal | **ADR 009 Decision 6, accepted not mitigated.** No code on this side can tell a correctly-proved assertion from a correctly-proved lie. What is controlled: the assertion is discarded unless proof of origin holds (two independent conditions, neither client-settable); the proxy asserts *who*, never *what they may do*, so role and camera scope still come from the local table; and an asserted principal that is unknown or suspended here is refused. What remains the operator's: allowlisting only peers they control, and keeping the proof secret secret. |
| Proxy authenticates a person this system never provisioned | operator error | None — refused | **ADR 009 Decision 6.** Counted as `surveillance_auth_failure_total{reason="unknown_principal"}`. A sustained non-zero rate means the proxy's user directory and this system's principal table have diverged, which is a provisioning gap worth alerting on rather than an attack. |
| Development login exposed beyond loopback | operator error | Shared local password could grant a SENTINEL session | **Controlled for the fixture boundary.** The development proxy binds a literal loopback address, keeps its password/session tokens in memory, bounds sessions and attempts, strips client assertions, validates mutation origins, and is not packaged. It has no TLS, MFA, recovery, or production claim. |

---

## Prioritisation

Severity × exposure, which is the order the plan implements:

1. **B7 forwarded-header spoofing** — total bypass, and the mistake is easy to
   make. P8.4.
2. **B2 unauthenticated mutation and read** — currently held only by loopback
   binding and a default-off toggle. P8.2, P8.3.
3. **B4/B3 secret exposure at rest** — cheap to fix, permanent if wrong. P8.5.
4. **B1 malformed media parsing** — a compromised camera is a realistic
   position and the parser is the most exposed attack surface in the tree.
   P8.10.
5. **B7 insider export** — no prevention is possible; detection is the control.
   P8.6.
6. **Everything operational** — failure matrix and runbooks. P8.7, P8.9.

## Accepted risks

Carried deliberately, each with a reason and a revisit condition.

| Risk | Reason | Revisit when |
|---|---|---|
| Camera credentials unencrypted on the wire | The camera profile offers no better; network segmentation is a deployment control | Cameras support RTSPS |
| `/metrics` unauthenticated | Scrapers are unauthenticated by convention on a private network; separate bind bounds the exposure | The collector moves off the unit, or a series identifies a person or event time |
| No authentication rate limiting | High-entropy tokens plus Argon2id plus a failure-spike alert; lockout policy deserves its own design | Passwords are introduced, or the alert fires in practice |
| Audit deletable by database write access | Same-transaction commit makes it tamper-evident, not tamper-proof; append-only and off-host shipping are their own project | Compliance requires it, or an insider incident occurs |
| Host-root attacker | No in-process control is honest against root | Never — this is a boundary, not a gap |
| Filesystem and process permissions unenforced | Depends on the packaging unit, which is unconfirmed | The deployment target is confirmed and P8.8 unblocks |
