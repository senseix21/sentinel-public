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

## Run locally

With the prerequisites installed, run from the repository root:

```bash
make run
```

No webcam, camera IP or display is required. This starts the dashboard,
PostgreSQL, LiveKit and development login proxy with no bootstrap camera. Open
**Cameras** to add an RTSP endpoint, dimensions and optional camera credentials.
New cameras are saved stopped; click **Start** to connect. The server must be
able to reach the camera's address, through its LAN or a configured VPN/tunnel.
A laptop webcam has no RTSP IP unless you explicitly publish it through a bridge.

Wait for **SENTINEL ONLINE**, then open **http://127.0.0.1:3001/** and log in:

- Username: `operator`
- Password: `sentinel-local`

The command starts the database, LiveKit, server and development login proxy.
It does not open a capture device, download models, or start a decoder while
there are no running cameras. Keep this terminal open.

In another terminal, check the running system:

```bash
cd /Users/abuhamza/dev/camera_project
make live
make ready
```

Open **Live** in the dashboard to view the camera. After at least 60 seconds,
open **Playback** to search saved recordings. To stop, press **Ctrl-C once** in
the startup terminal and wait for the shutdown messages. Start again with the
same command to reuse the saved database and recordings.

For a startup failure, inspect `.local/server-run/logs/build.log` and
`.local/server-run/logs/server.log`. For device selection, optional settings,
and recording checks, follow [the detailed startup guide](#start-the-system-step-by-step).
The [latest verification](#latest-local-verification--2026-10-23) records the
remaining RTSP recovery, shutdown, build, and dependency-check failures.

## Run on the dev server and open the dashboard

No camera or display is needed to start the application. Use two terminals on
your laptop: one keeps the server running, and the other keeps the tunnel open.

### 1. Start the application on the server

In terminal A on your laptop:

```bash
ssh root@172.104.62.66
```

Then, inside that SSH session:

```bash
cd /home/dev/camera_project
make run
```

Wait for **SENTINEL ONLINE** and leave this terminal open. On this provisioned
server, the launcher automatically switches to the existing `sentinel-test`
repository owner before starting PostgreSQL. LiveKit automatically falls back
to loopback on a public-only server, so no `LIVEKIT_MEDIA_IP` override is needed
for dashboard startup.

If the optional synthetic demo is running, stop it before `make run`, since it
uses the same ports:

```bash
systemctl stop sentinel-demo.service sentinel-demo-source.service
```

### 2. Forward the dashboard port from your laptop

In terminal B on your laptop, outside the SSH session:

```bash
ssh -N -o ExitOnForwardFailure=yes \
  -L 127.0.0.1:3001:127.0.0.1:3001 root@172.104.62.66
```

Enter the SSH password and leave this terminal open. A blank terminal after
authentication is normal. Reuse an existing tunnel on port 3001 if one is
already running.

### 3. Open the dashboard and configure cameras

Open **http://127.0.0.1:3001/** in your laptop's browser and sign in:

- Username: `operator`
- Password: `sentinel-local`

An empty camera list is normal on first startup. Open **Cameras**, enter a camera
identifier, RTSP endpoint, stream dimensions and optional camera credentials,
then click **Add camera** followed by **Start**. The server must be able to reach
the camera's network address. Configuration and running/stopped choices survive
restarts; starting the dashboard itself never requires a webcam.

This tunnel provides dashboard/API access and HTTP recording playback. Live
WebRTC viewing also requires reachable LiveKit signalling and media endpoints;
forwarding port 3001 alone does not configure those connections.

### 4. Stop when finished

Press **Ctrl+C** in terminal A and wait for the shutdown messages to stop the
application. Press **Ctrl+C** in terminal B to close the tunnel. Closing only the
tunnel leaves the application running. Reuse `make run` and the same tunnel
command the next time.

## Play existing dev-server recordings without a camera

The following footage was found on the server on **2026-11-01**. These camera
identifiers are the names used for playback:

- **`dev-test-pattern`** — 1,372 synthetic demo recordings from **October 31
  through November 1, 2026 (UTC)**, stored in
  `/home/dev/camera_project/.local/server-demo/recordings/`.
- **`camera-01`** — 5 copied laptop recordings from **October 29, 2026 (UTC)**,
  around **17:46–17:48 UTC**, stored in
  `/home/dev/camera_project/.local/laptop-recordings/`. These copies have not
  been indexed for Playback; copying files alone does not add them to its list.

Counts describe that inventory check; configured retention may remove footage
later. Normal `make run` uses `.local/server-run`, whose recording index was
empty at the time of the check. It does not automatically load the separate
demo database or laptop copies.

To view the existing **`dev-test-pattern`** footage:

1. Stop the current `make run` with **Ctrl+C**. If the optional demo services
   are running, stop them too, so the same ports and state are not used twice:

   ```bash
   systemctl stop sentinel-demo.service sentinel-demo-source.service
   ```

2. On the server, reuse the demo's existing database and recording directory:

   ```bash
   cd /home/dev/camera_project
   STATE_DIR=/home/dev/camera_project/.local/server-demo \
   make run RECORDING_ROOT=/home/dev/camera_project/.local/server-demo/recordings
   ```

3. Keep your port-3001 SSH tunnel open, open **http://127.0.0.1:3001/**, and
   sign in with the development login above.
4. Open **Playback**, select **`dev-test-pattern`**, choose **UTC**, and select
   **2026-10-31** or **2026-11-01**. Choose a time range containing saved footage
   and click **Load**, then **Play recording**.

No connected camera is required to play these saved recordings. This command
serves existing footage; the optional source demo below is used to generate new
synthetic footage.

## Optional: open the synthetic recording demo, step by step

The headless dev server does not need a display or webcam to serve saved video.
You watch recordings in your laptop's browser through an SSH tunnel.

For normal dashboard startup, use the `make run` instructions above. Stop that
stack before starting this optional demo, which uses the same ports and has its
own database and recording directory.

This demo guide applies to the **already-provisioned demo server** at
`172.104.62.66`. Its `sentinel-demo` and `sentinel-demo-source` systemd services
were installed separately; a fresh clone does not create those services.
The source is generated video labeled **TEST FOOTAGE - NOT A LIVE CAMERA**,
with camera ID `dev-test-pattern`. Detection is disabled for this demo.

### 1. Check the demo on the server

From your laptop, connect:

```bash
ssh root@172.104.62.66
```

Enter the SSH password shared privately. Nothing appears while you type the
password; press Enter when finished. Do not put that password in this README.

At the server's `root@localhost` prompt, run:

```bash
cd /home/dev/camera_project
systemctl start sentinel-demo.service
systemctl is-active sentinel-demo.service sentinel-demo-source.service
```

Both services should report `active`. The first startup can take time; inspect
progress with:

```bash
journalctl -u sentinel-demo.service -n 30 --no-pager
```

The synthetic recording demo is separate from normal application startup.
For a camera-free dashboard, stop both demo services and use:

```bash
cd /home/dev/camera_project
make run
```

On this provisioned server the launcher switches from root to the existing
repository owner, `sentinel-test`, before starting PostgreSQL. No user is created.
For another deployment, run as an unprivileged repository owner or set
`SENTINEL_DEV_USER` when invoking the launcher as root.

### 2. Open the SSH tunnel from your laptop

Open a **second terminal on your laptop**, outside the SSH session, and run:

```bash
ssh -N -o ExitOnForwardFailure=yes \
  -L 127.0.0.1:3001:127.0.0.1:3001 root@172.104.62.66
```

Enter the server's SSH password again. After authentication, a blank terminal
is normal: `-N` opens the tunnel without a remote shell. **Leave it open.**
If this tunnel is already running, reuse it instead of starting another copy.

### 3. Sign in through your laptop's browser

Open **http://127.0.0.1:3001/** on your laptop. This address reaches the server
through the tunnel; it is not a public dashboard address.

Use the development dashboard login:

- Username: `operator`
- Password: `sentinel-local`

These are dashboard development credentials, separate from the SSH password.

### 4. Load the demo recordings

1. Open **Playback**.
2. Select camera **dev-test-pattern**.
3. Set **Timezone** to **UTC**.
4. Select the UTC date when the footage was recorded. For new footage, use the
   current UTC date, which can differ from your local calendar date.
5. If your browser shows a **12-hour clock**, set **From** to **12:00 AM** and
   **To** to **11:59 PM**. Set the AM/PM fields explicitly. If seconds are shown,
   use **12:00:00 AM** through **11:59:59 PM**.
6. If your browser shows a **24-hour clock**, use **00:00** through **23:59**.
7. Click **Load**.
8. In the matched segments, click **Play recording**.

**12:00 AM means midnight; 12:00 PM means noon.** These times are interpreted
in the selected timezone, so keep UTC selected for the instructions above.

Allow at least 30-60 seconds after startup for a recording segment to finalize.
The demo retains approximately one day of footage; an older date may have no
remaining recordings. This guide demonstrates recording and playback, not a
physical camera or live detection. The HTTP tunnel alone does not configure
remote WebRTC live viewing.

### 5. Close the tunnel or stop the demo

Press **Ctrl+C** in the laptop's tunnel terminal to close your connection.
The server continues recording until its service is stopped.

To stop the demo, run this **on the server**:

```bash
systemctl stop sentinel-demo.service
```

The source service stops with it. Existing recordings are preserved until the
configured retention policy removes them. Start it again with
`systemctl start sentinel-demo.service`.

If the tunnel reports **Address already in use**, an existing tunnel or local
application is using laptop port 3001. Reuse the existing demo tunnel, or close
the previous tunnel with Ctrl+C before reconnecting.

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

## Start the system, step by step

`make run` starts with no bootstrap camera. It restores dashboard registrations
and resumes only cameras whose saved desired state is running. Stopped and
unreachable cameras do not block login or `/ready`; inspect their camera rows
and health resources for connection status.

Optional webcam testing uses **`make webcam`** (or `make webcam-release`):

```text
webcam -> server-owned FFmpeg -> bounded H.264 distributor
                                      |-> local MP4 recording
                                      |-> detection
                                      `-> LiveKit -> browser WebRTC
```

FFmpeg handles operating-system capture and H.264 encoding. The Rust camera
pipeline owns the child process, parsing, lifecycle, reconnect epochs, queues,
recording, and downstream consumers.

Run these commands from the repository root. Use two terminals: keep the stack
running in terminal A and run checks in terminal B. Do not start a second stack
on the same ports.

### 1. Check prerequisites

Required commands:

- Rust 1.88 or newer (`cargo` and `rustc`)
- PostgreSQL (`initdb`, `pg_ctl`, and `psql`)
- FFmpeg (`ffmpeg`, `ffprobe`, and optionally `ffplay`)
- LiveKit Server (`livekit-server`)
- Python 3 (`python3`), Node.js (`node`, for dashboard tests), `curl`, and `lsof`

On macOS, install the third-party services with your preferred package manager,
then verify the basic media tools:

```bash
make prerequisites
rustc --version
node --version
python3 --version
```

`make prerequisites` must exit successfully. It checks executables, not service
health, Node.js, model accuracy, or camera permission. If PostgreSQL is installed
but its commands are missing, add that installation's `bin` directory to `PATH`.
The dependency security gate additionally needs `cargo-deny` and `cargo-audit`;
see [the security test steps](#4-run-the-dependency-and-security-gate).

The launcher checks FFmpeg, LiveKit, and port-inspection dependencies before
changing runtime state.

### 2. Select a source (optional)

For a webcam smoke test on macOS, list AVFoundation devices. Skip this step
for a headless server or dashboard-configured RTSP cameras:

```bash
make webcam-devices
```

The launcher defaults to video device `0` at 1280×720 and 30 FPS for local development.
Override `VIDEO_FPS` with a mode supported by your camera. On first
use, macOS may ask for camera access.
Grant the terminal application access under **System Settings -> Privacy &
Security -> Camera**.

### 3. Start SENTINEL in terminal A

Start the application without a camera:

```bash
make run
```

Or explicitly start the webcam fixture with a camera identity:

```bash
CAMERA_ID=camera-01 WEBCAM_DEVICE=0 make webcam
```

The launcher opens the development login page automatically:

```text
http://127.0.0.1:3001/
```

Wait for **SENTINEL ONLINE**, then open the printed dashboard URL and log in.
The default local login is `operator` / `sentinel-local`; the launcher prints the
effective credentials. These are development credentials only.

The stack starts PostgreSQL, LiveKit, the Rust server, and a loopback development
login proxy so recording search, playback, settings, and authenticated dashboard
requests work locally. By default it verifies/downloads the development models
and loads ONNX Runtime before building the server. First startup needs network
access when these artifacts are not cached. Logs are under
`.local/webcam-run/logs/`; inspect `build.log` if compilation fails.

Keep the terminal running. Press Ctrl-C once to stop the server-owned webcam
process and the launcher-owned LiveKit process. Recordings are preserved.

### 4. Check health in terminal B

```bash
make live
make ready
make metrics
```

Each command must exit successfully. Liveness alone does not prove recording or
video delivery. Readiness should report the camera connected; metrics should
show media progressing and recording segment completions after a segment boundary.
An empty recording list immediately after startup is normal: wait at least
60 seconds, then continue with the verification steps below.

### 5. Check the dashboard

1. Open `http://127.0.0.1:3001/#live` and log in through port **3001**.
2. Confirm moving live video, not just a connected status or a still frame.
3. Open a second tab and confirm both keep playing for at least 20 seconds.
4. With detection enabled, confirm the person/face capability readout appears.
   Face boxes require a visible face; person presence may continue without one.
   A zero count is valid for an empty scene and does not qualify accuracy.
5. Open Playback after a segment finalizes. Search the camera and the current
   UTC time range, then play a recording. Confirm its image and duration.
6. Check System and Settings for dependency state and resolved configuration.

For the automated two-tab decode check, follow [test step 5](#5-check-real-browser-decoding).
Physical webcam turn-away and Firefox remain separate manual checks.

### 6. Stop and verify saved recordings

Press Ctrl-C once in terminal A and wait for shutdown to finish. The launcher
joins the server and its FFmpeg source, login proxy, LiveKit, and PostgreSQL.
It preserves recordings and database state for the next run.

```bash
CAMERA_ID=camera-01 make recordings
CAMERA_ID=camera-01 make verify-recordings
```

The second command must decode every finalized MP4 without errors. A
`.partial.mp4` is excluded; inspect server logs and reconciliation state if it
remains after graceful shutdown. Do not delete partials to make a check pass.
Restart with the same command and confirm historical recordings remain searchable.

### 7. Override local fixture values

All values are optional:

```bash
WEBCAM_DEVICE=1 \
VIDEO_WIDTH=1920 \
VIDEO_HEIGHT=1080 \
VIDEO_FPS=15 \
VIDEO_BITRATE_KBPS=2500 \
CAMERA_ID=office-webcam \
make webcam
```

Use the development build for the normal edit/run loop:

```bash
make run
```

Use the optimized binary for manual qualification or performance measurements.
On the currently affected macOS toolchain, use the verified release workaround:

```bash
CARGO_PROFILE_RELEASE_STRIP=none CAMERA_ID=camera-01 make webcam-release
```

On hosts without the documented SQLx linker issue, use
`CAMERA_ID=camera-01 make webcam-release`.

Do not publish performance numbers from `make run`; it deliberately uses the
faster-to-build development profile.

## What `make run` starts

The launcher owns and supervises this local stack:

| Component | Default endpoint or path | Responsibility |
|---|---|---|
| PostgreSQL | `127.0.0.1:55432` | Recording index, settings, identities, and audit |
| LiveKit | `ws://127.0.0.1:7880` | WebRTC distribution to browser viewers |
| FFmpeg child (only with `make webcam`) | selected webcam | Captures and encodes H.264 into the Rust source owner |
| SENTINEL server | `http://127.0.0.1:3000` | API, dashboard assets, ingest, recording, and publication |
| Development proxy | `http://127.0.0.1:3001` | Loopback login and trusted identity assertion |

Persistent local state for `make run` defaults to:

```text
.local/server-run/logs        service logs
.local/server-run/pgdata      persistent PostgreSQL data
.local/server-run/cameras.json protected camera configuration and desired state
recordings/                  finalized and partial recording segments
```

`make webcam` retains `.local/webcam-run` for its separate development fixture.
Set `STATE_DIR` and `RECORDING_ROOT` explicitly to reuse an existing stack's
state. Never run two stacks on the same state directory or listener ports.
Camera credentials live in the catalog, which requires mode **0600**. Back it up
as a secret, together with the database and recordings; do not commit it. Set
`SURVEILLANCE_CAMERA_CATALOG_PATH` to select another location. A standalone Rust
server defaults to `.local/cameras.json`. Catalog corruption or insecure
permissions fails startup rather than silently discarding registrations.
Start/Stop update durable intent; Ctrl-C preserves it. Remove stops the camera
and deletes registration, while indexed footage remains selectable in Playback.
See [ADR 019](docs/adr/019-camera-independent-startup.md).

The HTTP server and development login proxy listen on loopback. LiveKit WebRTC
media uses the machine's active private IPv4 address, with UDP on `LIVEKIT_PORT + 2`
(7882 by default); TCP fallback is disabled. This supports browsers that reject
loopback ICE. Media still requires a signed, camera-scoped viewer token.

Set `LIVEKIT_MEDIA_IP=127.0.0.1` for loopback-only media, or choose a private IPv4
address assigned to this machine to override automatic route selection. The
launcher prints the selected media address. With loopback-only media, Firefox
may need `media.peerconnection.ice.loopback=true` in `about:config`. Multiple
tabs receive distinct viewer identities so they cannot evict one another.

To qualify actual browser decoding while the stack is running, supply Playwright
from a separate installation and run:

```bash
PLAYWRIGHT_MODULE_PATH=/path/to/node_modules/playwright make verify-live-browser
```

Install Playwright's Chromium browser in that installation first. The check logs
in, uses a separate development viewer identity, and requires playback and decoded
frame counts to advance across multiple dashboard refreshes.

## Security model

### Local development

`make run` binds its HTTP, database, and LiveKit signalling listeners to loopback
and serves the development login at
`http://127.0.0.1:3001/`. The launcher prints its local credentials; override
them with `SENTINEL_DEV_USERNAME` and `SENTINEL_DEV_PASSWORD`. This is not a
deployable authentication design.

The Phase 8 security fixtures separately exercise bearer authentication and the
identity-aware reverse-proxy boundary with PostgreSQL-backed principals. Those
fixtures remain the evidence for the production authorization contract.

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
RTSP camera ----------------------.
                                    v
Local webcam -> owned FFmpeg -> Camera ingestion owner (one source per camera)
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
| H.264 camera ingest | RTSP/TCP and server-owned FFmpeg webcam sources implemented | Representative cameras, non-H.264 codecs, and 10/20-camera qualification remain |
| Local recording | H.264 MVP implemented | Deployment filesystem and genuine disk-full behavior remain unqualified |
| Recording index and retention | Implemented with PostgreSQL reconciliation and watermarks | Backup/restore and target storage sizing remain open |
| Live browser video | Implemented through viewer-gated transcoding and LiveKit | First-picture time, viewer-visible reconnect, latency, representative cameras, and production concurrency remain open |
| Time-range search and playback | Implemented with segment-aligned stream-copy semantics | No frame-exact arbitrary boundaries; browser seeking is limited by the assembled media |
| Detection pipeline | Systems seam implemented | Real YOLOX person and YuNet face ONNX adapters available; fake backend retained for deterministic tests; production accuracy unqualified |
| ROI, rules, heatmaps, analytics | Not implemented | Blocked on product semantics, including what a heatmap means |
| Authentication and authorization | Server authorization implemented; local browser login available | Production identity provider and TLS/deployment boundary are not packaged |
| Packaging and operations | Partial runbooks and metrics exist | No confirmed Linux target, service package, upgrade/rollback, or complete backup runbook |
| Scale | Architecture and bounded behavior are designed for measurement | 10- and 20-camera support must be demonstrated, not inferred |

The roadmap is the authoritative item-by-item status record. A green unit-test
gate does not execute the ignored external fixtures and does not close hardware
or product-decision gates.

## Using the dashboard

The dashboard provides the implemented operator views for live
cameras, camera state, playback search, settings, system state, and identity
administration. Availability of actions depends on the authenticated role,
camera scope, enabled server surfaces, and configured dependencies.

For the local stack:

- **Live view** requests a short-lived viewer token and subscribes through
  LiveKit. Transcoding runs only while a remote participant is present.
- **Playback** searches the PostgreSQL recording index started by `make run`.
- **Settings** uses the persistent settings store and the launcher's explicit
  development-only mutating-surface opt-in.
- **System** reports server capabilities and dependency state.
- **Logout** revokes the in-memory development session.

`make webcam` and `make webcam-release` enable real ONNX inference by default.
Camera-free startup downloads no models; dashboard detection is disabled by
default. Enabling detection requires a separately configured backend; a build
without a model reports fixture inference and supports no accuracy claim.
They select **person presence with face-only boxes** (`person_face`) at a 70% confidence threshold. On first use
they download the checksum-verified YOLOX person model, YuNet face model and pinned
ONNX Runtime 1.28.3 into `.local/onnx/`. The runtime bundle supports Apple Silicon
macOS and Linux x64/ARM64; other hosts must supply `ORT_DYLIB_PATH`.
The default detection confidence threshold is 70%. Override it with
`SURVEILLANCE_DETECTION_FACE_SCORE_THRESHOLD_PERCENT` (0–100); changes require a
camera pipeline restart.
This development model has not been qualified for production surveillance accuracy.

Set `SURVEILLANCE_DETECTION_ENABLED=false` to skip inference and artifact downloads,
and `SENTINEL_OPEN_BROWSER=false` to skip opening the browser. Face overrides use
`SURVEILLANCE_DETECTION_FACE_MODEL_PATH` and
`SURVEILLANCE_DETECTION_FACE_MODEL_SHA256`; this adapter requires a YuNet-compatible
640×640 artifact. The live UI shows faces detected, with no recognition or unique
identity count. No outlines are burned into recordings.

Set `SURVEILLANCE_DETECTION_MODE=person` to retain the YOLOX person path. Its custom
model overrides are `SURVEILLANCE_DETECTION_MODEL_PATH` and
`SURVEILLANCE_DETECTION_MODEL_SHA256`, with `SURVEILLANCE_DETECTION_INPUT_WIDTH`
and `_HEIGHT` matching its geometry. Set `ORT_DYLIB_PATH` to use an installed
compatible runtime. See [face detection specification](docs/face-detection/SPEC.md)
and [ADR 016](docs/adr/016-face-only-live-detection.md).
[Combined mode](docs/person-presence/SPEC.md) uses independent person and face lanes.
Back-facing fixture evidence stays in the presence readout without drawing a body box.
See [qualification and remaining reliability gates](docs/person-presence/VERIFICATION.md).
Set `SURVEILLANCE_DETECTION_MODE=face` to restore face-only mode.

To qualify real face observations in the running browser alongside decoded video,
run `SENTINEL_EXPECT_FACE=true make verify-live-browser` with Playwright supplied
through `PLAYWRIGHT_MODULE_PATH`. See [verification evidence and open gates](docs/face-detection/VERIFICATION.md).

In face-only mode, overlay expiry defaults to three seconds. Set the validated duration
`SURVEILLANCE_CAMERA_DETECTION_OVERLAY_MAX_AGE` (for example `5s`, at most `60s`);
it applies after a pipeline restart. Combined mode uses the presence freshness
duration instead (one second by default, `camera.detection.presence_freshness_duration`).
Camera health exposes `detection_mode` and
`detection_overlay_max_age_ms`; face-mode live packets use `face/v1`.
The macOS release build currently requires `CARGO_PROFILE_RELEASE_STRIP=none`
to avoid the documented SQLx proc-macro linker issue.

## HTTP API

Use port `3001` for the loopback-only `make run` dashboard. In a configured
deployment, scripted API work must authenticate through the trusted proxy or an
authorized bearer token. The examples below show route shapes, not a production
credential workflow.

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
| `GET` | `/api/cameras/{camera_id}/recordings/{recording_id}/video[?duration_seconds=N]` | Stream a full recording or its first 1–60 seconds |

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

## Running the server manually

Supply a direct camera endpoint and run the server:

```bash
RTSP_URL=rtsp://camera-host:554/stream \
CAMERA_ID=camera-01 \
VIDEO_WIDTH=1920 \
VIDEO_HEIGHT=1080 \
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

For `make run`, inspect `.local/server-run/logs/`:

```text
build.log
server.log
livekit.log
postgres.log
proxy.log
```

For `make webcam`, the equivalent directory is `.local/webcam-run/logs/`.
`/live` answers whether the process can serve requests. `/ready` reports
application dependency readiness independently of camera count or connectivity.
Use camera resources and `/metrics` for camera degradation, queue, recording,
storage, detection, LiveKit, security and reconciliation status.

Startup recording reconciliation runs as an owned background repair task rather
than blocking `/live`. Already indexed immutable files are skipped when their
path and size evidence still match; unindexed durable files are probed before
import. Readiness and reconciliation metrics expose degraded repair state.

## Troubleshooting

### Direct API says `401 Unauthorized`

With PostgreSQL or a non-loopback deployment boundary configured, this is
expected: protected routes require a valid bearer token or a trusted proxy
assertion. Use the development login on port `3001`; the development launcher starts PostgreSQL and
does not provide an anonymous administrator API on port `3000`.

### Startup stops at the webcam

- Grant the terminal camera permission in macOS settings.
- Run `make webcam-devices` and select the correct `WEBCAM_DEVICE`.
- Inspect `.local/webcam-run/logs/server.log`; owned FFmpeg failures are reported
  through the camera lifecycle.
- Close software that holds exclusive access to the camera.

### A port is already in use

The launcher refuses to attach to an old server or LiveKit process. It prints
the owning PID. Confirm that it belongs to the previous development run, then
stop that specific process and restart SENTINEL.

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

## Test the system, step by step

Run automated checks before starting the manual webcam smoke test. Store logs
outside committed source; the following examples use `target/verification/`.
A passing command must exit with status zero. Inspect its log before moving on;
redirected output is not evidence of success by itself.

### 1. Run deterministic checks

```bash
mkdir -p target/verification
make verify > target/verification/verify.log 2>&1
```

This checks shell syntax, Python helpers, dashboard JavaScript, Rust formatting,
default-feature Clippy/tests, and the default-feature release build. It does
**not** enable ONNX features, run ignored integration tests, or audit dependencies.
On the current macOS toolchain the plain release build has a known SQLx
proc-macro `mis-aligned LINKEDIT string pool` failure. Preserve that failure and
rerun using the explicit workaround:

```bash
CARGO_PROFILE_RELEASE_STRIP=none make verify > target/verification/verify-no-strip.log 2>&1
```

The workaround also applies to `make run-release`. It is not proof that the
plain release gate passes. Sandbox `Operation not permitted` errors from local
socket binding or child-process inspection require an execution environment
that permits those test operations; they are not application test results.

### 2. Check every Rust feature, including ONNX

After the launcher's first successful model/runtime preparation on Apple Silicon:

```bash
export ORT_DYLIB_PATH="$PWD/.local/onnx/onnxruntime-osx-arm64-1.28.3/lib/libonnxruntime.dylib"
test -f "$ORT_DYLIB_PATH"
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
CARGO_PROFILE_RELEASE_STRIP=none cargo build --workspace --all-features --release
```

On Linux, supply the actual compatible `libonnxruntime.so` path instead. Compiling
ONNX features does not prepare a loadable runtime: tests that initialize the
adapter need `ORT_DYLIB_PATH` too. `make verify-onnx` runs ONNX Clippy/tests with
that same runtime requirement. Model-based ignored tests additionally need their
pinned artifacts and frame fixtures; see
[person presence evidence](docs/person-presence/VERIFICATION.md).

### 3. Exercise real PostgreSQL and media integrations

These fixtures create temporary local databases, apply migrations and clean up
their own database processes. FFmpeg/libx264 and PostgreSQL commands must be on
`PATH`. Run them separately so each failure has its own log:

```bash
bash scripts/phase4/postgres-fixture.sh > target/verification/postgres.log 2>&1
bash scripts/phase7/config-fixture.sh > target/verification/config.log 2>&1
```

Phase 4 covers metadata constraints, overlapping searches, real MP4 playback,
unreadable-media refusal, reconciliation, retention, and database outage/recovery.
Phase 7 covers settings persistence, transactional audit, restart persistence,
and store-outage degradation. Neither demonstrates real disk-full behavior on
the eventual deployment filesystem.

### 4. Run the dependency and security gate

Install the audit tools if they are missing:

```bash
cargo install --locked cargo-deny
cargo install --locked cargo-audit
```

```bash
make audit
bash scripts/phase8/security-fixture.sh
make check
```

`make check` combines `make verify` and `make audit`; it stops at the first
failure. On affected macOS hosts also run
`CARGO_PROFILE_RELEASE_STRIP=none make check` and label that workaround explicitly.
The security fixture requires both audit tools before it reaches its database
checks. Missing tools, unavailable advisory feeds, or a policy violation mean
this gate is incomplete or failed; do not describe it as green.

### 5. Check real browser decoding

Start the stack using the startup steps above. Install Playwright into an
independent test directory, keeping the repository free of extra dependencies:

```bash
mkdir -p .local/browser-tests
npm install --prefix .local/browser-tests playwright
.local/browser-tests/node_modules/.bin/playwright install chromium
PLAYWRIGHT_MODULE_PATH="$PWD/.local/browser-tests/node_modules/playwright" make verify-live-browser
```

The test logs in through the proxy, opens two viewers, and verifies advancing
playback time and decoded-frame counters across dashboard refreshes. A successful
WebRTC connection alone is insufficient. With combined detection enabled:

```bash
SENTINEL_EXPECT_PRESENCE=true \
PLAYWRIGHT_MODULE_PATH="$PWD/.local/browser-tests/node_modules/playwright" \
make verify-live-browser
```

This requires real, available person observations and non-fixture face packets;
it does not require a positive face count in every image. Use
`SENTINEL_EXPECT_FACE=true` for `SURVEILLANCE_DETECTION_MODE=face`.
Playwright's package and installed browser revision must match.

### 6. Repeat the full stack with a file-backed RTSP source

This path exercises production RTSP ingestion without using a physical webcam.
Supply an installed MediaMTX binary and a known H.264 MP4. Use a separate state
and recording root to avoid mixing qualification evidence with webcam data.
Create `.local/rtsp-test.yml`:

```yaml
logLevel: info
rtspAddress: 127.0.0.1:8554
rtspTransports: [tcp]
rtmp: false
hls: false
webrtc: false
srt: false
paths:
  qualification:
    source: publisher
```

In terminal A, start MediaMTX:

```bash
mediamtx .local/rtsp-test.yml
```

In terminal B, loop the source in real time:

```bash
ffprobe -v error -select_streams v:0 -show_entries stream=codec_name,width,height,r_frame_rate -of json /absolute/path/source.mp4
ffmpeg -re -stream_loop -1 -i /absolute/path/source.mp4 \
  -c:v copy -an -rtsp_transport tcp -f rtsp rtsp://127.0.0.1:8554/qualification
```

Require H.264 and set the actual width/height below (1280×720 is an example).
In terminal C, start the normal stack against that fixture:

```bash
STATE_DIR="$PWD/.local/rtsp-test" \
RECORDING_ROOT="$PWD/.local/rtsp-test-recordings" \
CAMERA_ID=qualification-01 VIDEO_WIDTH=1280 VIDEO_HEIGHT=720 \
SENTINEL_RTSP_FIXTURE_URL=rtsp://127.0.0.1:8554/qualification \
SENTINEL_OPEN_BROWSER=false scripts/dev/local-webcam-stack.sh
```

Run the health, browser and playback checks from another terminal. The launcher
accepts only credential-free loopback RTSP fixture URLs. For the marked
front/back/empty detection sequence, build a source using
`scripts/dev/build-presence-scenes.sh FRONT_IMAGE COCO_77_IMAGE OUTPUT_MP4`, then
run `scripts/dev/check-presence-scenes.cjs` with the same Playwright environment.
That scene checker requires the helper's 1280×720 output and supplied public
images; an arbitrary video cannot satisfy its scene assertions.

Stop the stack first, then FFmpeg, then MediaMTX with Ctrl-C in their terminals.
Finally decode saved files:

```bash
RECORDING_ROOT=./.local/rtsp-test-recordings CAMERA_ID=qualification-01 make verify-recordings
```

### 7. Check recovery and sustained operation

For the isolated file-backed stack above, enable the MediaMTX API on loopback in
its YAML before starting MediaMTX:

```yaml
api: true
apiAddress: 127.0.0.1:9997
```

Then run the automated recovery fixture (replace the source file with the same
H.264 MP4 used by your publisher):

```bash
SENTINEL_RECOVERY_SOURCE=/absolute/path/source.mp4 \
SENTINEL_RTSP_FIXTURE_URL=rtsp://127.0.0.1:8554/qualification \
SENTINEL_RECOVERY_CAMERA=qualification-01 \
PLAYWRIGHT_MODULE_PATH="$PWD/.local/browser-tests/node_modules/playwright" \
node scripts/dev/check-rtsp-recovery.cjs
```

Run this only against the isolated development fixture: it disconnects the
selected MediaMTX publisher, starts an owned replacement FFmpeg publisher, and
stops that replacement when finished. Restart your original publisher afterward
if you want to keep using the fixture. The test keeps two Chromium viewers open,
requires a new camera epoch, advancing inference/publication, a complete new
recording segment, advancing viewer frames and exactly one upstream reader.
Recorder rejection/failure counters must not increase. It requires the ONNX
combined detection configuration and a source that the browser can decode;
this is a recovery check, not an accuracy or latency benchmark.


While the file-backed stack is recording, stop only the RTSP publisher. Confirm
liveness remains available, camera health reports degradation/reconnection, and
logs explain the source loss. Restart the publisher and confirm a new connected
epoch and new playable segments. Repeat the browser check after recovery.

Keep a named workload running long enough to investigate the known recorder
backpressure failure. Save `/metrics`, server/LiveKit logs, duration, completed
segments and any partials. A short smoke test cannot close that reliability gate.
Do not increase queue capacity to hide drops. Physical webcam, Firefox, genuine
disk-full and 10/20-camera target-hardware tests remain separate qualifications.

## Latest local verification — 2026-10-23

The RTSP reconnect stalls observed on October 20 are fixed for the local fixture.
Detection teardown now disconnects its full frame queue before joining the reader.
Live publication resets pacing, RTP mapping and decoder state together at a new
media run, keeping the existing room and track. Combined detection reports
Degraded while resetting or failing decode and Running after decoded frames return.

Two deterministic regressions first reproduced the old deadlock/publication
stall, then passed with the fixes; another covers detection health recovery.
All-feature workspace tests passed: **590 passed, 39 ignored**. Formatting,
all-target/all-feature Clippy, the default `make verify` gate and all-feature
release passed, using `CARGO_PROFILE_RELEASE_STRIP=none` for macOS release builds.
The two real FFmpeg decode tests were also executed explicitly and passed.

Three automated local outage/recovery cycles passed with real ONNX models and
two Chromium viewers. The final clean-state replay required a complete new
recording segment: camera epoch 1→2, inference 29→270, publication 27→631 access
units, completed segments 0→2, no recorder rejections/failures, and exactly one
surveillance RTSP reader. Both existing browser tabs continued advancing frames.
All 33 files from the earlier run and three clean-state files decoded. Rust server
and detection teardown finished within one second; whole-stack shutdown varied
from about one second to 26 seconds, with the latter waiting on LiveKit.
Use [the recovery fixture](#7-check-recovery-and-sustained-operation) to repeat it.

Raw logs are in `target/verification-2026-10-23/` (ignored local output).
Long source-offline gaps in the manually interrupted run produced two immutable
recording-metadata conflict errors; that separate indexing/reconciliation issue
needs investigation. This is not a sustained-load qualification. The older
recorder-backpressure incident, dependency advisories, plain macOS release issue,
physical-camera/Firefox checks and production capacity/accuracy gates remain open.

## Latest local verification — 2026-10-20

Tested on Apple Silicon macOS with Rust 1.92.0, Node.js 26.11.0 and Python 3.15.0.
The workload was one file-backed H.264 RTSP source, 1280×720 at 20 FPS, combined
YOLOX/YuNet CPU inference at 70%, local PostgreSQL/LiveKit and Chromium viewers.
This is development evidence, not a capacity or production-accuracy claim.

- Formatting and all-target/all-feature Clippy passed.
- Default-feature `make verify` passed with `CARGO_PROFILE_RELEASE_STRIP=none`:
  580 Rust tests passed, 34 ignored; 46 dashboard tests and 24 Python helper tests passed.
- All-feature Rust tests passed with the cached `ORT_DYLIB_PATH`: 587 passed,
  39 ignored. Two ignored real-model presence tests were run separately and passed.
- All-feature release build passed with stripping disabled. Plain `make verify`
  failed at the known macOS SQLx release linker issue.
- Phase 4 PostgreSQL/media fixture passed (13 tests), including corrupt-media
  refusal, playback, reconciliation, retention and database outage/recovery.
  Its unreadable-media test was corrected to use a valid nonempty `.mp4`
  metadata contract before asserting that FFmpeg rejects the corrupt bytes.
- Phase 7 configuration fixture passed (28 tests plus SQL checks).
- Login, authenticated API reads, recording search and a two-second HTTP MP4
  download passed; the downloaded media decoded without errors. Anonymous
  access to the direct camera API returned `401`.
- Chromium passed the two-viewer advancing-frame check and marked scene checks:
  front 3 people/2 faces, back 1 person/0 faces, empty 0/0. MediaMTX showed one
  surveillance RTSP reader.
- An intentional RTSP publisher outage preserved `/live`, degraded `/ready`,
  and recovered into a second camera connection epoch when publishing resumed.
  Recording resumed, but the post-recovery browser check **failed** after 45
  seconds. Live frame/publication and detection inference counters stopped
  advancing while health still reported Running/Publishing; live media
  rejections and detection drops increased. Recovery of these consumers remains
  a confirmed local failure requiring investigation.
- All 17 finalized recordings decoded without FFmpeg errors after the roughly
  eight-minute run, including segments created after source recovery. The
  launcher took about 33 seconds to join the server before stopping other
  services; prompt graceful shutdown is not qualified by this run.
- `cargo deny check` **failed** its advisory policy with seven vulnerability
  findings (four rustls, three zip); bans, licenses and sources passed. This
  reports dependency-policy findings, not demonstrated application exploits.
  `make audit` and the Phase 8 security fixture also stopped because `cargo-audit`
  is unavailable; the complete `make check` gate is not green.

Local raw logs and isolated runtime state are in
`target/verification-2026-10-20/` (ignored build output, not a committed artifact).
The historical sustained recorder-backpressure incident remains unresolved;
this smoke test does not close it. Physical webcam, Firefox, representative
accuracy, target storage and 10/20-camera qualification remain unverified.

## Repository map

```text
apps/server       current modular-monolith executable and HTTP surface
apps/worker       empty stub; no separately deployable worker exists
crates/auth       principals, roles, scope, and bearer-token verification
crates/camera     camera lifecycle and one-owner ingestion
crates/config     layered settings and secret handling
crates/database   PostgreSQL repositories and migrations
crates/detector   sampled detection, ONNX person/face lanes, presence, test backend
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

- investigation and resolution of the saved sustained-run `RecorderBackpressure`
  failure and preceding LiveKit closed-engine warnings;
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
