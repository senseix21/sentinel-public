# Decisions and Open Questions — 2026-08-11 21:44 +06

## Accepted engineering decisions

- PostgreSQL 14+ with SQLx Tokio/Rustls is the reversible Phase 4 adapter choice.
- Provisional restart-applied pool bounds are 8 connections and 5 seconds to
  acquire; change only from workload evidence.
- Recorder completion remains rename/sync/probe before lifecycle delivery;
  database persistence follows in an application owner.
- Durable files are the recovery journal. A bounded in-memory queue is not
  described as durable delivery.
- Normal overlap selection is camera-scoped, completed-only, half-open, bounded,
  and ordered by `(started_at, recording_id)`.
- Retention must claim `deletion_pending` before safe unlink and finalize only
  after directory sync; implementation follows indexed query/reconciliation.

## Provisional, reversible bounds

- Metadata channel: 256 facts.
- Reconciliation page: 256 candidates.
- Outage critical limits: 10,000 files, 100 GiB, or 24-hour oldest age.
- Retention pass: 100 files and 10 GiB.

These are operational starting points, not accepted production capacity.

## External/open gates

- Production PostgreSQL version/topology/TLS/backups/migration owner and secrets.
- Protected footage window, watermarks/hysteresis, and exhaustion behavior.
- Playback stream/download, keyframe accuracy/overshoot, transcode permission,
  request bounds, startup objective, and viewer concurrency.
- Authorization policy and production storage/filesystem durability.
