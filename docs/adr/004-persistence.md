# ADR 004: Recording Persistence and Container Lifecycle

Status: accepted for the H.264 MVP vertical slice; production storage durability remains to be qualified

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

