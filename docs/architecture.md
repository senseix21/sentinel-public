# Architecture

The durable architecture is defined by `analysis.md`, constrained by `AGENTS.md`, and delivered through `ROADMAP.md`.

The central media topology is:

```text
Camera -> Ingestion -> Bounded Media Distribution
                           |-> Recorder
                           |-> Detector -> Domain Events
                           `-> LiveKit
```

Media packets and frames never travel over the domain event bus.

The Phase 3 process ownership boundary is:

```text
ServerRuntime
├── bounded CameraSupervisor command task
│   └── CameraId -> owned CameraPipeline -> camera + recorder children
├── bounded credential-free health registry
└── loopback health/metrics/camera API sibling
```

The supervisor is the only pipeline-map writer. Stop and remove operations keep
servicing unrelated commands while supervisor-owned join operations complete;
removal becomes visible only after its pipeline is cancelled and joined. API
listener failure does not send a media shutdown signal.
