# Architecture handoff

Accepted: [ADR 016](../../docs/adr/016-face-only-live-detection.md) records direct
full-frame face inference and one selected mode per camera. The existing detector
owner retains decoder, sampling, freshness queue and blocking inference ownership.
Separate face events and `face/v1` packets preserve person semantics. Maximum
64 faces / 16 KiB packets; overlays hold one latest result, reject old sequences
and expire with a validated per-camera duration (default three seconds).

Recording/live remain independent of model startup failures. Face mode has no
tracker, recognition, embeddings or identity state. Original recordings contain
no overlays. Core safe defaults remain person/fixture; the development launcher
explicitly selects real face mode. Previous ownership and API architecture:
[earlier snapshot](../2026-10-04-2021/architecture.md).
