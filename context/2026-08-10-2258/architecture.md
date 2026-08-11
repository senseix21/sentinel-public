# Architecture Context — 2026-08-10 22:58 +06

`RecorderConsumer<S>` is a nonblocking adapter between
`MediaSubscriber<H264AccessUnit>` and a narrow `H264AccessUnitSink`. It owns both,
so there is no informal sharing of muxer or queue state.

Each step writes at most one complete access unit. Success, temporary empty,
drained closure, and sink rejection are distinct outcomes. Sink rejection
returns the failed reference-counted unit to the supervisor and increments an
explicit counter.
