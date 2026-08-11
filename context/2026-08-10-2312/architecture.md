# Architecture Context — 2026-08-10 23:12 +06

Bounded queue state remains protected by the media distributor's synchronous
mutex. A Tokio `watch<u64>` generation is only a wake mechanism. An unseen
generation closes the empty-check/wait race, and producer closure also advances
the generation.

`RecorderTask<S>` owns cancellation and the sole `JoinHandle`. Its task owns
`RecorderConsumer<S>`, subscription, and sink. Input drain, cancellation, and
sink failure return those resources to the supervisor. Graceful drain is
producer-close followed by join; cancellation explicitly makes no drain claim.
