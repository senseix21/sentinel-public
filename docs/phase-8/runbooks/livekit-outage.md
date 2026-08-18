# Runbook: LiveKit Outage

Failure-matrix case 3 (LiveKit half). Alert: `LiveKitPublicationFailing`.

## Severity

**Warning, not a page.** Live viewing is priority 3 in `AGENTS.md`, below
recording and camera connectivity. LiveKit is an independent consumer attached
to the media distributor: its failure affects live viewing and nothing else,
which Phase 5 tested.

If someone escalated this out of hours, the escalation is wrong. Recording is
unaffected and the footage will be there in the morning.

## Confirm the blast radius before anything else

```bash
curl -s localhost:3000/metrics | grep -E 'surveillance_livekit_|surveillance_recorder_segments_completed_total'
curl -s localhost:3000/api/cameras -H 'Authorization: Bearer <token>'
```

Segments still completing and cameras still `connected` means this is
live-viewing only. **If recording has also stopped, this is not a LiveKit
problem** — the LiveKit publisher cannot stop the recorder, so a shared cause is
at work. Go to `camera-offline.md`.

## Triage

1. **Is the LiveKit server up?** It is a separate service and usually a separate
   host. Check it before looking at this system at all.

2. **Are the credentials present?** The API key and secret live in the secrets
   file (P8.5) as `livekit.api_key` and `livekit.api_secret`, or the matching
   `SURVEILLANCE_SECRET_LIVEKIT_*` variables.

   ```bash
   ls -l <secrets file>    # -rw------- (0600); a wider mode is refused at startup
   ```

   A missing secret is a named failure and a metric, never a silent fallback.

3. **Is it one camera or all?** All cameras means the server or the credentials.
   One camera means that camera's publisher — and check whether that camera is
   healthy at all first.

4. **Is it a codec problem?** Only H.264 is qualified for publication, and
   multi-slice input exposed NAL-versus-access-unit pacing during Phase 0/5
   qualification. A camera that recently changed encoder settings is the likely
   cause. This is unresolved qualification work, not a deployment fault — see
   `ROADMAP.md` Phase 5, which carries several `[~]` and `[!]` items.

## Recovery

Restoring the LiveKit server is usually the whole fix; the publisher reconnects
without intervention.

To force one camera's publisher to restart:

```bash
curl -sX POST localhost:3000/api/cameras/<id>/stop  -H 'Authorization: Bearer <token>'
curl -sX POST localhost:3000/api/cameras/<id>/start -H 'Authorization: Bearer <token>'
```

**This interrupts recording for that camera** for the duration. Given live
viewing outranks nothing, do not do this to fix a live-view problem unless the
camera is not recording usefully anyway.

## Viewer tokens

Viewer tokens are subscribe-only and camera-scoped by room. Since P8.3, issuing
one is a protected, camera-scoped action: a principal can only mint a token for
a camera in its scope.

They remain labelled **development-only**. `ROADMAP.md` Phase 5 records that
full subscriber decode, representative High Profile cameras, browser playback,
and target concurrency are unqualified. Do not present live viewing as a
supported production capability on the strength of this runbook.

## Verify

```bash
curl -s localhost:3000/metrics | grep surveillance_livekit_publication_state
```

Recovered means `state="publishing"` is `1` for each expected camera, and the
alert clears within its 10-minute window.

## What not to do

- Do not restart the server to fix live viewing. That interrupts every camera's
  recording to repair the lowest-priority subsystem.
- Do not disable the recorder to "free capacity" for publication. The priority
  ordering runs the other way.
- Do not raise the LiveKit queue capacity to stop drops. The live queue drops
  stale media *by design* to preserve latency; a growing live queue would trade
  the property it exists to provide.

---

**Execution status**: **NOT EXERCISED.** No LiveKit server is available in this
environment, so no step in this runbook has been executed. It is written from
the Phase 5 implementation and its metrics, which is a weaker basis than
execution. See `docs/phase-8/local-evidence-2026-08-15.md`.
