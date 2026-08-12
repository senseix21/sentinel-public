# Architecture — Phase 4 First Slice

```text
Recorder durable rename + probe
└── bounded lifecycle fact (existing; not yet database-connected)

storage contracts
├── CompletedRecording (validated durable evidence)
├── RecordingLocator (relative, non-traversing durable .mp4)
├── RecordingStatus legal transition relation
└── RecordingTimeRange (bounded half-open overlap predicate)

PostgreSQL schema
├── cameras
└── recording_segments
    ├── completed overlap partial index
    ├── reconciliation partial index
    └── retention partial index
```

Filesystem bytes remain storage-owned. PostgreSQL currently has only a proven
schema contract; no runtime camera or recorder path opens a database connection.
