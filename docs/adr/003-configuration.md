# ADR 003: Configuration

Status: proposed

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

- Production secret store and rotation mechanism.
- Persistence transaction boundary and optimistic concurrency/versioning.
- Which settings are operator-editable through the first API.
- Rollback behavior when runtime application fails after persistence.

## Reversal criteria

Split configuration into a remote control-plane service only when multi-node coordination is a confirmed deployment requirement.

