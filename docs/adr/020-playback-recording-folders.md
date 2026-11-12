# ADR 020: Read-only recording folders in Playback

Status: Accepted for the user-authorized folder picker (2026-11-01).

## Decision

Plain `make run` remains camera-independent. Playback can select an existing
recording folder through a server-side picker. The browser never supplies an
arbitrary filesystem path. Discovery is bounded beneath deployment-approved
roots (`SURVEILLANCE_RECORDING_BROWSE_ROOTS`, platform path-list syntax); the
launcher defaults to its recordings directory and local development-state tree.
Symlinks and paths escaping those roots are refused. Folder labels are relative;
absolute server paths are not returned over HTTP.

Each registered folder has a persistent source ID and its own namespace in the
PostgreSQL recording index. Existing rows migrate to `primary`, retaining current
URLs. Archive URLs include the source ID explicitly, so simultaneous users and
in-flight playback never depend on mutable global selection. Camera authorization
and export auditing still apply; export rows retain the source ID. Camera scope
continues to mean the logical camera ID across approved sources. Registering
archives from a separate tenant/site would require a source-scoped authorization
decision and is not part of this single-deployment picker. Discovery/registration/indexing require an
administrator; viewers retain camera-scoped recording reads.

Archives are read-only media sources: indexing reuses the bounded reconciliation
and actual ffprobe metadata path, never copies, moves or deletes video. Retention
and the primary metadata/reconciliation owners operate only on `primary`.
Duplicate recording IDs/relative paths may exist in different sources without
conflict. Only the established camera/UTC segmented-fMP4 layout is supported;
this is not a generic arbitrary-MP4 import feature.

One owned background index worker handles a bounded queue with one running job
and no backlog (busy requests receive 409). Progress/state are polled by the UI;
HTTP does not wait for thousands of ffprobe calls. Shutdown cancels between
bounded reconciliation work and joins the worker. Source roots are revalidated
before indexing and before playback; file canonicalization remains enforced.
Index errors and unsupported files are visible; partial successful indexing is
explicit, not reported as complete. Index data survives restart. Selection is
browser-local UI state and is reloaded from the persisted source list.

## Trade-offs and reversal

The shared recording schema gains a source namespace and composite identities.
This is preferable to copying large media, symlink mounts, per-folder databases,
or serving directory scans as playback queries. Rollback to the old schema
requires removing non-primary index rows and restoring the old uniqueness
constraints; archive video is untouched. Physical camera/media compatibility,
packaging, distributed storage and arbitrary media timestamp inference remain
outside this slice.
