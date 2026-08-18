# Runbook: Database Outage and Recovery

Failure-matrix case 4. Alerts: `DatabaseUnavailable`, `ConfigurationStale`,
`IdentityStoreUnavailable`.

## Read this before doing anything

**Recording does not stop when the database does.** Media preservation never
depended on the index: segments are written to the filesystem, and the index is
reconstructed by reconciliation. This is a Phase 4 design property with tests
behind it.

So this is not a page about losing footage. It is a page about losing
*retrieval*, *retention*, and — since Phase 8 — *the API*.

| Subsystem | During the outage |
|---|---|
| Recording | Unaffected. Segments continue and finalize. |
| Retrieval and search | Unavailable. Queries need the index. |
| Retention | **Stopped.** Watch the disk. |
| Configuration | Served from the last-known-good cache; `/ready` reports `Stale`. |
| API authentication | **Every request is refused `503`.** |

The two that bite are retention and authentication.

## Triage

```bash
curl -s localhost:3000/ready          # distinguishes liveness from dependency readiness
curl -s localhost:3000/metrics | grep -E 'surveillance_database_|surveillance_configuration_|surveillance_auth_unavailable'
pg_isready -h <host> -p <port>
```

`/live` staying `200` while `/ready` degrades is correct and is the distinction
the two endpoints exist for. Do not restart the server because `/ready` is
unhappy — that would interrupt recording, which is the one thing still working.

## Recovery

1. **Bring PostgreSQL back.** It is local to the unit (ADR 008), so this is a
   local service, not a network dependency.

2. **Do nothing else.** Reconciliation is automatic: startup reconciliation plus
   the continuous pass rebuild the index from what is on disk. There is no
   manual import step and running one would risk duplicate rows.

3. **Watch reconciliation catch up.**

   ```bash
   curl -s localhost:3000/metrics | grep surveillance_reconciliation_
   ```

4. **Confirm retention resumed.** This is the one that can turn a database
   outage into a disk-full outage hours later.

   ```bash
   curl -s localhost:3000/metrics | grep -E 'surveillance_retention_|surveillance_storage_used_ratio'
   ```

   If the disk climbed during the outage, go to `disk-full.md` now rather than
   waiting for `StorageCritical`.

5. **Confirm configuration is fresh.** `surveillance_configuration_stale` back
   to `0` and `/ready` no longer reporting `Stale`.

6. **Confirm the API authenticates.**

   ```bash
   curl -s -o /dev/null -w '%{http_code}\n' localhost:3000/api/cameras \
     -H 'Authorization: Bearer <token>'
   ```

   `200`, not `503`. A `503` with `identity_store_unavailable` means the
   identity store is still unreachable; `401` means the store is fine and the
   token is not.

## If the database is lost entirely

The RPO is **one segment** and the RTO is **15 minutes** (ADR 009 Decision 5).
Losing the index does not breach the RPO, because the index is derived data.

- **Recordings**: recovered by reconciliation from the filesystem. Nothing is
  lost.
- **Settings**: restored from backup. Without a backup, the configuration falls
  back to versioned defaults and every deployment-specific value must be
  re-entered. Losing this is annoying, not destructive.
- **Identities and tokens**: restored from backup. Without a backup, every token
  must be reissued — and until at least one enabled principal exists, a
  routable listener will refuse to start (ADR 009). That refusal is deliberate:
  binding without an identity store would grant administrator to anyone who can
  reach the socket.
- **Audit rows**: **unrecoverable** if not backed up. They are not derived from
  anything. This is the strongest argument for the nightly dump actually
  running.

Restoring identities is the step that gets forgotten, because recording comes
back on its own and everything looks fine until someone tries to use the API.

## Backup

Nightly dump plus WAL archiving, per the RPO/RTO above.

**Blocked**: the concrete backup command, schedule mechanism, and restore
procedure depend on the deployment target and on where PostgreSQL's data
directory lives — both unconfirmed (ADR 009). Writing them against a guessed
target would produce a procedure that fails when followed, which is worse than
its absence. Roadmap item 9's backup/restore runbook stays `[!]`.

---

**Execution status**: the outage and recovery path was exercised by stopping and
restarting a local PostgreSQL against `scripts/phase4/postgres-fixture.sh` on
2026-08-15 by the author. The **restore-from-backup** path was **not** exercised,
because the backup procedure is blocked above. See
`docs/phase-8/local-evidence-2026-08-15.md`.
