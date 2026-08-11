# Architecture Context — 2026-08-10 22:56 +06

Recording state uses distinct owning types: `OpenSegment`,
`FinalizingSegment`, `CompletedSegment`, and `FailedSegment`. Forward transitions
consume the prior state, preventing duplicate completion/failure operations.

Only H.264 fragmented MP4 is currently representable. Active paths end in
`.partial.mp4`; the expected durable target is derived in the same directory.
Completion requires a strictly ordered UTC interval and durable-file evidence
containing the expected path, nonzero size, and known nonzero dimensions.
