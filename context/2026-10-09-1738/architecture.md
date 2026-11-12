# Proposed architecture

[Specification](../../docs/person-presence/SPEC.md) and
[proposed ADR 017](../../docs/adr/017-person-presence-with-face-overlays.md): one
camera ingest and detection decoder, immutable sampled RGB, independently owned
bounded person/face lanes, time-based presence states and separate snapshots.
Only observed faces draw boxes; back-facing presence uses person evidence.
These are design proposals, not running implementation.
