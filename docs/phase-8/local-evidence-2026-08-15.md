# Phase 8 Local Evidence — 2026-08-15

Format follows `docs/phase-7/local-evidence-2026-08-15.md`: commands with actual
results, measurements separated from assumptions, and an explicit list of what
is **not** covered.

Host: macOS 24.6.0 (Darwin), Apple silicon. This is a development machine, not
the deployment target — the deployment target is unconfirmed (ADR 009).

## Commands run, and their results

### `make check`

```
cargo fmt --all --check                                              pass
cargo clippy --workspace --all-targets --all-features -- -D warnings pass
cargo test --workspace --all-features                                pass
cargo build --workspace --release                                    pass
make audit                                                           pass
```

Test totals: **429 passed, 0 failed, 32 ignored**, up from 346 passed / 32
ignored at `e65a318` — 83 new tests. The 32 ignored are unchanged and are the pre-existing
tests that need FFmpeg/libx264, MediaMTX, PostgreSQL, or a LiveKit server.

### `make audit` (new to the gate)

```
cargo deny check    advisories ok, bans ok, licenses ok, sources ok
cargo audit         0 vulnerabilities, 1 allowed warning
```

Two policy entries, both with reasons and re-check conditions in
`docs/phase-8/dependency-policy.md`:

- **RUSTSEC-2023-0071** (`rsa`, Marvin attack, no fix available) is ignored in
  `.cargo/audit.toml`. Verified rather than assumed: `cargo tree -i rsa --target
  all` reports "nothing to print", and `cargo deny check advisories` — which
  reads the resolved graph with `all-features = true` — passes. `rsa` reaches
  `Cargo.lock` only through the optional `sqlx-mysql`, which this workspace
  never builds.
- **RUSTSEC-2025-0119** (`number_prefix` unmaintained) is a warning, not a
  vulnerability.

`deny.toml` needed two deliberate allowances, both recorded in the file:
`CDLA-Permissive-2.0` (the Mozilla CA bundle via `sqlx`) and
`allow-wildcard-paths`, which required marking all sixteen workspace crates
`publish = false` — correct metadata regardless.

### `scripts/phase8/security-fixture.sh` (new)

```
EXIT=0, 18 checks passed
```

Against a real PostgreSQL. What it proves that a unit test cannot:

| Check | Result |
|---|---|
| Migrations 0001–0004 apply in order | pass |
| Plaintext credential refused by the schema | pass |
| Non-Argon2id (bcrypt-shaped) hash refused by the schema | pass |
| Token with no expiry refused by the schema | pass |
| Expiry before issuance refused by the schema | pass |
| Well-formed Argon2id token row accepted | pass |
| Partial export audit row refused by the schema | pass |
| Complete export audit row accepted | pass |
| Unknown audit category refused by the schema | pass |
| Non-fingerprint in the fingerprint column refused | pass |
| `0644` and `0600` secrets files behave as the unit tests assume | pass |
| `cargo deny check`, `cargo audit` | pass |
| `surveillance-auth`, secret store, server unit tests | pass |

The credential and expiry refusals are database-level, independent of the
application, in the same spirit as 0002's refusal of credential keys.

### `scripts/phase4/postgres-fixture.sh`

Pass, unchanged. Re-run because migrations 0003 and 0004 were added to the same
migrator.

### `scripts/phase7/config-fixture.sh`

Pass, unchanged.

## Live run against the real binary

Everything above is tests and fixtures. This section is the release binary,
running, driven with `curl`. It found three defects that the test suite did not
— all three recorded under "Regressions found" below.

Setup: `target/release/surveillance-server` against a temporary PostgreSQL 14.18,
one camera pointed at an unreachable RTSP URL (so the pipeline is permanently
reconnecting, which is itself a useful state to observe).

### No identity store configured (the Phase 2–7 posture)

| Request | Result | Expected |
|---|---|---|
| `GET /live` | `200` | yes |
| `GET /ready` | `503` | yes — camera cannot reach RTSP |
| `GET /metrics` | `200` | yes |
| `GET /api/cameras` | `200` | yes — loopback development identity |
| `GET /api/settings` | `200` | yes |
| `PUT /api/settings` | `404` | yes — `MutatingSurface` defaults off |
| `POST /api/cameras/camera-01/stop` | `200` | yes — lifecycle is not gated by the toggle |

### Identity store configured, no credential

| Request | Result |
|---|---|
| `/live`, `/metrics` | `200` — still open |
| `/ready` | `503` |
| `GET /api/cameras`, `/api/settings`, `/api/cameras/{id}/health` | `401` |
| `Authorization: Bearer garbage` | `401`, counted `reason="malformed"` |
| Well-formed but never issued | `401`, counted `reason="unknown_token"` |

