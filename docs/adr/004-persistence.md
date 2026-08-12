# ADR 004: Recording Persistence and Container Lifecycle

Status: accepted for the Phase 4 indexed-recording slice; production deployment and storage durability remain to be qualified

Date refreshed: 2026-08-11

## Context

Continuous video dominates storage volume while cameras, segment intervals, events, ROI, and settings require indexed queries. Database outages must not immediately stop media preservation. Process interruption must not cause an active segment to masquerade as completed.

Crash experiments found that fragmented MP4 and Matroska retained decodable media for both H.264 and H.265 after `SIGKILL`; fragmented MP4 also retained a known duration for completed fragments. Classic MP4 was not probeable after interruption for either codec.

## Decision

- Store finalized video segments in filesystem-backed storage using camera/UTC-derived paths.
- Use fragmented MP4 for the H.264 MVP recording path, with fragments cut at keyframes.
- Write active files as `.partial.mp4` and never return/index them as completed recordings.
- On normal completion: flush/finalize, apply the configured durability sync, validate essential media metadata, and atomically rename within the same filesystem to `.mp4`.
- Persist completed recording metadata only after durable rename succeeds.
- Store metadata and indexes in PostgreSQL; never store video blobs in PostgreSQL.
- Model segment lifecycle explicitly: open, finalizing, completed, incomplete/recovered, quarantined, and failed.
- Reconcile partial/orphan files and missing metadata at startup and continuously as required.
- Query normal playback ranges from indexed metadata, not filesystem scans.
- Use a bounded durable reconciliation strategy while PostgreSQL is unavailable.

### Phase 4 persistence boundary

- Use PostgreSQL 14 or newer. Phase 4 development is qualified against the
  repository-owned fixture version recorded by the fixture output; the
  production version, topology, TLS, backups, and migration operator remain
  deployment gates.
- Use SQLx's Tokio/Rustls PostgreSQL support, bounded `PgPool`, and embedded
  forward-only migrations. SQLx owns SQL/driver conversion inside `database`;
  validated lifecycle and range policy remain in `storage`.
- The provisional pool bound is 8 connections with a 5-second acquisition
  timeout. Both are versioned configuration applied at process restart, not
  hidden driver defaults. Revisit from measured query/write concurrency.
- A recorder completes durable rename, directory sync, and successful probe
  before emitting a bounded lifecycle fact. The application metadata writer,
  never the recorder, persists that fact. PostgreSQL is therefore outside the
  media-write critical path.
- Metadata delivery uses a bounded per-process channel (provisional capacity:
  256 facts). A full channel is a visible critical degradation; it cannot
  silently discard a fact or block an unrelated camera indefinitely. Durable
  final files remain the recovery journal.
- Reconciliation uses deterministic bounded pages (provisional maximum: 256
  candidates/pass), continuous retries with bounded exponential backoff and
  jitter, and idempotent insert-or-confirm semantics. The filesystem cursor is
  restartable work bounding, not a claim of transactional delivery.
- Provisional outage limits are 10,000 unindexed durable files, 100 GiB, or 24
  hours oldest age. Crossing a limit changes storage readiness to critical and
  requires operator action; exact stop/retention behavior remains a product
  decision and is not implemented implicitly.
- Completed replay succeeds only when immutable evidence matches. Conflicting
  identity/path/time/media evidence becomes an explicit invariant conflict.
- Retention claims work in PostgreSQL before touching bytes:
  `completed -> deletion_pending`, then safely resolves beneath the configured
  root without following symlinks, unlinks and syncs the directory, then marks
  `deleted`. Failure leaves recoverable deletion state. Provisional batch bounds
  are 100 files and 10 GiB per pass.

### Transaction and concurrency rules

- Camera upsert and recording insert/confirm occur in one database transaction.
- Recording identity and durable locator are unique. Database checks duplicate
  Rust invariants because migrations and other maintenance clients remain trust
  boundaries.
- Status updates use legal expected-state predicates. Zero-row updates are
  reread and classified as idempotent completion, concurrent progress, or typed
  conflict; last-write-wins lifecycle mutation is forbidden.
- Normal overlap queries are camera scoped, completed-only, half-open, bounded,
  and ordered by `(started_at, recording_id)`. The matching partial index is
  qualified with `EXPLAIN` in the PostgreSQL fixture.
- Cancellation drops the query future and returns the connection to the pool;
  owned writer/reconciliation/retention tasks retain explicit shutdown and join
  policies.

### Initial acceptance tests

