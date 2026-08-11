# Repository Status — 2026-08-11 20:54 +06

## Verified implementation

- `CameraSupervisor` owns a bounded validated-ID map and bounded add/start/stop/
  remove/shutdown protocol with request/reply timeouts and explicit full/closed
  outcomes.
- Pipeline start/stop operations and unexpected completion are supervisor-owned;
  process shutdown cancels and joins all media children.
- Per-camera snapshots expose pipeline/connection state, attempts/epochs/
  reconnects, last connected/media timestamps, bounded actionable failures,
  recorder counts, and queue bounds/outcomes without credentials or media.
- Loopback camera lifecycle/API resources translate onto supervisor commands and
  do not expose source URLs, credentials, storage paths, or raw internal errors.
- Stable-session backoff reset is explicitly configured at 30 seconds.

## Qualification evidence

- `scripts/phase3/multi-camera-fixture.sh` passed with ten 640x360/15 FPS H.264
  RTSP sources. All ten produced fully decodable segments. Camera A outage and
  recovery produced a fresh epoch while camera B completed another segment with
  zero reconnects. Connected and reconnecting removal, invalid credentials, and
  dimension mismatch remained camera-local. Exactly 12 read sessions were
  observed: ten initial, one A reconnect, and one dimension-validation session.
- The final successful debug-process sample was 7.5% CPU and 27,668 KiB RSS on the
  development Mac. Recorder queues had zero rejection. Shutdown finalized viable
  segments; each too-short retained partial was decodable and matched by an
  explicit shutdown-failure fact (the final run produced none). Open file
  descriptors decreased from 33 before outage to 28 after reconnect/removals.
  These are point samples, not percentile/capacity results.
- `scripts/phase0/retina-rtsp-fixture.sh` passed after the ownership migration:
  five camera integration cases, the three-segment recorder case, composed
  server, SIGINT/SIGKILL, full decode, and reconciliation discovery.
- Final gate passed after all changes: `cargo fmt --all --check`, workspace/all
  target/all-feature Clippy with warnings denied, 79 ordinary tests plus all doc
  tests, `cargo build --workspace --release`, and `git diff --check`.

## Scope boundary

Representative-camera and target-hardware soak evidence is blocked externally
and is explicitly not claimed complete.