### With a genuinely issued token

Minted with `cargo run -p surveillance-auth --example mint-token`, inserted, then
used:

| Request | Result |
|---|---|
| `GET /api/cameras` (administrator) | `200` |
| `GET /api/settings` (administrator) | `200` |
| `GET /api/cameras/camera-01/health` | `200` |
| `GET /api/cameras/camera-01/recordings` | `400` — missing `start`/`end`, i.e. validation, not auth |
| Same token with one character changed | `401`, counted `reason="bad_secret"` |

### Authorization properties, live

| Case | Result |
|---|---|
| Viewer scoped to `camera-02` reads `camera-01` health | `404` — existence hidden, not `403` |
| Same viewer stops `camera-02` (in scope, wrong role) | `403` — role checked before scope |
| `GET /api/cameras` as scoped viewer | `[]` |
| `GET /api/cameras` as administrator | `camera-01` present |
| Token revoked in the database, then reused | `401` immediately, no restart, counted `reason="revoked"` |
| Revocation undone, token reused | `200` |

Revocation taking effect without a restart is the property that makes a stolen
token survivable, and it is checked on every request rather than cached.

### Audit rows, live

After wiring (see regression 5 below), four requests produced four rows:

```
 category      | action                 | actor                         | camera_id | outcome | reason
 authorization | view_camera            | principal:human:live-admin    | camera-01 | granted |
 authorization | view_camera            | principal:human:scoped-viewer | camera-01 | denied  | not_in_scope
 camera        | stop_camera            | principal:human:scoped-viewer | camera-02 | denied  | unauthorized
 authorization | update_global_settings | principal:human:live-admin    |           | denied  | surface_disabled
```

## Closing P8.6

P8.6 was reported `[~]` because two of its five audited categories had no code
path that could produce them. Closing it needed two things, and building them
surfaced four more defects — every one found by running the server, none by the
test suite.

### What was added

**Export.** `/api/cameras/{id}/recordings/playback` is now authorized as
`ExportRecording` rather than `ViewRecordings`. Calling it a "view" was a
fiction: it is the only path by which media leaves this system, and the client
can save whatever it is streamed. `ExportRecording`'s required role dropped from
Operator to **Viewer** — watching recorded footage is what the Viewer role is
for, and requiring Operator would not make export rarer, only the Viewer role
useless. The separation that earns its keep is the audit row, not the role.
`stream_playback` now returns what it handed over, and a second row records
camera, exact range, and byte count.

**Identity.** A new `/api/identities` surface: list, create, mint token, revoke
token, enable, disable. This exists because "audit authorization changes" was
vacuous without it — the only way to create a principal or token was raw SQL,
which no audit can observe.

### Defects found while doing it

6. **`route_identities` returned `503` before authorizing.** An unauthenticated
   caller would learn whether an identity store was configured, and skip the
   chokepoint. The same shape as the playback defect — a return path above the
   check — caught this time by writing the table test first.

7. **Foreign-key violations were reported as `503`.** Creating a principal
   scoped to a camera that does not exist answered `identity_store_unavailable`,
   sending an operator to the database runbook for a typo. `RepositoryError`
   already classified it as `Constraint`; the handler simply was not reading the
   category. Now `400 unknown_camera_in_scope`.

8. **Revocation silently failed for freshly minted tokens.** `Timestamp` is
   whole-second; `issued_at` defaults to sub-second `clock_timestamp()`.
   Revoking within the same second stamped `revoked_at` fractionally *before*
   `issued_at`, tripping `auth_tokens_revocation_after_issue`. The API returned
   `503` and the token kept working — an operator would believe a credential was
   dead when it was live. Fixed with `GREATEST($2, issued_at)`.

9. **Every playback request failed its audit write, silently.** The decision row
   was categorised `export`, but the schema refuses an export row without range
   and byte count — which a decision row cannot have, because the bytes are not
   known until after streaming. Because an export is a *read*, the documented
   policy logged the failure and let the request proceed. The only symptom was
   an empty `access_audit` table.

   This one is worth dwelling on: the read/write asymmetry is deliberate and
   still correct, but it converts an audit-schema mismatch into a warning nobody
   reads. The `WARN audit write failed for a read` line is the only thing that
   made it findable. `no_authorization_decision_is_categorised_as_an_export`
   now asserts the invariant directly.

### Verified live, after the fixes