- `completed_metadata_rejects_invalid_evidence`
- `lifecycle_transitions_are_idempotent_and_never_regress_completed`
- `overlap_predicate_matches_nonempty_intersection_for_bounded_domain`
- `migrations_apply_from_empty_and_are_rerunnable`
- `database_constraints_reject_invalid_completed_rows`
- `overlap_query_uses_completed_camera_time_index`
- `insert_or_confirm_is_idempotent_and_conflicts_are_typed`
- `database_outage_preserves_recording_and_reconciles_after_restart`
- `retention_interruption_leaves_recoverable_deletion_pending`

The first three are pure contract tests. The next three are the initial real
PostgreSQL gate. Later names define package acceptance before implementation.

### Reproducible PostgreSQL fixture

`scripts/phase4/postgres-fixture.sh` initializes a temporary local PostgreSQL
cluster with trust authentication bound only to a temporary Unix socket, applies
the checked-in migrations with `ON_ERROR_STOP`, reruns them, exercises database
constraints and overlap predicates, records server/version and query-plan
evidence, then stops the owned server. It uses no DSN or checked-in secret.

### Open decisions and assumptions

The 30-day age policy remains a provisional accepted MVP profile, but protected
coverage, warning/critical/emergency watermarks, hysteresis, outage exhaustion,
and deletion-failure escalation require product/deployment approval.

### Reversible Phase 4 playback policy

- The first byte-delivery mode is an HTTP streamed fMP4 response assembled from
  every indexed segment covering the requested half-open interval. Download
  disposition and HTTP byte ranges are deferred.
- Accuracy is explicitly `segment-aligned`: a mid-segment request includes that
  segment from its first available sample and may overshoot both requested
  boundaries. It is not advertised as frame accurate.
- The assembler passes a bounded ffconcat manifest over standard input to one
  owned FFmpeg child and performs H.264 stream-copy remuxing to fragmented MP4.
  It neither decodes nor transcodes.
- Gaps, overlaps, unordered evidence, cross-camera evidence, missing files, and
  corrupt files fail explicitly; they are never hidden by returning a partial
  interval.
- Requests are limited to 24 hours and 1,000 indexed segments. The sequential
  loopback adapter permits one active client, buffers at most 64 KiB of media,
  applies a five-second I/O/startup boundary, and cancels the child when the
  client stalls or disconnects.

Product approval is still required for boundary overshoot, startup objective,
viewer concurrency, download/range behavior, and any frame-accurate/transcoding
mode. These defaults are deliberately replaceable behind the playback-plan and
assembler boundaries.

## Failure ordering

No transaction can atomically cover PostgreSQL and a local file rename. The source of truth during reconciliation is therefore explicit lifecycle evidence:

1. A `.partial.mp4` file is never completed, regardless of database state.
2. A validated final `.mp4` without metadata is an orphan eligible for metadata reconstruction.
3. Completed metadata pointing to a missing file is inconsistent and must be surfaced, not silently omitted.
4. Retention marks/deletes metadata and files through a recoverable state transition rather than an untracked unlink.

## Alternatives

### Classic MP4

Rejected for continuous segment writing because interrupted outputs lacked final metadata and could not be probed.

### Matroska

Retained as a recovery-oriented fallback. Interrupted synthetic outputs decoded, but duration was unavailable and the MVP playback path benefits from MP4-family compatibility. Reconsider if real-camera muxing or durability tests favor it materially.

### PostgreSQL video blobs

Rejected because video volume, backup behavior, and write amplification make the metadata database the wrong ownership boundary.

### Filesystem only with directory scanning

Retained only as a reconciliation/disaster-recovery input. It cannot meet indexed time-range query and retention requirements as the normal read path.

### Object storage immediately

Deferred. It adds deployment and consistency concerns before the 10-camera local-storage requirement demonstrates a need.

## Consequences

- The recorder must understand safe fragmentation/finalization rather than treating arbitrary byte writes as recordings.
- Partial files can preserve some media but require explicit incomplete metadata and validation.
- Fragment/GOP policy bounds likely crash loss but must be measured on real streams/storage.
- Browser support, fragmented-MP4 concatenation, time-range playback, and H.265 compatibility still require qualification.

## Evidence

See `docs/phase-0/container-finalization.md` and `scripts/phase0/container-crash-fixture.sh`.

## Reversal criteria

Choose Matroska, another container, or object storage only when representative-camera, playback, power-loss, or measured scale evidence demonstrates a material advantage and the replacement preserves explicit lifecycle/reconciliation semantics.

Replace SQLx if cancellation, migration, observability, or measured pool behavior
cannot meet this contract. Replace PostgreSQL only if measured operation at the
qualified workload shows it is the limiting boundary and the replacement
preserves constraints, indexed overlap behavior, and recoverable transitions.
