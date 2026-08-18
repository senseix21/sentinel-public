# Runbook: Camera Offline, Flapping, or Stalled

Failure-matrix cases 1, 2, and 7. Alerts: `CameraDisconnected`,
`CameraFlapping`, `RecordingStalled`, `RecorderQueueSaturated`,
`AuthenticationFailureSpike`.

**One camera's failure does not affect the others.** That is proven by Phase 3's
isolation tests. If several cameras fail together, the shared thing — network,
switch, power, the server itself — is the fault, not the cameras.

## Which failure is this

The three alerts describe different faults and want different responses.

| Symptom | Alert | Meaning |
|---|---|---|
| Down and staying down | `CameraDisconnected` | Reachability or credentials |
| Reconnecting repeatedly | `CameraFlapping` | Marginal link, or the camera is restarting |
| Connected but no segments | `RecordingStalled` | Ingest is fine, the recorder is not |

`RecordingStalled` is the dangerous one: nothing looks broken, and no footage is
being kept.

## Triage

```bash
curl -s localhost:3000/api/cameras/<id>/health -H 'Authorization: Bearer <token>'
curl -s localhost:3000/metrics | grep 'camera_id="<id>"'
```

The health snapshot gives connection state, last-connected and last-media
timestamps, reconnect count, and the last actionable failure — credential-free
by construction. Read `last_failure` before doing anything: it usually names the
fault outright.

| `last_failure` | Meaning | Go to |
|---|---|---|
| Authentication rejected | Credentials wrong or rotated at the camera | §Credentials |
| Dimension mismatch | Camera resolution changed | §Camera-side change |
| Unsupported media | Codec changed; only H.264 is qualified | §Camera-side change |
| Connection refused / timeout | Reachability | §Reachability |
| Recorder backpressure | Disk or recorder | `disk-full.md` |

## Reachability

```bash
ping -c 3 <camera address>
ffprobe -rtsp_transport tcp rtsp://<camera address>/<path>   # no credentials on the command line
```

`ffprobe` succeeding while the server cannot connect points at the server —
credentials, or a session limit on the camera that a previous connection is
still holding. Many cameras allow only one or two concurrent RTSP sessions;
check nothing else is connected before concluding the camera is broken.

Reconnect backoff is bounded, exponential, jittered, and resets only after a
30-second stable session. A camera that has been down for a while will not
reconnect instantly after the network returns — that is designed behavior, not a
hang.

## Credentials

Per-camera credentials live in the secrets file (P8.5), keyed
`camera.<id>.rtsp_password`, or in the matching
`SURVEILLANCE_SECRET_CAMERA_<ID>_RTSP_PASSWORD` environment variable.

```bash
ls -l <secrets file>        # must be -rw------- (0600); a wider mode is refused at startup
```

To rotate: update the camera, then update the file, then restart the camera
pipeline. Existing sessions keep the old credential until their next reconnect —
that is the documented application boundary, not a bug.

```bash
curl -sX POST localhost:3000/api/cameras/<id>/stop  -H 'Authorization: Bearer <token>'
curl -sX POST localhost:3000/api/cameras/<id>/start -H 'Authorization: Bearer <token>'
```

A missing secret is a named startup failure and a metric — never a silent
fallback to an unauthenticated connect. If the camera is failing to start and
the log names a missing secret, the secret is genuinely absent.

## Camera-side change

Dimension mismatch and unsupported media both mean someone changed the camera.
The server refuses rather than silently accepting, because accepting would mean
recording something other than what was configured.

Either restore the camera's settings, or update the configured expectation:

```bash
curl -sX PUT localhost:3000/api/settings \
  -H 'Authorization: Bearer <token>' -H 'Content-Type: application/json' \
  -d '{"changes":{"camera.<id>.width":"2560","camera.<id>.height":"1440"}}'
```

Only H.264 is qualified. H.265 is not supported and must not be worked around.

## Flapping

Flapping is worse than being down: fragmentary segments, and neither the
disconnect alert nor a manual check catches it.

```bash
curl -s localhost:3000/metrics | grep surveillance_camera_reconnects_total
```

Usually a marginal link (PoE power budget, cable, saturated switch port) or a
camera rebooting on a schedule. If the cause cannot be found quickly, **stop the
camera**: a stopped camera is an honest absence, while a flapping one produces
footage nobody can rely on.

## Authentication failure spike

`AuthenticationFailureSpike` is about the **API**, not the cameras. It fires on
sustained `bad_secret` or `unknown_token` — someone guessing tokens.

```bash
curl -s localhost:3000/metrics | grep surveillance_auth_failure_total
```

There is deliberately no rate limiting (see the threat model's accepted risks);
this alert is what makes that acceptable. Respond by revoking the targeted token
and, if the source is external, blocking it at the proxy.

Distinguish it from `reason="expired"`, which is an operational problem — a
service whose token lapsed — and is excluded from the alert expression for that
reason.

## Verify recovery

```bash
curl -s localhost:3000/api/cameras/<id>/health -H 'Authorization: Bearer <token>'
```

Recovered means: state `connected`, `last_media_at` advancing, reconnect count
stable, and a new completed segment within one segment duration. Connection
alone is not recovery — `RecordingStalled` exists precisely because a connected
camera can still be recording nothing.

---

**Execution status**: exercised against synthetic sources
(`scripts/phase3/multi-camera-fixture.sh`) on 2026-08-15 by the author, not by a
separate operator following it literally, and not against representative
cameras. See `docs/phase-8/local-evidence-2026-08-15.md`.
