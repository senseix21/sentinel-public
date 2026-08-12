# Architecture — Phase 4 local gate

The Phase 4 topology remains a bounded modular monolith:

```text
Recorder -> durable filesystem -> bounded MetadataWriter -> PostgreSQL index
                    |                         |
                    `-> ReconciliationOwner <-'
PostgreSQL index -> camera API -> PlaybackPlan -> owned FFmpeg stream-copy child
PostgreSQL index -> RetentionOwner -> claim -> safe unlink/sync -> finalize
```

One shared constant-memory `Phase4Metrics` handle is observed by the loopback
adapter. It carries only fixed counters/gauges and storage pressure; it never
retains media, identifiers, paths, timestamps, DSNs, or arbitrary error text.

Readiness remains separate from liveness. Database or metadata degradation may
leave media recording active, while critical storage pressure explicitly makes
the process unready.
