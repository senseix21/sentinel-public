# Surveillance Platform

Production-oriented Rust workspace for a self-hosted, multi-camera surveillance and video-analytics platform.

The workspace contains tested media, camera-lifecycle, RTSP/TCP ingestion, and
recorder-boundary contracts. The original single-camera `cam_rtsp` prototype
remains under `src/` as reference material and is not a workspace package.

## Start here

- `AGENTS.md` — standing engineering rules and protected boundaries.
- `ROADMAP.md` — detailed phase plan and acceptance gates.
- `context/README.md` — latest compact repository handoff.
- `analysis.md` — full architecture proposal and rationale.
- `docs/architecture.md` — durable architecture summary.
- `docs/requirements.md` — confirmed requirements and explicit assumptions.

## Architecture

```text
Camera -> Ingestion -> Bounded Media Distribution
                           |-> Recorder
                           |-> Detector -> Domain Event Bus -> Rules/API/Analytics
                           `-> LiveKit Publisher
```

Media packets and decoded frames travel through the media plane. The event bus carries lightweight domain facts only.

## Workspace checks

Rust 1.88 or newer is required. The minimum version is set by the pinned Retina
RTSP adapter dependency.

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --workspace --release
```

## Run the Phase 3 multi-camera server

The server has no default camera URL. Deployment identity, source, and actual
encoded dimensions are required. Credentials are optional but must be supplied
as a complete pair and are never embedded in the URL.

```bash
export SURVEILLANCE_CAMERA_ID=camera-01
export SURVEILLANCE_RTSP_URL=rtsp://127.0.0.1:8554/camera-01
export SURVEILLANCE_VIDEO_WIDTH=1920
export SURVEILLANCE_VIDEO_HEIGHT=1080
export SURVEILLANCE_RECORDING_ROOT=./recordings
# Optional; Phase 2 rejects non-loopback health bindings.
export SURVEILLANCE_HEALTH_BIND=127.0.0.1:3000

# Set both only when the source requires authentication:
# export SURVEILLANCE_RTSP_USERNAME=operator
# export SURVEILLANCE_RTSP_PASSWORD='injected-by-secret-manager'

RUST_LOG=info cargo run -p surveillance-server
```

Ctrl-C requests bounded RTSP teardown, closes media production, drains the
recorder, and finalizes viable active media before the process exits. FFprobe
must be available on `PATH`.

The initial environment camera starts automatically. Additional cameras are
configured and controlled through the loopback-only Phase 3 API. The deployment
recording root applies to every camera and is deliberately not an API field.

```bash
curl -X POST http://127.0.0.1:3000/api/cameras \
  -H 'Content-Type: application/json' \
  --data '{
    "camera_id":"camera-02",
    "rtsp_endpoint":"rtsp://127.0.0.1:8554/camera-02",
    "width":1920,
    "height":1080
  }'
curl -X POST http://127.0.0.1:3000/api/cameras/camera-02/start
```

The operational endpoints are intentionally loopback-only in Phase 3:

- `GET /live` reports process liveness.
- `GET /ready` requires the camera session to be connected.
- `GET /metrics` exposes bounded-cardinality Prometheus text metrics.
- `GET|POST /api/cameras` lists or adds credential/path-free camera resources.
- `GET /api/cameras/{id}` and `/health` expose bounded per-camera state.
- `POST /api/cameras/{id}/start|stop` controls explicit lifecycle transitions.
- `DELETE /api/cameras/{id}` cancels and joins before removal.

### Mac webcam convenience targets

The root Makefile wraps the manual AVFoundation → MediaMTX → RTSP workflow.
Run each long-lived target in its own terminal:

```bash
make prerequisites
make webcam-devices
make mediamtx
make webcam WEBCAM_DEVICE=0
make server
```

Then inspect the running system or completed files:

```bash
make health
make recordings
make verify-recordings
make play FILE=recordings/mac-webcam/YYYY/MM/DD/example.mp4
```

`WEBCAM_DEVICE`, dimensions, FPS, camera ID, paths, URL, and health binding are
overridable Make variables. Use `make help` for the complete target list.

## Current status

Roadmap Phase 3 is locally implemented and qualified against deterministic H.264
RTSP fixtures. Ten synthetic sources proved multi-camera ownership, recording,
source-failure isolation, reconnection, lifecycle removal, bounded metrics, and
structured shutdown. Representative 4 MP cameras, target hardware, and an agreed
soak duration remain an explicit external gate before a ten-camera product claim.
