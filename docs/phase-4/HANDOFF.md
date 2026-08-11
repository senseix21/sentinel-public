# Phase 4 Handoff — Indexed Recording Retrieval, Reconciliation, and Retention

Status: ready to implement after Phase 3 local gate

Baseline date: 2026-08-11

## Mission

Turn the durable camera-scoped fMP4 files produced by Phase 3 into an indexed,
queryable, and safely retained recording system. PostgreSQL owns recording
metadata and query indexes; the filesystem remains the owner of video bytes.
Database interruption must not immediately stop recording, and no database row
may make a partial, absent, or unvalidated file look playable.

This phase extends the current recorder/storage ownership model. It must not
move video into PostgreSQL, make filesystem scans the normal query path, expose
internal paths through the API, or couple database availability to camera
connectivity.

## Read first

Read these sources in order before changing persistence or playback behavior:

1. `AGENTS.md`
2. `context/README.md` and its latest snapshot
3. `ROADMAP.md`, Phase 4
4. `docs/requirements.md`
5. `docs/adr/004-persistence.md`
6. `docs/adr/005-fragmented-mp4-muxer.md`
7. `docs/adr/006-multi-camera-supervision.md`
8. `crates/recorder/src/runtime.rs`
9. `crates/recorder/src/segment.rs`
10. `crates/storage/src/segment_store.rs`
11. `crates/storage/src/reconciliation.rs`
12. `crates/storage/src/filesystem.rs`
13. `apps/server/src/pipeline.rs`
14. `apps/server/src/camera_supervisor.rs`
15. `apps/server/src/health_http.rs`
16. `crates/database/` and `migrations/`

Read `analysis.md` only when resolving an architectural ambiguity. It is the
target proposal, not evidence of current implementation.

## Verified starting point

Phase 3 currently provides:

- independently owned camera-to-recorder pipelines with joined shutdown;
- validated `CameraId` and `RecordingId` domain identifiers;
- deterministic camera/UTC paths beneath one configured recording root;
- H.264 fMP4 files created as `.partial.mp4`, incrementally synced, atomically
  renamed, probed, and emitted as truthful completed lifecycle facts;
- typed failed-segment facts retaining the partial path and failure stage;
- actual start/end, codec, dimensions, duration, size, and durable path evidence;
- a bounded recorder lifecycle-event channel drained by each pipeline owner;
- bounded startup discovery of partial and durable recording candidates;
- loopback camera API, per-camera health, recorder counts, and metrics;
- deterministic Phase 2 and Phase 3 fixtures covering crash evidence, multiple
  playable segments, camera isolation, and multi-camera shutdown.

The final Phase 3 local gates passed: format, workspace Clippy with warnings
denied, 79 ordinary tests, all doc tests, release build, the Phase 2 RTSP/crash
fixture, and the ten-source Phase 3 synthetic fixture. The representative-camera
soak remains an explicit external Phase 3 gate and does not block locally
implementable Phase 4 persistence work.

## Current limitations that Phase 4 owns

- `crates/database` contains placeholder modules and no dependencies or adapters.
- `migrations/` contains only a README; no schema is applied or versioned.
- `crates/storage/src/metadata.rs` and `recordings.rs` are placeholders.
- `crates/recorder/src/retention.rs` is a placeholder.
- completed/failed lifecycle events are logged and counted but not persisted.
- startup reconciliation only discovers bounded filesystem candidates; it does
  not compare them with database state, probe/import orphans, or update status.
- reconciliation scans the configured root once during process startup and is
  not a continuous, resumable, or database-aware workflow.
- no persistent camera row or recording lifecycle state exists.
- no indexed overlap query, recording-search API, multi-segment retrieval, or
  playback boundary policy exists.
- no storage watermark, retention protection, deletion workflow, or deletion
  failure recovery exists.
- database health/readiness, pool bounds, query latency, reconciliation backlog,
  and retention metrics do not exist.
- the current server configuration has no PostgreSQL connection boundary.

Verify each relevant fact before relying on it because the repository will
continue to evolve. Never present an unrun migration or unqueried repository as
working.

