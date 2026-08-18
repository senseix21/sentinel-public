# Repository status — 2026-08-18 18:45 +06

Base: `ecd2e80` on `main`, synchronized with `origin/main` before this slice.
The work below is verified but uncommitted.

## Verified development-login slice

- `make run` now presents a SENTINEL login page instead of asserting an
  operator for every browser request.
- The loopback Python proxy owns a bounded in-memory session, validates the
  mutation origin, strips client assertion/cookie headers, throttles failures,
  and asserts only the configured principal. It remains a development fixture,
  not a production identity provider.
- A password comes from `SENTINEL_DEV_PASSWORD`; otherwise the launcher
  generates and prints a per-run value. No reusable password is committed or
  persisted.
- Seven focused proxy tests cover anonymous denial, redirect, bad origin, bad
  password, assertion stripping, authenticated access, logout/revocation, and
  cross-origin mutation refusal.
- A browser-origin regression found after the gate was fixed: both literal
  loopback names (`127.0.0.1` and `localhost`) are accepted on the configured
  port, while arbitrary origins remain forbidden. A second browser variant
  omitted both optional `Origin` and deliberately suppressed `Referer`; login
  now accepts that only with an exact allowed loopback `Host`. Authenticated
  mutations and logout still require explicit same-origin evidence. Browsers
  using the privacy value `Origin: null` receive the same login-only treatment
  on an exact loopback Host. Ten focused proxy tests pass after the corrections.
- The dashboard redirects an expired proxy session to `/login`, distinguishes
  direct `401` from `403`, and offers logout.

## Verified startup change

- The complete reconciliation pass no longer blocks process startup. The owned
  background task still starts immediately and is joined on shutdown.
- Indexed completed files compare immutable path and size evidence instead of
  spawning `ffprobe` on every periodic pass. Unindexed durable files are still
  probed before import; missing or mismatched evidence is marked inconsistent.
- New bounded metrics expose reconciliation active state and indexed skips.
- The real 419-candidate archive completed reconciliation in about 1.25 seconds
  and skipped 404 indexed files. The API answered `/live` after 9 seconds,
  compared with 183 seconds for the prior blocking pass on the same persistent
  development archive.
- Real loopback login flow returned: login `303`, authenticated API `200`,
  logout `303`, revoked session API `401`. Anonymous proxy and direct-server API
  checks both returned `401`.
- `make run` uses the development build and labels it as carrying no
  performance claim. `make run-release` preserves the optimized qualification
  path.

## Validation

- `scripts/phase4/postgres-fixture.sh`: passed, including PostgreSQL repository,
  real FFmpeg playback, reconciliation idempotence without a second ffprobe,
  retention, and database-outage recovery.
- `make check`: passed after one Clippy-only correction. Rust formatting and
  workspace Clippy were clean; 509 tests passed with 34 ignored; release build
  succeeded; all four `cargo deny` checks passed; `cargo audit` scanned 461
  dependencies against 1217 advisories with the existing allowed
  `number_prefix` warning.
- The final Ctrl-C presentation-only trap correction passed `bash -n` and does
  not change child ownership or the validated runtime path.

## Uncommitted files

This slice includes the previously preserved `AGENTS.md` gRPC policy change.
Nothing is staged or committed.
