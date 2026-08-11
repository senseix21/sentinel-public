# Architecture Context — 2026-08-11 11:54 +06

The segment recorder is an owned future rather than a detached task. It receives
one bounded `MediaSubscriber`, owns one `Fmp4SegmentSink`, awaits one root-owned
`SegmentFileStore`, and publishes immutable completed/failed lifecycle facts to
a caller-created bounded Tokio channel. A closed critical channel is a visible
runtime failure; metadata cannot accumulate without bound or disappear silently.

On rotation, the runtime maps the exclusive media boundary to wall time, moves
`OpenSegment` to `FinalizingSegment`, persists and probes the bytes, constructs
durable evidence, and only then emits `CompletedSegment`. Storage failure emits a
stage-retaining `FailedSegment` before the runtime returns its error.

The camera supervisor and segment recorder remain separately owned futures. The
next application boundary must poll both, order shutdown as camera/distributor
close then recorder drain, and define which exit terminates or restarts the pair.

