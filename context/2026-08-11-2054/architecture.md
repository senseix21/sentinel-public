# Architecture — Phase 3 Local Gate

```text
ServerRuntime
├── bounded CameraSupervisor command task
│   └── BTreeMap<CameraId, CameraEntry>
│       ├── Arc<validated configuration>
│       ├── pipeline ownership state
│       ├── optional CameraPipeline owner
│       ├── bounded health reader
│       └── pending bounded command replies / terminal outcome
├── supervisor-owned start/stop JoinSet
├── bounded credential-free health registry
└── loopback health/metrics/camera API task
```

Each `CameraPipeline` constructs exactly one Retina ingestion child and one
mandatory recorder child. The joint owner stops and joins the sibling on every
exit path. Stop/remove operations move the pipeline into a supervisor-owned join
operation, so commands for other cameras continue while teardown completes.
Removed entries disappear only after join. The API never owns the map, task,
media, camera credentials, or storage path.

