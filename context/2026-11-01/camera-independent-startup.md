# Camera-independent startup — 2026-11-01

User authorized implementing camera-free startup and dashboard configuration.
ADR 019 records the desired-state and write-only credential decisions and amends
ADRs 006/009. The media topology and upstream owner are unchanged.

`make run` now runs `scripts/dev/local-stack.sh` without a source. It discovers
PostgreSQL's versioned binaries; root on Linux switches to the existing checkout
owner before starting services, creating a new user-owned state directory when
needed. It does not create accounts or rewrite ownership of existing footage.
`make webcam`, `make webcam-release` and `make fixture` select explicit sources.
No ONNX artifacts are downloaded and no capture/decoder/model starts for an empty
fleet. Local services are still development-only and loopback-bound.

The supervisor serializes durable camera registration/intent before applying
commands. The 1 MiB, versioned JSON catalog requires mode 0600 and an exclusive
process lock; writes atomically rename after fsync. Secrets are not returned in
API responses, settings or audit values. A failed write rejects mutation and
requires restart before another catalog write. A failure after rename can leave
committed intent with an error response; inspect state after restarting.
Default launcher path: `.local/server-run/cameras.json`; standalone binary:
`.local/cameras.json`; override `SURVEILLANCE_CAMERA_CATALOG_PATH`. Back up as a
secret. Corrupt catalogs fail startup. Invalid individual camera configurations
are preserved and logged without values; operator repair is currently by editing
the protected catalog while stopped. The dashboard offers add/start/stop/remove,
not an endpoint/credential edit form; remove/re-add reconfigures a valid camera.

`/ready` measures application dependencies, not camera count or connectivity.
Camera API/metrics continue showing per-camera degradation. Catalog write failure
sets `surveillance_camera_catalog_available` to 0 and degrades `/ready` until
restart; this does not stop existing recording pipelines. The authenticated,
scoped `/api/recording-cameras` catalog reads indexed archive identities, including
removed runtime cameras. It returns at most 1024 identities and refuses larger
results explicitly rather than silently truncating. Existing recording routes
already work without a runtime registry entry.

Validation:

- Workspace fmt and all-target/all-feature Clippy with warnings denied passed.
- All-feature Rust tests: 596 passed, 0 failed, 39 ignored.
- Dashboard tests: 49 passed. Python fixture tests: 24 passed.
- PostgreSQL/playback fixture: 13 passed, including archive/empty-scope queries.
- Release build passed with `CARGO_PROFILE_RELEASE_STRIP=none`; the previously
  recorded ordinary macOS SQLx LINKEDIT failure remains a toolchain limitation.
- Chromium against the full stack: zero-camera login; credentialed registration
  saved stopped; start of an unreachable loopback RTSP endpoint; `/ready` 200;
  stack restart restores running intent; stop/remove clears catalog; credentials
  absent from API reads, input clears after success; zero browser errors.
- Test sources use synthetic credentials and no physical camera. This does not
  qualify a client camera, network, codec or deployment.

Remote source path remains `/home/dev/camera_project`, owned by sentinel-test.
Earlier root-owned `.local` prevented creating a child directory; the launcher
now creates its own new state directory under the selected user. Existing demo
services and test footage remain separate from the normal zero-camera launcher.

Remote verification completed:

- Source changes synced to `/home/dev/camera_project`; no media/build caches
  transferred and existing recording/demo directories preserved.
- Actual root `make run` launched through the generic wrapper on isolated
  loopback ports, switched to sentinel-test and reached login with zero cameras.
- Chromium through SSH forwarding passed the same add/start/restart/stop/remove
  flow against Debian. `/ready` stayed 200 for the unreachable test camera;
  catalog availability gauge was 1. No FFmpeg processes existed after removal.
- Linux server unit suite: 140 passed, 0 failed. Its three catalog tests cover
  exclusivity, durable intent, secure/corrupt inputs and failed writes.
- Temporary `sentinel-camera-free-validation` unit stopped after testing;
  pre-existing `sentinel-demo` and `sentinel-demo-source` remained inactive.
- No physical RTSP camera was supplied or tested. The complete source tree is synchronized separately from runtime state.

## Follow-up: public-only VPS startup

The first remote qualification explicitly set `LIVEKIT_MEDIA_IP=127.0.0.1`,
which masked failure of the default auto-selection path. User's plain `make run`
on default ports selected the VPS public interface and failed LiveKit's private
address guard. A regression test reproduced that failure before the fix.
`livekit-config.sh` now falls back to loopback when automatic selection yields a
public or otherwise disallowed address. Explicit invalid/public overrides remain
rejected; automatically selected private addresses are preserved. No Rust or
media topology changed.

Follow-up verification: all five LiveKit tests passed on macOS and Debian,
including the real LiveKit listener check. The actual root command `make run`
was run on default ports with no environment overrides and reached SENTINEL
ONLINE. `/ready` returned 200; dashboard port 3001 redirected to login (303).
The test was stopped with Ctrl-C afterward to free the normal ports for the user.