## Non-negotiable invariants

1. Video bytes remain filesystem-owned; PostgreSQL stores metadata and indexes.
2. Recording continues through a temporary database outage until an explicit,
   observable bounded reconciliation/storage limit is reached.
3. A `.partial.mp4` file is never exposed or indexed as a completed recording.
4. Completed metadata requires a durable final path, successful media probe,
   nonzero size, ordered timestamps, codec, and dimensions.
5. File rename and database commit are not falsely described as atomic.
6. A durable `.mp4` without metadata is recoverable orphan evidence.
7. Completed metadata pointing to an absent/corrupt file is an inconsistency,
   not a successful empty query result.
8. Normal search and retention selection use indexed metadata, never directory
   scans.
9. Every queue, database pool, reconciliation batch, query range/result, and
   deletion batch has an explicit bound, overload policy, and metric.
10. Retention never deletes outside the configured recording root and never
    follows symlinks.
11. Retention does not unlink a file before recording a recoverable deletion
    intent/status transition.
12. API resources remain camera-scoped and never expose filesystem paths,
    database keys, credentials, or raw internal errors.
13. Database/reconciliation/retention failure cannot terminate camera ingest or
    unrelated recording pipelines.
14. Time intervals use UTC and half-open `[start, end)` semantics with `start < end`.
15. Playback accuracy, remuxing, and transcoding policy must be explicit; a
    stream-copy response must not be advertised as frame-accurate.

## Target ownership

```text
CameraPipeline
└── Recorder
    ├── durable file publication
    └── bounded lifecycle fact
             |
             v
RecordingMetadataWriter
├── bounded in-process input
├── PostgreSQL recording repository
├── explicit database-outage state
└── durable filesystem evidence for later reconciliation

ReconciliationOwner
├── bounded filesystem candidate discovery
├── bounded database inconsistency query
├── probe/validation boundary
└── idempotent repair decisions

RecordingQueryService
└── indexed repository -> logical API result -> playback assembler

RetentionOwner
├── indexed candidate selection
├── storage pressure observer
└── recoverable mark -> unlink -> finalize workflow
```

`database` implements repositories and migrations; it does not decide playback,
retention, reconciliation, or storage-pressure policy. `storage` owns metadata
semantics, query/reconciliation coordination, and safe file operations. `api`
translates external requests into bounded application use cases. The recorder
does not perform SQL and the API does not scan the filesystem.

## Contracts to design before implementation

The shapes below describe required semantics, not prescribed Rust syntax.
Write focused tests against the contracts before selecting the SQL library or
HTTP representation.

### Recording metadata and lifecycle

At minimum model:

```text
Open -> Finalizing -> Completed
  |         |            |
  `---------+----------> Failed/Incomplete

Completed -> DeletionPending -> Deleted
     |              |
     `----------> Quarantined/Inconsistent
```

Required completed fields include:

- `RecordingId` and `CameraId`;
- exact UTC `started_at` and `ended_at` with `started_at < ended_at`;
- internal durable relative path or another storage-owned locator;
- container/codec and qualified media profile fields;
- width and height;
- observed duration and size;
- lifecycle status and status timestamps;
- creation/reconciliation provenance sufficient for diagnosis.

Do not duplicate derived values unless the query/index requirement justifies
them. Do not accept arbitrary absolute paths from API or database callers. The
storage layer must resolve a validated relative locator beneath its owned root.

Define legal, idempotent transitions and concurrency behavior. A completed row
must not regress to open. Replaying the same completion or reconciliation result
must be safe. Conflicting evidence must produce an explicit inconsistency rather
than last-write-wins corruption.

### Repository boundaries

Use narrow substitution/test boundaries around required behavior, for example:

```text
insert or confirm completed recording
record failed/incomplete evidence
find recordings overlapping camera + [start, end)
find bounded reconciliation candidates/cursor page
mark bounded retention candidates deletion-pending
finalize deletion or retain retryable deletion failure
report dependency health
```

Define transaction boundaries, uniqueness constraints, optimistic/concurrent
behavior, retryable database errors, cancellation, and pool acquisition timeout.
Repository errors are typed and credential-free. Application boundaries may add
context without embedding DSNs or SQL values containing secrets.