| Step | Result |
|---|---|
| List identities as administrator | `200`, no hashes, no token identifiers |
| Create principal scoped to an unknown camera | `400 unknown_camera_in_scope` |
| Create principal scoped to a real camera | `201` |
| Mint token | `201`, plaintext returned exactly once |
| New token used | `200` |
| Viewer lists identities | `403` |
| Revoke immediately after minting | `204` |
| Revoked token reused | `401` |
| Disable principal | `200` |
| List tokens | public halves only, with expiry and revocation |
| Playback authorized | `authorization / export_recording / granted` row |
| Playback unauthenticated | `authorization / export_recording / denied / absent` row |
| Silent audit failures in the log | **0** |

Category coverage observed live: `authorization`, `camera`, `identity`, and the
export authorization row. The **export row itself** — the one carrying byte
count — was not observed, because producing it needs real recorded segments in
the index and on disk. Its range and byte-count mapping is unit-tested instead,
and that is a weaker claim than the others here.

## What was measured versus what is assumed

**Measured** — everything in the tables above: test counts, fixture exit codes,
schema refusals against a real PostgreSQL, `cargo deny`/`cargo audit` results,
and the absence of `rsa` from the build graph.

**Assumed, and stated as such:**

- That Argon2id at m=19456/t=2/p=1 is an appropriate cost for this hardware.
  The parameters are the OWASP first-choice set, not a measurement on the
  target. `VerifyConcurrency::from_budget` derives 13 concurrent verifications
  from a 256 MiB budget — that arithmetic is tested, the *budget* is a judgement.
- That the alert thresholds (0.5 failures/s, 2m/5m/10m windows) are right. They
  are reasoned, not tuned against production series.
- That the reverse proxy will be configured to append `X-Forwarded-For` and to
  send the proof header. The server proves origin before reading either, so a
  misconfigured proxy fails closed — but nothing here tests a real proxy.
- That the subsystem priority in `AGENTS.md` (recordings > connectivity > live >
  detection) is correct. It remains unconfirmed by the product owner and the
  runbooks encode it.

## Failure matrix

| # | Case | Telemetry | Runbook | Status |
|---|---|---|---|---|
| 1 | Camera disconnect and flapping | `surveillance_camera_connection_state`, `_reconnects_total` | `camera-offline.md` | `[~]` synthetic sources only |
| 2 | Network partition | camera state, `_last_media_at` | `camera-offline.md` | `[!]` needs multi-host |
| 3 | Detector and LiveKit process failure | `surveillance_livekit_publication_state` | `livekit-outage.md` | `[~]` detector yes, LiveKit **unexercised** |
| 4 | PostgreSQL outage and recovery | `surveillance_database_available`, `_configuration_stale` | `database-outage.md` | `[x]` outage/recovery; restore **not** |
| 5 | Disk warning/critical/full/write error | `surveillance_storage_used_ratio`, `_segments_failed_total` | `disk-full.md` | `[~]` no real exhaustion run |
| 6 | Abrupt restart during finalization | partial-file discovery, reconciliation counters | — | `[~]` Phase 2 evidence, not re-qualified |
| 7 | Invalid/expired credentials, cross-camera access | `surveillance_auth_failure_total{reason}` | `camera-offline.md` | `[x]` fully tested |
| 8 | Graceful SIGTERM under load | shutdown counters | — | `[~]` combined case not qualified |
| 9 | Concurrent settings updates | `surveillance_configuration_*` | — | `[!]` **still untested**, carried from Phase 7 |
| 10 | Boundary-scoped change during recording | `surveillance_configuration_*` | — | `[!]` carried from Phase 7 |

Cases 1–6 have partial Phase 4–6 evidence. That evidence is **not** re-reported
as Phase 8's. The Phase 8 question is whether an operator can diagnose and
recover from documented telemetry, which is a different claim from whether the
system survives, and it is only genuinely answered for case 7.

## Not covered — reasons, not omissions

- **`promtool` alert-rule tests were not executed.** `promtool` is not installed
  here. `docs/phase-8/alerts.yml` (13 rules) and `alerts_test.yml` (a firing and
  a silent case for each critical alert) are written and unvalidated. The exit
  gate item stays `[!]`. An alert that has only been written is a belief.
- **No coverage-guided fuzzing.** `cargo-fuzz` needs a nightly toolchain; this
  workspace pins stable 1.88 with `unsafe_code = "forbid"`. Adding a nightly
  toolchain to the gate is its own decision. Targets are named and prioritised
  in `docs/phase-8/dependency-policy.md`. The existing hand-chosen rejection
  tests are not a fuzzer and are not reported as one.
