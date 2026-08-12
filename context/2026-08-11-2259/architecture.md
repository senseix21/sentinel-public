# Phase 4 architecture — 2026-08-11 22:59 +06

## Implemented ownership

```text
CameraPipeline -> Recorder -> durable rename/probe -> bounded lifecycle fact
                                                   -> MetadataWriter -> PostgreSQL

Filesystem evidence <-> ReconciliationOwner <-> PostgreSQL lifecycle rows
Indexed rows -> camera-scoped API -> PlaybackPlan -> owned FFmpeg stream-copy child
Indexed rows -> RetentionOwner -> deletion_pending -> safe unlink/sync -> deleted
```

- PostgreSQL is never called from the recorder or camera media path.
- The application owns and joins metadata, reconciliation, retention, API, and
  camera-supervisor tasks.
- Search is a bounded indexed half-open overlap query. Filesystem inspection is
  only a consistency/playability check and reconciliation input.
- Playback is segment-aligned H.264 stream-copy fMP4 with a 24-hour/1,000-segment
  request bound, one sequential viewer, 64 KiB buffer, and child cancellation.
- Retention claims bounded oldest-first rows before unlink, rejects symlinks and
  root escape, syncs the containing directory, and resumes deletion-pending rows.