### Overlap query

The normative selection predicate is:

```text
recording.started_at < requested_end
AND recording.ended_at > requested_start
AND recording.camera_id = requested_camera
AND recording.status = completed/playable
```

Results are ordered deterministically by start time and recording identity. Set
explicit maximum time span, result count, and page/cursor semantics. Decide how
overlaps, gaps, duplicate evidence, missing files, and clock discontinuities are
represented to the caller. The database index and `EXPLAIN` evidence must match
the actual predicate.

### Database-outage and reconciliation contract

The database is not in the synchronous file-write path. Durable final files and
explicit partial files remain recovery evidence if metadata delivery fails.
Specify:

- metadata input channel capacity and critical overflow behavior;
- how an accepted lifecycle fact becomes durably recoverable before its in-memory
  copy can be lost;
- maximum unindexed files/bytes/age and the warning/critical response;
- retry/backoff/jitter and terminal configuration errors;
- bounded startup and continuous batch sizes;
- a resumable cursor/checkpoint or equivalent work-bounding strategy;
- idempotency when the process dies during repair;
- orphan durable-file probing/import;
- partial classification without promotion;
- completed-row/missing-file and corrupt-file handling;
- behavior when PostgreSQL returns during active recording.

Do not claim an in-memory retry queue is durable. Do not let a full metadata queue
silently discard lifecycle facts. A temporary database outage must leave recording
active and visibly degraded; unrecoverable reconciliation/storage exhaustion must
be an explicit operator-actionable state.

### Retention contract

Separate age policy from storage-pressure policy. Define:

- configurable retention duration and its application boundary;
- warning/critical/emergency storage watermarks with hysteresis;
- protected recent-footage window and minimum retained coverage;
- deterministic oldest-first selection from indexed metadata;
- maximum files/bytes per pass and inter-pass scheduling;
- deletion-pending transaction, safe path resolution, unlink, directory sync,
  final deleted status, and retryable failure behavior;
- races with queries/playback, reconciliation, and duplicate workers;
- behavior for missing files, symlinks, permission failures, and disk full;
- metrics for candidates, protected items, bytes reclaimed, failures, and pressure.

Retention policy belongs in `storage` (or a dedicated real vertical slice), not
inside the database adapter. Do not delete a file merely because its directory
timestamp is old.

### Playback and range response decision gate

Before implementing playback assembly, obtain or record a reversible default for:

- streamed response, download, or both;
- keyframe-aligned versus frame-accurate boundaries;
- acceptable start/end overshoot;
- remuxing versus concatenation behavior for fragmented MP4;
- when transcoding is permitted/required;
- startup latency objective and maximum request duration/bytes;
- gap and corrupt-segment behavior;
- HTTP range/cancellation behavior and concurrent viewer limit.

If the product owner is unavailable, a reversible initial policy may expose
indexed metadata/search first and defer byte streaming. Do not invent a
frame-accurate playback claim or silently return one selected segment.

## Work packages

### P4.1 — Refresh the persistence decision and acceptance contracts

- Update or supersede ADR 004 with the selected PostgreSQL client/migration tool,
  schema ownership, pool bounds, transaction ordering, outage strategy,
  reconciliation limits, deletion workflow, and reversal criteria.
- Record known facts, assumptions, external PostgreSQL requirements, and open
  playback/retention decisions.
- Define acceptance tests by name before implementation.
- Establish a reproducible local PostgreSQL fixture or explicitly document why
  an available repository-owned container/service mechanism is used.

Exit: every durable transition and child/task lifetime can be traced on paper;
the database test boundary is reproducible and contains no checked-in secret.

### P4.2 — Add recording metadata contracts and schema migrations

- Implement validated metadata/lifecycle types in `storage` or the owning domain.
- Add real versioned migrations for cameras and recording segments only as used
  by this vertical slice.
- Add primary/foreign keys, lifecycle checks, uniqueness constraints, UTC time
  checks, size/dimension checks, and indexes supporting overlap/reconciliation/
  retention queries.
