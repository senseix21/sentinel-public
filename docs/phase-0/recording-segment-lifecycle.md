# Recording Segment Lifecycle Contract

Date: 2026-08-10

## Scope

This contract models lifecycle ownership for the first H.264 fragmented-MP4
recording slice. It does not perform muxing, probing, synchronization, rename,
or database writes. Those adapters must produce evidence before the domain can
represent a segment as completed.

## State flow

```text
OpenSegment --stop accepting media--> FinalizingSegment
     |                                      |
     | fail                                 | durable rename + probe evidence
     v                                      v
FailedSegment                         CompletedSegment
                                            |
                                            `-- eligible for metadata indexing
```

Rust ownership makes the forward transitions consuming operations. The same
open/finalizing value therefore cannot be completed or failed twice.

## Invariants

- Active files have a nonempty `.partial.mp4` filename.
- The durable target removes only `.partial` and stays in the same directory.
- `storage` owns placement beneath the configured root using validated camera
  and recording identifiers plus validated Gregorian UTC components. The
  recording identity prevents collisions when reconnects create multiple
  segments in one second.
- Segment end UTC is strictly later than start UTC.
- A completed segment has a nonzero observed file size and nonzero encoded
  dimensions.
- Durable validation evidence must identify the exact expected rename target.
- Failed segments retain their partial path, lifecycle stage, and known time
  interval for metrics and startup reconciliation.
- The only qualified format represented today is H.264 fragmented MP4. Adding
  another codec/container requires evidence and an explicit enum variant.

## Required adapter ordering

The future finalizer performs these operations in order:

1. Stop accepting encoded media at a valid access-unit boundary.
2. Flush and finalize the fragmented MP4 muxer.
3. Apply the selected filesystem durability policy.
4. Atomically rename `.partial.mp4` to `.mp4` on the same filesystem.
5. Probe the durable file for dimensions, timing, codec, and size.
6. Construct `DurableFileEvidence` and transition to `CompletedSegment`.
7. Persist completed metadata; reconcile later if the database is unavailable.

A failure before step 6 produces `FailedSegment`; it must never expose the
partial file as a completed recording.

## Deferred decisions

- Muxer implementation and child-process/FFI ownership.
- Power-loss durability (`fsync` file/directory policy) on production storage.
- Camera and recording identifiers in persisted metadata.
- Exact metadata-reconciliation queue and database transaction behavior.
- H.265/container support.
