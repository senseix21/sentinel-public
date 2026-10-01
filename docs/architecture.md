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

The camera owner accepts an exhaustive H.264 source configuration. Physical IP
cameras connect directly through Retina RTSP/TCP. Local webcams are captured by
a server-owned FFmpeg child that emits bounded Annex-B H.264 into the same
source lifecycle; there is no local RTSP proxy in that path. The distributor,
recorder, detector, and LiveKit boundaries are identical for both source kinds.

MediaMTX retirement and fixture replacement are governed by
`docs/ffmpeg-webcam-source/SPEC.md` and `PLAN.md`. External RTSP protocol,
authentication, and reconnect qualification still requires an owned RTSP peer;
the FFmpeg webcam source does not prove those transport behaviors.

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