- Make migrations forward-only and test application from an empty database.
- Do not pre-create events/ROI/settings tables without a Phase 4 use case.

Tests: invalid intervals/statuses, duplicate identity, camera isolation, migration
from empty, migration re-run behavior, and database constraint violations.

Exit: a completed segment has one validated Rust representation and one enforced
database representation; an empty fixture database migrates reproducibly.

### P4.3 — Implement the PostgreSQL recording repository

- Select a bounded async pool and cancellation-safe query boundary.
- Implement concrete camera/recording persistence needed by the slice.
- Persist/confirm completed, failed, and incomplete evidence idempotently.
- Classify retryable connectivity/pool errors separately from invariant conflicts.
- Add structured tracing and bounded metrics for pool, writes, errors, and latency.
- Keep SQL and driver types inside `database`.

Tests: success, duplicate replay, conflicting replay, transaction rollback, pool
timeout, PostgreSQL restart, cancellation, and secret-redacted diagnostics.

Exit: repository behavior is proven against real PostgreSQL, not only mocks.

### P4.4 — Connect recording completion without coupling media to PostgreSQL

- Introduce one owned metadata-writer/reconciliation boundary at the application
  level; do not add SQL calls to `run_segment_recorder`.
- Preserve the bounded recorder lifecycle channel and define critical overflow.
- On successful durable rename/probe, attempt metadata persistence in the agreed
  order while retaining filesystem evidence on database failure.
- Ensure one camera's metadata failure cannot block another recorder indefinitely.
- Supervise the metadata writer with shutdown, drain, restart/backoff, and join
  policy; expose degraded readiness separately from liveness/camera recording.

Tests: normal completion, database unavailable before/after rename, channel
saturation, writer exit, process shutdown with pending facts, database recovery,
and multiple-camera isolation.

Exit: completed recordings become indexed in normal operation, and PostgreSQL
loss leaves recording active with bounded observable reconciliation work.

### P4.5 — Implement database-aware reconciliation

- Generalize the current discovery scan into a bounded, resumable owner.
- Compare filesystem evidence and database lifecycle state in deterministic pages.
- Probe/import durable orphan `.mp4` files only after all completion invariants
  pass.
- Record incomplete partial evidence without presenting it as playable.
- Mark/surface completed metadata whose file is missing or corrupt.
- Avoid scanning active files as completed and avoid racing pipeline startup.
- Run startup reconciliation and a bounded continuous retry loop according to the
  ADR; cancellation and database outage remain designed states.

Tests: orphan durable import, partial classification, missing file, corrupt file,
duplicate replay, crash during repair, scan/cursor bounds, symlink rejection,
database outage/recovery, and concurrent recording creation.

Exit: every file/row mismatch has a deterministic idempotent outcome and a metric;
normal queries still never scan directories.

### P4.6 — Add indexed recording search

- Implement typed UTC half-open query ranges with maximum span/result bounds.
- Add repository overlap selection and prove its index plan.
- Add camera-scoped logical API resources, for example:

```text
GET /api/cameras/{camera_id}/recordings?start=<RFC3339>&end=<RFC3339>&cursor=<opaque>
GET /api/cameras/{camera_id}/recordings/{recording_id}
```

- Normalize accepted RFC 3339 offsets to UTC and reject ambiguous/malformed,
  reversed, excessive, or unauthorized-preparation ranges before repository work.
- Return logical metadata and playback links/status, never storage paths.
- Distinguish no overlap from inconsistent/missing storage evidence.

Tests: boundary-touching intervals, overlap across at least three segments,
ordering, pagination, offset normalization, DST/property cases, maximum bounds,
cross-camera identifiers, missing/corrupt evidence, and stable external errors.

Exit: production recording search is entirely index-backed and returns all
overlapping segments in deterministic order.

### P4.7 — Decide and implement the first multi-segment playback behavior

- Resolve the playback decision gate before coding byte delivery.
- Define an application playback plan from indexed segments, including gaps and
  boundary/keyframe accuracy.
- Stream/remux/assemble without loading complete recordings into memory.
- Bound concurrent work, buffered bytes, requested duration, response time, and
  child processes; cancellation must stop owned work.
