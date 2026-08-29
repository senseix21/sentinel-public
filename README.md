# SENTINEL

SENTINEL is a self-hosted, multi-camera surveillance and video analytics
platform written in Rust. It ingests each RTSP camera once, distributes bounded
media streams to recording, live viewing, and detection consumers, indexes
recordings in PostgreSQL, and serves an authenticated operator dashboard.

The current system is a production-oriented implementation under active
qualification. The local H.264 path is runnable end to end. Representative
production cameras, target deployment hardware, 10/20-camera sustained load,
production identity infrastructure, packaging, and several product decisions
remain open. See [Current capabilities](#current-capabilities) before treating a
development result as a production claim.

## Documentation

- **[Client system guide](docs/CLIENT_SYSTEM_GUIDE.md)** — plain-language
  explanation of the product, end-to-end operation, components, delivered
  capabilities, limitations, client decisions, and path to production.
- **[Complete HTTP API reference](docs/API.md)** — authentication, roles,
  cameras, recording search and video retrieval, LiveKit viewer access,
  settings, identities, errors, examples, and current limitations.
- [Delivery roadmap](ROADMAP.md) — authoritative implementation status and open
  qualification gates.
- [Architecture decisions](docs/adr/) — accepted design choices and reversal
  criteria.

## Quick start on macOS

The fastest way to run the complete system uses the Mac webcam as an RTSP camera:

```text
AVFoundation webcam -> FFmpeg -> MediaMTX -> RTSP ingest
                                              |-> local MP4 recording
                                              `-> LiveKit -> browser WebRTC
```

### 1. Install prerequisites

Required commands:

- Rust 1.88 or newer (`cargo` and `rustc`)
- PostgreSQL (`initdb`, `pg_ctl`, and `psql`)
- FFmpeg (`ffmpeg`, `ffprobe`, and optionally `ffplay`)
- MediaMTX (`mediamtx`)
- LiveKit Server (`livekit-server`)
- Python 3, `curl`, and `lsof`

On macOS, install the third-party services with your preferred package manager,
then verify the basic media tools:

```bash
make prerequisites
```

The full-stack launcher checks its additional PostgreSQL, LiveKit, Python, and
port-inspection dependencies before changing runtime state.

### 2. Select the webcam

List AVFoundation devices:

```bash
make webcam-devices
```

The default video device is `0`. On first use, macOS may ask for camera access.
Grant the terminal application access under **System Settings -> Privacy &
Security -> Camera**.

### 3. Start SENTINEL

Use an explicit local-only password when you want a predictable login:

```bash
SENTINEL_DEV_PASSWORD=sentinel-local make run
```

Then open:

```text
http://127.0.0.1:3001/
```

Sign in with:

```text
Username: operator
Password: sentinel-local
```

If `SENTINEL_DEV_PASSWORD` is omitted, the launcher generates a new password and
prints it in the `AUTH` startup lines. It is neither persisted nor written to a
log file.

Keep the terminal running. Press Ctrl-C once to stop the owned processes. The
database, recordings, settings, and recording index are preserved for the next
run.

### 4. Override local fixture values

All values are optional:

```bash
SENTINEL_DEV_PASSWORD=sentinel-local \
WEBCAM_DEVICE=1 \
VIDEO_WIDTH=1920 \
VIDEO_HEIGHT=1080 \
VIDEO_FPS=15 \
VIDEO_BITRATE=2500k \
CAMERA_ID=office-webcam \
make run
```

Use the development build for the normal edit/run loop:

```bash
make run
```

Use the optimized binary for manual qualification or performance measurements:

```bash
make run-release
```

Do not publish performance numbers from `make run`; it deliberately uses the
faster-to-build development profile.

## What `make run` starts

The launcher owns and supervises this local stack:

| Component | Default endpoint or path | Responsibility |
|---|---|---|
| PostgreSQL | `127.0.0.1:55432` | Cameras, identities, settings, audit, and recording index |
| LiveKit | `ws://127.0.0.1:7880` | WebRTC distribution to browser viewers |
| MediaMTX | `rtsp://127.0.0.1:8554/webcam` | Local RTSP bridge |
| FFmpeg | webcam device `0` | Publishes H.264 from AVFoundation to MediaMTX |
| SENTINEL server | `http://127.0.0.1:3000` | API, dashboard assets, ingest, recording, and publication |
| Development login proxy | `http://127.0.0.1:3001` | Local login, session, and trusted principal assertion |

Persistent local state defaults to:

```text
.local/run/pgdata    PostgreSQL data
.local/run/logs      service logs
recordings/          finalized and partial recording segments
```

Generated LiveKit and trust-boundary secrets are per-run development values.
They are not deployment credentials.

## Security model

### Local development

Always browse through `http://127.0.0.1:3001`. The development proxy establishes
an in-memory, expiring session after the login form succeeds. It then proves its
origin to the Rust server and asserts the configured principal. The Rust server,
not the proxy, loads that principal's role and camera scope and makes the final
authorization decision.

Port `3000` is the server itself. A direct request to a protected `/api` route
normally returns `401 Unauthorized`; that is intentional. The public liveness,
readiness, metrics, and UI asset routes remain available without an API identity.

The local login is a development fixture. It has no TLS, MFA, account recovery,
durable session store, or production rate-limit infrastructure. It binds only to
loopback and must not be exposed to a network or deployed as the identity
provider.

### Production boundary

ADR 009 requires an identity-aware reverse proxy to authenticate the human and
assert a principal only after the server verifies both the proxy peer and a
constant-time shared-secret proof. Bearer tokens hashed with Argon2id support
service identities. Every protected route passes through one authorization
chokepoint, with roles and camera scope held locally.

TLS termination, the identity provider, deployment packaging, secret ownership,
upgrade/rollback, and backup/restore remain deployment decisions. A non-loopback
server bind is refused unless the configured trust boundary and an enabled
principal make that exposure explicit.

## System architecture

```text
RTSP camera
    |
    v
Camera ingestion owner  (one authenticated upstream connection per camera)
    |
    v
Bounded media distributor
    |-----------------------> Recorder -> fragmented MP4 -> PostgreSQL index
    |-----------------------> LiveKit publisher -> WebRTC viewer
    `-> sampled detector -> bounded domain event bus -> future rules/analytics

Browser -> identity-aware proxy -> HTTP/JSON API -> application use cases
                               `-> playback response streams recorded MP4 bytes
```

Media and events are separate planes. Encoded packets are not placed on the
domain event bus. Each consumer has an explicit bounded queue and overload
policy. A recorder, detector, publisher, database, or individual camera failure
must not terminate unrelated camera pipelines.

The external control API is HTTP/JSON, playback is streamed over HTTP, and live
video uses WebRTC through LiveKit. Ordinary gRPC is not used to carry media. A
future internal gRPC control plane is allowed only after Phase 9 measurements
show that a process boundary is necessary; it is not part of the current system.

## Current capabilities

| Capability | Current state | Important limits |
|---|---|---|
| RTSP camera ingest | Implemented for the qualified H.264/TCP path | Representative cameras, H.265, High Profile/multi-slice, and 10/20-camera qualification remain |
| Local recording | H.264 MVP implemented | Deployment filesystem and genuine disk-full behavior remain unqualified |
| Recording index and retention | Implemented with PostgreSQL reconciliation and watermarks | Backup/restore and target storage sizing remain open |
| Live browser video | Implemented through viewer-gated transcoding and LiveKit | First-picture time, viewer-visible reconnect, latency, representative cameras, and production concurrency remain open |
| Time-range search and playback | Implemented with segment-aligned stream-copy semantics | No frame-exact arbitrary boundaries; browser seeking is limited by the assembled media |
| Detection pipeline | Systems seam implemented | `DeterministicFakeInference` is the only inference implementation; no production accuracy claim |
| ROI, rules, heatmaps, analytics | Not implemented | Blocked on product semantics, including what a heatmap means |
| Authentication and authorization | Server authorization implemented; local browser login available | Production identity provider and TLS/deployment boundary are not packaged |
| Packaging and operations | Partial runbooks and metrics exist | No confirmed Linux target, service package, upgrade/rollback, or complete backup runbook |
| Scale | Architecture and bounded behavior are designed for measurement | 10- and 20-camera support must be demonstrated, not inferred |

The roadmap is the authoritative item-by-item status record. A green unit-test
gate does not execute the ignored external fixtures and does not close hardware
or product-decision gates.

## Using the dashboard

After login, the dashboard provides the implemented operator views for live
cameras, camera state, playback search, settings, system state, and identity
administration. Availability of actions depends on the authenticated role,
camera scope, enabled server surfaces, and configured dependencies.

For the local stack:

- **Live view** requests a short-lived viewer token and subscribes through
  LiveKit. Transcoding runs only while a remote participant is present.
- **Playback** searches PostgreSQL for all segments overlapping the requested
  camera and time range, then streams assembled MP4 content.
- **Settings** reads resolved values and their origins. The local launcher opts
  into the normally disabled mutating surface so the development screen can save
  operational settings.
- **System** reports server capabilities and dependency state.
- **Logout** revokes the in-memory development session.

Detection is disabled by `make run` because the repository has no production
model. Enabling fake inference would spend decode resources and display
synthetic facts as if they represented a real detector.

## HTTP API

Use the dashboard through port `3001` for interactive development. For scripted
API work, authenticate through the configured proxy session or use an authorized
bearer token. The examples below show route shapes, not a production credential
workflow.

### Public routes

| Method | Route | Purpose |
|---|---|---|
| `GET` | `/` or `/ui` | Dashboard |
| `GET` | `/ui/livekit-client.js` | Browser LiveKit client asset |
| `GET` | `/live` | Process liveness |
| `GET` | `/ready` | Dependency and camera-aware readiness |
| `GET` | `/metrics` | Prometheus text metrics |

### Protected camera and recording routes

| Method | Route | Purpose |
|---|---|---|
| `GET` | `/api/cameras` | List cameras visible to the principal |
| `POST` | `/api/cameras` | Add a camera |
| `GET` | `/api/cameras/{camera_id}` | Camera resource |
| `GET` | `/api/cameras/{camera_id}/health` | Per-camera health |
| `POST` | `/api/cameras/{camera_id}/start` | Start ingest |
| `POST` | `/api/cameras/{camera_id}/stop` | Stop ingest |
| `DELETE` | `/api/cameras/{camera_id}` | Cancel, join, and remove a camera |
| `POST` | `/api/cameras/{camera_id}/viewer-token` | Mint a short-lived LiveKit viewer token |
| `GET` | `/api/cameras/{camera_id}/recordings?start={RFC3339}&end={RFC3339}&cursor={opaque}` | Search overlapping segments |
| `GET` | `/api/cameras/{camera_id}/recordings/{recording_id}` | Recording metadata |
| `GET` | `/api/cameras/{camera_id}/recordings/playback?start={RFC3339}&end={RFC3339}` | Stream the selected recorded range |

### Protected operations routes

| Method | Route | Purpose |
|---|---|---|
| `GET` | `/api/system` | Capabilities, storage, and dependency state |
| `GET` | `/api/settings` | Resolved settings and origins |
| `GET` | `/api/settings/{setting_key}` | One setting's stored and effective state |
| `PUT` | `/api/settings` | Transactional settings batch; surface is default-off |
| `GET` | `/api/identities` | List identities |
| `POST` | `/api/identities` | Create an identity |
| `GET` | `/api/identities/{principal_id}/tokens` | List token metadata |
| `POST` | `/api/identities/{principal_id}/tokens` | Mint a bearer token |
| `DELETE` | `/api/identities/{principal_id}/tokens/{token_id}` | Revoke a token |
| `POST` | `/api/identities/{principal_id}/disable` | Disable an identity |
| `POST` | `/api/identities/{principal_id}/enable` | Enable an identity |

Inputs are validated before invoking domain behavior. Camera credentials and
storage paths are never returned as camera resources. Out-of-scope camera access
uses `404` to avoid revealing that another camera exists.

## Recordings and playback

SENTINEL records the original encoded H.264 stream when compatible with the
fragmented-MP4 muxer. Segments are written under deterministic camera/UTC paths,
finalized atomically where practical, probed for actual metadata, and indexed in
PostgreSQL. Partial files remain as reconciliation evidence after abrupt exit.

List finalized recordings for the selected Make camera:

```bash
make recordings
```

Decode every finalized recording to detect corrupt media:

```bash
make verify-recordings
```

Play a specific file locally:

```bash
make play FILE=recordings/mac-webcam/2026/08/18/example.mp4
```

Time-range playback selects every indexed segment overlapping the requested
range and streams assembled MP4 bytes. The current policy is segment-aligned and
stream-copy oriented. It does not promise arbitrary-frame accuracy or a fully
seekable synthesized file.

## Running components manually

The convenience components remain available as separate long-running targets
for focused debugging. Run each long-lived target in its own terminal:

```bash
make mediamtx
make webcam WEBCAM_DEVICE=0
make server
```

The standalone `make server` path does not create PostgreSQL, LiveKit, an
identity, or a browser session for you. It is useful for component work, not the
recommended full dashboard workflow.

Inspect operational endpoints:

```bash
make live
make ready
make metrics
make health
```

Run `make help` for all targets and local fixture overrides.

## Running against an existing RTSP camera

The server requires a camera identity, RTSP endpoint, and actual encoded
dimensions. Do not embed credentials in the URL. Supply username and password as
a complete pair through the secret boundary.

For a focused loopback server run:

```bash
export SURVEILLANCE_CAMERA_ID=camera-01
export SURVEILLANCE_RTSP_URL=rtsp://127.0.0.1:8554/camera-01
export SURVEILLANCE_VIDEO_WIDTH=1920
export SURVEILLANCE_VIDEO_HEIGHT=1080
export SURVEILLANCE_RECORDING_ROOT=./recordings
export SURVEILLANCE_HEALTH_BIND=127.0.0.1:3000

# Only when the camera requires authentication:
# export SURVEILLANCE_RTSP_USERNAME=operator
# export SURVEILLANCE_RTSP_PASSWORD='injected-by-secret-manager'

RUST_LOG=info cargo run -p surveillance-server
```

This direct command does not establish the full browser authentication or
LiveKit environment. For a deployment, configure the PostgreSQL repository,
LiveKit credentials, trusted reverse-proxy boundary, principals, and secret
store according to ADR 009 rather than copying local fixture secrets.

The H.264/TCP path is the qualified MVP. Probe and validate each real camera's
codec, profile, resolution, frame rate, bitrate, keyframe interval, audio needs,
and reconnect behavior before adding it to a supported-camera claim.

## Configuration

Copy `config/example.toml`, edit it, and point the server to it:

```bash
cp config/example.toml config/local.toml
export SURVEILLANCE_CONFIG_FILE="$PWD/config/local.toml"
```

Configuration precedence is:

```text
compiled defaults < configuration file < persisted settings < environment
```

An API update writes the persisted layer. If an environment value shadows that
change, the response reports that it was persisted but is not currently
effective. Multi-field updates are transactionally validated and rejected as a
whole when an invariant fails.

Secrets do not belong in ordinary settings responses. The secret store enforces
`0600` permissions and supports per-key environment overrides, including
per-camera credentials. Never commit real database passwords, camera passwords,
LiveKit secrets, bearer tokens, proxy proofs, or fingerprint salts.

The versioned example documents the supported keys, boundary timing, and
file-layer behavior:

- `config/example.toml` — deployment-oriented configuration example
- `crates/config/src/classification.rs` — setting identities and application
  boundaries
- `crates/config/src/secret_store.rs` — secret-file and environment behavior

## Logs, metrics, and health

For `make run`, inspect:

```text
.local/run/logs/build.log
.local/run/logs/server.log
.local/run/logs/proxy.log
.local/run/logs/postgres.log
.local/run/logs/livekit.log
.local/run/logs/mediamtx.log
.local/run/logs/webcam.log
```

`/live` answers whether the process can serve requests. `/ready` reports whether
required dependencies and camera state are ready; an individual camera can be
degraded without taking down unrelated recording pipelines. `/metrics` exports
bounded-cardinality Prometheus metrics for camera, queues, recording, storage,
detection, LiveKit, API, security, and reconciliation behavior.

Startup recording reconciliation runs as an owned background repair task rather
than blocking `/live`. Already indexed immutable files are skipped when their
path and size evidence still match; unindexed durable files are probed before
import. Readiness and reconciliation metrics expose degraded repair state.

## Troubleshooting

### Login says `Forbidden`

- Use exactly `http://127.0.0.1:3001/` or `http://localhost:3001/`.
- Restart `make run` after changing proxy code; the running Python process does
  not reload itself.
- Use the username printed in the `AUTH` line. It defaults to `operator`.
- Use the supplied `SENTINEL_DEV_PASSWORD`, or the generated password printed by
  that specific run.
- Do not browse through `0.0.0.0`, another hostname, or port `3000`.
- Inspect `.local/run/logs/proxy.log` without copying session cookies or secrets
  into an issue.

### Direct API says `401 Unauthorized`

This is expected on port `3000`. Protected routes require either a valid bearer
token or a trusted assertion from the configured proxy. Use port `3001` for the
local browser session.

### Startup stops at the webcam

- Grant the terminal camera permission in macOS settings.
- Run `make webcam-devices` and select the correct `WEBCAM_DEVICE`.
- Inspect `.local/run/logs/webcam.log` and `.local/run/logs/mediamtx.log`.
- Close software that holds exclusive access to the camera.

### A port is already in use

The launcher refuses to attach to an old server, proxy, LiveKit, or MediaMTX
process because stale processes can carry stale secrets. It prints the owning
PID. Confirm that it belongs to the previous development run, then stop that
specific process and restart SENTINEL.

### The first build is slow

The first Rust build compiles the dependency graph. Later `make run` invocations
reuse `target/debug`. Use `make run-release` only when an optimized binary is
needed; it takes longer to build.

### `/live` starts before old recordings finish reconciliation

This is intentional. Reconciliation is background repair and does not own HTTP
availability. Check `/ready`, `/metrics`, and `server.log` for its state.

### Playback has gaps or cannot seek precisely

Confirm that indexed segments overlap the requested UTC range and that the files
still exist. Current playback is segment-aligned and does not guarantee
frame-exact boundaries or arbitrary browser seeking.

## Development and validation

Run the complete local quality gate:

```bash
make check
```

It runs shell and proxy checks, Rust formatting, Clippy with warnings denied,
workspace tests, a release build, `cargo deny`, and `cargo audit`.

The equivalent core Rust commands are:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --workspace --release
```

Ignored tests require external programs or services such as FFmpeg/libx264,
MediaMTX, PostgreSQL, or LiveKit. `cargo test` reports them but does not execute
them. Run the corresponding fixture when changing that integration:

| Area | Fixture |
|---|---|
| RTSP ingest, recording, and reconnect | `scripts/phase0/` |
| Multi-camera isolation | `scripts/phase3/multi-camera-fixture.sh` |
| PostgreSQL, playback, reconciliation, and retention | `scripts/phase4/postgres-fixture.sh` |
| LiveKit publication and outage isolation | `scripts/phase5/` |
| Persistent configuration | `scripts/phase7/config-fixture.sh` |
| Security boundary | `scripts/phase8/security-fixture.sh` |
| Authenticated dashboard and capacity evidence | `scripts/phase9/` |

Use the workload, hardware, codec, resolution, FPS, bitrate, camera count, viewer
count, and test duration when reporting results. Architecture alone is not scale
evidence.

## Repository map

```text
apps/server       current modular-monolith executable and HTTP surface
apps/worker       empty stub; no separately deployable worker exists
crates/auth       principals, roles, scope, and bearer-token verification
crates/camera     camera lifecycle and one-owner ingestion
crates/config     layered settings and secret handling
crates/database   PostgreSQL repositories and migrations
crates/detector   sampled detection pipeline and fake inference seam
crates/events     bounded domain-event contracts
crates/livekit    independent live-publication consumer
crates/media      encoded/decoded media and bounded distribution
crates/recorder   segmented fragmented-MP4 recording
crates/storage    metadata, retrieval, reconciliation, and retention
migrations        versioned PostgreSQL schema
scripts           local stack and external qualification fixtures
docs/adr          accepted architecture decisions
context           dated repository handoffs
```

`crates/api`, `crates/analytics`, `crates/rules`, and `apps/worker` are empty
stubs. The HTTP implementation currently lives in
`apps/server/src/health_http.rs`. The original `cam_rtsp` prototype remains in
the root `src/`, `examples/`, and `tests/` directories outside the workspace. It
is reference material, is not compiled, and is not shipped.

## Sources of truth

Read these in order when making or reviewing changes:

1. `AGENTS.md` — engineering rules and non-negotiable boundaries.
2. `ROADMAP.md` — implementation status and acceptance gates.
3. `context/README.md` — latest verified repository handoff.
4. `docs/adr/` — accepted decisions and reversal criteria.
5. `analysis.md` — target architecture proposal, not current implementation.

Useful durable references include `docs/architecture.md`,
`docs/requirements.md`, the Phase 8 threat model and runbooks, and the Phase 9
qualification evidence. Historical handoffs may describe older behavior; prefer
the latest dated context snapshot and the current code.

## Known open gates

Before a production readiness claim, SENTINEL still needs at least:

- representative camera qualification, including the required codec/profile
  matrix and reconnect behavior;
- sustained 10-camera and 20-camera measurements on named target hardware;
- viewer first-picture, reconnect, latency, and concurrency objectives with
  repeatable browser-side evidence;
- a selected production detection model, hardware/runtime, labeled dataset, and
  acceptance criteria;
- product definitions for ROI, rules, tracking, occupancy, and heatmaps;
- confirmed deployment OS, storage layout, service packaging, TLS/identity
  provider, secret ownership, upgrade/rollback, and backup/restore;
- real disk-full, network-partition, and combined shutdown qualification;
- execution of alert-rule tests with `promtool` and rendering/review of the
  supplied dashboards.

Until those gates close, describe the system as an implemented and locally
qualified H.264 MVP with explicit unverified production boundaries—not as a
production-certified 10-, 20-, or 200-camera platform.
