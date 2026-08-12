# Next actions — Phase 5 local qualification

1. Run the full workspace format, Clippy, tests, release build, and diff checks.
2. Replace or extend the adapter boundary so complete access-unit timestamps
   drive real RTP publication; do not accept CLI clock-skew warnings as latency.
3. Add a deterministic real server stop/restart owner and prove one participant,
   one track, recording continuity, reconnection, and cleanup.
4. Retain and fully decode subscriber output for sustained single- and
   multi-slice real-time fixtures.
5. Measure a named publication boundary at p50/p90/p99 only after clock behavior
   is correct; record one/two/ten-camera CPU, memory, network, queues, and drops.
6. Obtain the external product/deployment inputs listed in the decision snapshot.