- Never decode/re-encode unless the accepted accuracy/codec boundary requires it.
- Keep filenames and storage paths internal.

Tests: range spanning three or more segments, exact boundary, mid-GOP request,
gap, overlap, corrupt/missing file, client cancellation, slow client, concurrent
limit, and full decode of the produced response.

Exit: one documented playback mode returns the correct interval under its stated
accuracy policy; unsupported accuracy modes fail explicitly.

### P4.8 — Implement indexed retention and storage pressure

- Add validated versioned retention/watermark configuration and document whether
  each update applies immediately or on the next retention pass.
- Observe filesystem capacity without blocking Tokio workers.
- Select bounded candidates from indexed metadata, honoring protected recent
  footage and deterministic ordering.
- Implement recoverable deletion-pending -> unlink/sync -> deleted transitions.
- Contain deletion failure and continue safely with other eligible candidates
  only according to the documented policy.
- Reconcile missing files and interrupted deletion transitions.
- Expose bounded metrics and storage readiness/degradation.

Tests: age boundary, watermark hysteresis, protected window, ordering, batch
bounds, path traversal, symlink, permission failure, missing file, interrupted
deletion, concurrent workers, disk-full simulation, and exact reclaimed bytes.

Exit: retention cannot escape the configured root, cannot delete protected recent
footage, and every attempted deletion has recoverable indexed state.

### P4.9 — End-to-end failure and operational qualification

Create a deterministic fixture that:

1. starts PostgreSQL from an empty schema and applies migrations;
2. runs at least two cameras through multiple segment completions;
3. proves database rows match probed files;
4. stops PostgreSQL while recording continues and reconciliation backlog becomes
   visible without unbounded memory;
5. restarts PostgreSQL and proves idempotent catch-up;
6. queries a range overlapping at least three segments;
7. exercises the accepted playback policy and fully decodes the response;
8. creates orphan/partial/missing/corrupt evidence and proves each classification;
9. exercises retention warning/critical behavior and a recoverable deletion;
10. shuts down and joins camera, recorder, metadata, reconciliation, API, database,
    playback, and retention owners consistently.

Record exact PostgreSQL/server versions, filesystem, fixture bitrate/profile,
segment duration, query plan, queue/backlog maxima, and relevant resource samples.

Exit: the complete Phase 4 data lifecycle is reproducible from durable file
publication through index, outage recovery, query/playback, and safe deletion.

## Required test matrix

| Scenario | Filesystem evidence | Database evidence | Required result |
|---|---|---|---|
| Normal completion | durable validated `.mp4` | completed row | searchable/playable |
| DB down at completion | durable `.mp4` | absent initially | recording continues; bounded backlog; later import |
| Process death before rename | `.partial.mp4` | no completed row | incomplete only; never searchable as complete |
| Death after rename/before row | durable `.mp4` | absent | orphan probed and imported idempotently |
| Completed row/file missing | absent | completed row | explicit inconsistency/alert; not silent success |
| Durable file corrupt | corrupt `.mp4` | absent or completed | quarantine/inconsistent; never playable |
| Duplicate completion replay | same durable file | same row | idempotent success |
| Conflicting completion replay | conflicting facts | existing row | typed conflict; no overwrite |
| Three-segment range | three overlapping files | indexed rows | deterministic complete selection |
| Boundary touch | ends at start / starts at end | indexed rows | excluded under half-open semantics |
| Retention eligible | durable old file | completed row | deletion-pending -> unlink -> deleted |
| Protected recent footage | durable recent file | completed row | never selected |
| Delete fails | file retained | deletion-pending/failure | retryable and visible |
| API client stalls | files untouched | query bounded | work cancelled/timed out; recording unaffected |
| Process shutdown | active/pending evidence | pending writes | all owners joined; evidence remains reconcilable |

## Observability requirements

Use bounded-cardinality labels such as operation, lifecycle status, and fixed
failure category. Never label by recording ID, file path, SQL text, DSN, error
text, or requested timestamps.

Expose at least:

