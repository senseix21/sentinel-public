# ADR 003: Configuration

Status: proposed; the "Open details" below are superseded by ADR 008 except for
the production secret store and rotation mechanism, which remains open

## Context

Retention, segmentation, sampling, reconnect behavior, capacities, storage watermarks, camera sources, and publisher behavior vary by deployment. The prototype's CLI/environment values and placeholder APIs do not provide validation, persistence, atomic change, or secret safety.

## Proposed decision

- Configuration is represented by validated domain types; invalid values cannot become active.
- Version-controlled defaults are non-secret and safe for local development.
- Persisted deployment settings survive restart.
- Environment/secret-store inputs supply secrets and narrowly scoped deployment overrides.
- Controlled APIs update permitted settings and emit audit/domain events without returning secret values.
- Multi-field changes validate and commit transactionally or fail without partial application.
- Every dynamic setting declares its application boundary: immediate, next segment, next reconnect, or restart.

## Proposed precedence

```text
compiled/versioned defaults < persisted deployment settings < explicit startup overrides
```

Secret material is referenced/injected separately rather than serialized into ordinary settings responses. Exact secret storage remains a deployment decision.

## Alternatives

- Hardcoded constants: rejected for operational values expected to change.
- Environment variables only: insufficient for safe dynamic administration and persisted camera/rule configuration.
- Arbitrary key/value settings without typed schemas: rejected because validation and application semantics become implicit.

## Open details

Phase 6 detection settings (`enabled`, encoded queue items, sample FPS, maximum
timestamp gap, decoded-frame byte bound, decoder executable, and decode timeout)
are transactionally validated with the camera pipeline memory budget and apply
on pipeline start/restart. They never mutate an active decoder or queue in place.
The reversible defaults are enabled, two encoded access units, 5 FPS, a two-
second discontinuity gap, exact configured RGB24 frame bytes, `ffmpeg`, and a
two-second response bound. Environment overrides are restart-applied.

- Production secret store and rotation mechanism. **Still open.** ADR 008 decides
  only the in-process representation (`Secret<T>`) and the audit representation
  (salted SHA-256 fingerprint); where secrets live in production is a Phase 8
  deployment decision.
- ~~Persistence transaction boundary and optimistic concurrency/versioning.~~
  Decided in ADR 008: settings and audit commit in one transaction. Optimistic
  concurrency is explicitly deferred, with the reversal criterion recorded there.
- ~~Which settings are operator-editable through the first API.~~ Decided in
  ADR 008 and enumerated in `docs/phase-7/contracts.md`: operational settings
  only, never bootstrap settings.
- ~~Rollback behavior when runtime application fails after persistence.~~ Decided
  in ADR 008 and specified in `docs/phase-7/contracts.md`.

## Reversal criteria

Split configuration into a remote control-plane service only when multi-node coordination is a confirmed deployment requirement.
