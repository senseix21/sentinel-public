# Next actions — 2026-08-18 18:45 +06

## 1. Review and commit the development-login/startup slice

Keep the pre-existing `AGENTS.md` gRPC policy separable if desired. The login,
startup, reconciliation, dashboard, fixture, ADR, threat-model, Makefile, and
context changes form one coherent reviewed slice. Re-run `bash -n` after any
shell presentation edits.

## 2. Confirm the production identity provider

ADR 009 still requires an identity-aware TLS reverse proxy. Choose the actual
OIDC/SSO or standalone-account deployment before packaging. Do not promote the
loopback Python password fixture into production.

## 3. Design bounded archive scrubbing

Reconciliation now checks index/filesystem agreement and no longer doubles as
a full historical probe. Specify a separately scheduled, bounded integrity
scrubber if periodic content verification is required: workload, cadence,
I/O budget, failure transition, recovery path, and metrics remain decisions.

## 4. Continue the existing Phase 9 actions

Viewer-visible reconnect/first-picture timing, dashboard playback rendering,
representative cameras, alert execution, LiveKit outage recovery, recording
evidence recovery semantics, and target-hardware 10/20-camera qualification
remain open as listed in the previous snapshot.