- database pool size/in-use/waiters/timeouts and connectivity state;
- metadata write success/failure/retry latency and bounded queue depth/capacity;
- unindexed file count/bytes/oldest age and reconciliation scan/import outcomes;
- inconsistent/missing/corrupt/partial counts by bounded category;
- recording overlap query count/result count/latency and rejected range count;
- playback active work, buffered bytes, cancellations, failures, and latency;
- storage total/free/used bytes and warning/critical state;
- retention candidates/protected/deletion-pending/deleted/failures/bytes reclaimed;
- per-camera recording completion/failure continuity during database outages.

Liveness remains process-only. Readiness must distinguish database/query
availability, metadata backlog degradation, storage pressure, and per-camera
recording health. A database outage must not report the entire media process dead.

## Explicit decision and external gates

Do not silently freeze:

- PostgreSQL deployment/version, DSN/secret injection, TLS, backup, and migration
  ownership;
- SQL client/pool/migration library;
- maximum database outage duration/backlog files/backlog bytes;
- metadata delivery guarantee and durable reconciliation checkpoint mechanism;
- exact lifecycle statuses exposed externally;
- playback stream/download behavior, keyframe accuracy, remux/transcode policy,
  startup target, maximum range, and concurrent viewers;
- retention duration approval, protected window, watermark thresholds/hysteresis,
  and emergency disk-full behavior;
- behavior for corrupt media, missing files, and unrecoverable deletion failure;
- authorization rules for cross-camera recording access;
- target filesystem/disk durability and PostgreSQL host topology.

When a decision is unavailable, continue independent work using a reversible,
explicitly documented default only where it cannot masquerade as an accepted
product requirement. Record genuinely external gates as unverified/blocked and
continue with independent Phase 4 packages.

## Validation commands

Run focused tests and migrations during each package. At the Phase 4 local gate
run:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --workspace --release
git diff --check
```

Also run:

- the repository-owned PostgreSQL migration/repository fixture;
- the complete Phase 4 database-outage/reconciliation/retrieval/retention fixture;
- `scripts/phase0/retina-rtsp-fixture.sh`;
- `scripts/phase3/multi-camera-fixture.sh`;
- query-plan evidence for the real overlap and retention predicates; and
- full media decode for the accepted multi-segment playback response.

Report unavailable production PostgreSQL, representative cameras, target storage,
and hardware qualification separately. Do not weaken tests or substitute a mock
for the real PostgreSQL gate.

## Phase exit gate

Phase 4 is locally complete only when:

- migrations apply reproducibly from an empty PostgreSQL database;
- every completed/failed/incomplete/deletion lifecycle transition is typed,
  constrained, and idempotent;
- normal completion persists indexed metadata without putting PostgreSQL in the
  media-write critical path;
- database interruption leaves recording active with bounded visible durable
  reconciliation work and successful catch-up after recovery;
- filesystem/database mismatches are classified and repaired or surfaced
  deterministically;
- production recording queries use indexed overlap selection, never directory
  scans, and return all matching segments;
- the first playback mode works across at least three segments under an explicit
  keyframe/remux/transcode policy and fully decodes;
- retention selects from indexed metadata, protects recent footage, stays beneath
  the configured root, and recovers from interrupted/deletion failures;
- database, reconciliation, query, playback, retention, and media failures remain
  isolated according to policy;
- every spawned task/process has an owner, cancellation path, and join result;
- state, queue/backlog pressure, database health, inconsistencies, query behavior,
  and storage pressure are observable; and
- all local validation commands and deterministic fixtures pass.

External product/deployment decisions that cannot be exercised locally must be
reported explicitly rather than claimed complete.

## First executable action

Do not begin with HTTP routes, a generic repository trait, or a broad schema.
Start by updating/superseding ADR 004 and writing focused contract/property tests
for recording lifecycle metadata and half-open overlap selection. Then add the
minimum camera/recording migrations and prove them against an empty PostgreSQL
fixture. Only after idempotent persistence and crash ordering are established
should the recorder lifecycle boundary feed the metadata writer. Reconciliation,
API search, playback, and retention follow in that order.