- **No packaging.** Deployment target unconfirmed. ADR 009 declines to guess.
- **No deployment, upgrade, rollback, or backup/restore runbook.** Same blocker.
  RPO/RTO are settled; the procedures are not writable without knowing the unit.
- **No runbook was executed by a separate operator following it literally.** The
  three that were exercised were exercised by their author, who knows what they
  meant. The handoff's standard is the stronger one and is not met.
- **No LiveKit server**, so `livekit-outage.md` is written from the
  implementation and its metrics, and no step in it has been run.
- **No representative cameras and no target hardware**, unchanged from Phases
  3–7. Every camera claim here rests on synthetic sources.
- **File ownership is not enforced** on the secrets file, only mode `0600`.
  Reading the process euid needs a `libc` call that `unsafe_code = "forbid"`
  rules out. Ownership belongs with the service user that packaging creates,
  which is blocked.
- **Concurrent settings updates remain untested**, carried forward from Phase 7
  unchanged. ADR 008 forbids inferring transactional safety from code structure,
  and nothing here changes that.

## Regressions found and fixed during this phase

Two, both worth recording because both were behaviour changes rather than test
breakage:

1. **The development posture initially dropped a Phase 7 control.** With no
   identity store, the first implementation granted the development identity to
   any peer. Phase 7's `authorize` had granted only on loopback. Fixed by
   restricting the development identity to a loopback peer, so the new posture
   is never weaker than the one it replaced.
   (`health_http::tests::a_non_loopback_caller_is_refused` caught this.)

2. **A remote caller's refusal changed from `403` to `401`.** Intentional: a
   caller with no credential is unauthenticated. Phase 7 said `403` because peer
   address was the only identity signal it had. The refusal is unchanged; only
   the code is, and the test now asserts the accurate one.

3. **`GET /api/cameras/{id}/recordings/playback` was unauthenticated.** Found
   while enumerating the API surface after the phase was otherwise complete, and
   the most serious defect in this work.

   The playback path short-circuits in `handle_client` *above*
   `route_with_recordings`, because it streams a response body and cannot use
   the string-returning router. So it never reached the chokepoint — and it is
   the endpoint that streams actual footage.

   `every_protected_route_reaches_the_chokepoint` could not have caught it: that
   test enters at `route_with_recordings`, which is below the branch. **A test
   that enters below the defect proves nothing about it**, and the phase was
   reported complete on the strength of it.

   Fixed by extracting `establish_context` so both paths build the request
   context the same way, and authorizing playback as `ViewRecordings` before any
   byte is streamed. `the_playback_stream_reaches_the_chokepoint` drives
   `handle_client` over a real socket, above the short-circuit. Verified to fail
   without the fix: reverting the `authorize` call makes the unauthenticated
   request reach the playback handler.

   The general lesson, worth more than the fix: every handler entry point needs
   enumerating, not every *route*. Two entry points existed and only one was
   examined.

4. **No security metrics were exported at all.** Found by running the server:
   `/metrics` on a unit with no identity store contained none of
   `surveillance_auth_*`, `surveillance_authz_*`, or
   `surveillance_proxy_proof_*`.

   `render_auth_metrics` was called only when an authenticator existed, but two
   of those three families count **global** facts — authorization decisions and
   proof-of-origin outcomes — that have nothing to do with whether an identity
   store is configured.

   Worse than missing data: an absent series makes `rate()` empty rather than
   zero, so `ProxyOriginProofFailing` and `AuthenticationFailureSpike` would
   never have fired. The cardinality unit test passed a `&AuthMetrics` directly
   and so never exercised the `None` path.

   Fixed: all five families always render, zero-valued when no store exists. The
   test now asserts the `None` path too. 21 series confirmed live.

5. **The access audit had nothing writing to it.** `access_audit` contained
   **0 rows** after a live session. Migration `0004`, the record type, and the
   repository all shipped in Phase 8 — and P8.6 was reported `[x]` — but no
   request path ever called `record()`. The storage layer existed; the feature
   did not.

   This is a reporting error, not just a coding one: "audit at the chokepoint"
   was claimed on the strength of the schema and its unit tests. Unit tests of a
   repository prove the repository works, not that anything uses it.

   Fixed by `authorize_audited`, which wraps the chokepoint so the decision and
   its record happen in one place — the same argument that justifies a single
   `authorize`. A follow-up in the same session: denial rows initially recorded
   `action = "denied"` with no camera, losing what the caller reached for, which
   is the part an incident review needs. The action label and camera are now
   captured before `authorize` consumes the action.
