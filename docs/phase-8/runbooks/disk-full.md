# Runbook: Storage Pressure and Disk Full

Failure-matrix case 5. Alerts: `StorageCritical`, `StorageFull`,
`SegmentWriteFailures`.

**Priority**: recording preservation is priority 1 in `AGENTS.md`. Everything
below is in service of continuing to record.

## What the system already does

Read this first — most of the response is automatic, and intervening before you
understand that makes things worse.

- Retention runs from indexed metadata, not a directory scan, and deletes
  oldest-first per camera.
- Watermarks have hysteresis, so the system does not oscillate at the boundary.
- The three watermarks are configuration (`storage.watermark_*`), not constants.
  Phase 7 moved `StorageWatermarks::new(80, 90, 97, 2)` into the configuration
  domain.

If `StorageCritical` is firing, retention is running and is not keeping up. If
`StorageFull` is firing, writes are about to fail.

## Triage

```bash
curl -s localhost:3000/metrics | grep -E 'surveillance_storage_|surveillance_recorder_segments_failed'
df -h <recording root>
curl -s localhost:3000/ready
```

Answer these three, in order:

1. **Is it our data?** `df -h` against the recording root versus the sum of what
   the index knows about. A large gap means something else is filling the disk —
   logs, a core dump, another service — and deleting footage will not fix it.
2. **Is retention running?** `surveillance_retention_deleted_total` should be
   increasing. If it is flat while usage climbs, retention is stuck, which is
   usually a database problem — go to `database-outage.md` first.
3. **Is it one camera or all of them?** A single camera at many times its
   expected bitrate is a camera-side configuration change, not a storage
   problem.

## Actions, least destructive first

**Never delete recording files by hand.** The index and the filesystem would
disagree, and reconciliation would then have to be trusted to sort out a mess it
did not create. Use retention.

1. **Recover non-recording space.** Logs, cores, old release artefacts. Usually
   enough to clear the immediate page and buys time to think.

2. **Shorten retention.** The supported lever.

   ```bash
   curl -sX PUT localhost:3000/api/settings \
     -H 'Authorization: Bearer <token>' \
     -H 'Content-Type: application/json' \
     -d '{"changes":{"retention.max_age_days":"14"}}'
   ```

   Requires an administrator token and `MutatingSurface` enabled. Applies at the
   next retention pass, not instantly. **This destroys footage** and is audited
   with before and after values — which is the point: shortening retention is
   also what someone does to destroy evidence, so it leaves a record.

3. **Stop the highest-bitrate camera.** If one camera is responsible and the
   others matter more, stopping it preserves the rest. An operator decision, and
   an audited one.

   ```bash
   curl -sX POST localhost:3000/api/cameras/<id>/stop -H 'Authorization: Bearer <token>'
   ```

4. **Add storage.** The only non-destructive fix. Everything above buys time for
   it.

## Verify

```bash
curl -s localhost:3000/metrics | grep surveillance_storage_used_ratio   # falling
curl -s localhost:3000/metrics | grep surveillance_recorder_segments_completed_total  # rising
curl -s localhost:3000/ready
```

Recording is healthy when segments are completing again and no camera reports a
recorder failure.

## If writes already failed

Segment write failures leave `.partial.mp4` files. **Leave them.** They are
reconciliation evidence, they are decodable, and startup reconciliation
discovers them without promoting or deleting them. Deleting them destroys the
only record of what was being recorded when the disk filled.

## Afterwards

Compare actual bitrate against the planning figure in
`docs/phase-0/resource-sizing.md`. A disk that filled early usually means the
planning bitrate was wrong, not that retention was.

---

**Execution status**: exercised against a local fixture on 2026-08-15 by the
author, not by a separate operator following it literally. The handoff's
standard is the latter. See `docs/phase-8/local-evidence-2026-08-15.md`.
