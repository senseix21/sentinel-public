# ADR 014: Recording video subresource

- Status: Accepted
- Date: 2026-09-05

## Context

The existing playback API requires a UTC start/end range and may assemble
multiple contiguous recording segments. Clients also need to retrieve one
known recording, optionally limited to the first N seconds. Making `end`
optional on the range API would give one route two different identities and
would make “full video” ambiguous across multiple segments.

## Decision

Add an authenticated, camera-scoped resource:

```text
GET /api/cameras/{camera_id}/recordings/{recording_id}/video
GET /api/cameras/{camera_id}/recordings/{recording_id}/video?duration_seconds=N
```

Omitting `duration_seconds` streams the complete completed recording. Supplying
it streams the leading N seconds. N is a decimal integer from 1 through 60 and
must not exceed the indexed recording duration. Unknown or duplicate query
parameters are rejected.

The first implementation uses FFmpeg stream-copy into fragmented MP4. It is
packet/keyframe bounded, not frame-exact, and remains unseekable and chunked.
It uses the existing export authorization, audit, bounded buffer, stall timeout,
and owned-child lifecycle. The existing range playback API is unchanged.

## Consequences

- Existing clients and storage metadata remain compatible; no migration is
  required.
- One recording ID identifies one media object, so omitted duration has a clear
  meaning.
- Retrieval does not add a camera connection or a decode/re-encode path.
- The requested end may differ slightly from the last emitted packet timestamp.
  Frame-exact clipping would require transcoding, capacity limits, measurements,
  and a follow-up decision.

## Reversal criteria

Remove the additive route and UI controls if packet-bounded clipping is not
useful. Stored recordings and the time-range playback endpoint are unaffected.
