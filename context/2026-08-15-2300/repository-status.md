# Repository Status — 2026-08-15 23:00 +06

Branch `phase-8-handoff-and-config-metrics`, on top of `e65a318`.

## What changed in this session

Phase 8 implemented from its handoff: threat model, identity and authentication,
camera-scoped authorization, the reverse-proxy trust boundary, secret storage,
access audit, security metrics and alert rules, dependency policy, and four
runbooks.

### New

| Path | What |
|---|---|
| `crates/auth` | Principals, roles, camera scope, bearer tokens, Argon2id verification, injected clock. Owns identity only. |
| `crates/config/src/secret_store.rs` | Permission-enforced secrets file with per-key environment override. |
| `crates/database/src/auth.rs` | `TokenStore` against PostgreSQL. |
| `crates/database/src/access_audit.rs` | Access, camera, export, and identity audit. |
| `apps/server/src/authenticate.rs` | Bearer extraction, off-thread bounded Argon2id, auth metrics. |
| `apps/server/src/trust_boundary.rs` | Proof of origin: peer allowlist plus constant-time shared secret. |
| `apps/server/src/security_config.rs` | The non-loopback bind unlock, as validation. |
| `migrations/0003_auth.sql`, `0004_access_audit.sql` | Schema-level credential and audit controls. |
| `docs/adr/009-*.md` | The five decisions the handoff demanded before code. |
| `docs/phase-8/` | SPEC, PLAN, threat model, dependency policy, alerts, evidence, 4 runbooks. |
| `deny.toml`, `.cargo/audit.toml`, `scripts/phase8/security-fixture.sh` | Dependency policy and the new fixture. |

### Changed

- `apps/server/src/authorize.rs` — the seam Phase 7 built now has a body.
  `Action` gained a camera dimension and eleven variants; `Denied` gained
  `Unauthenticated` and `NotInScope`.
- `crates/common/src/secret.rs` — moved from `crates/config`, which
  re-exports it. Secret carriage became cross-domain when identity needed it;
  pointing `auth` at the whole configuration domain would have inverted the
  dependency direction.
- All sixteen workspace crates are now `publish = false` (correct metadata, and
  required by `cargo deny`).

## Verified

```
make check                          pass   426 passed, 0 failed, 32 ignored
make audit                          pass   deny + audit clean
scripts/phase8/security-fixture.sh  pass   18 checks against real PostgreSQL
scripts/phase4/postgres-fixture.sh  pass
scripts/phase7/config-fixture.sh    pass
```

Full detail, including what is **not** covered, in
`docs/phase-8/local-evidence-2026-08-15.md`.

## Still true, and important

- **Detection still runs `DeterministicFakeInference`.** Unchanged. There is no
  production model and no accuracy claim.
- **`crates/api`, `crates/analytics`, `crates/rules`, `apps/worker` are still
  empty stubs.** The HTTP surface is still `apps/server/src/health_http.rs`.
- **No representative cameras, no target hardware, no LiveKit server.**

## Phase 8 is not closed

Four exit-gate items cannot be met here:

1. **Packaging** — deployment target unconfirmed. ADR 009 declines to guess.
2. **Deploy/upgrade/rollback/backup runbooks** — same blocker.
3. **Alert testing** — `promtool` absent; rules written, unvalidated.
4. **Fuzzing** — needs a nightly toolchain, which is its own decision.

Plus: no runbook has been executed by a separate operator following it
literally, and Phase 7's concurrent-settings-update case is still untested.
