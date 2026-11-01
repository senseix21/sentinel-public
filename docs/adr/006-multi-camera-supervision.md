# ADR 006: In-Process Multi-Camera Supervision

Status: accepted for Phase 3 synthetic qualification; representative-camera qualification pending

Date: 2026-08-11

Amended 2026-11-01 by [ADR 019](019-camera-independent-startup.md): camera
registrations and desired running state are durable in a protected local catalog;
write-only dashboard RTSP credentials are permitted for camera onboarding.

## Context

Phase 2 has one structured camera-to-recorder lifetime, but `main` constructs it
from borrowed configuration and directly couples its exit to the health listener.
Phase 3 must own multiple independent pipelines without adding upstream RTSP
connections, retaining media in control state, or allowing one child failure to
cancel unrelated cameras.

Known facts are that the existing camera owner returns its distributor on exit,
the recorder drains only after that producer is dropped, and Retina session
ownership is contained inside the camera future. The provisional product priority
requires visible recording failure rather than continued ingest that silently
discards footage. Representative hardware and the ten-camera soak duration remain
external decisions.

## Decision

`CameraSupervisor` is one process-level task and the singular owner of a bounded
map keyed by validated `CameraId`. Each map entry retains validated configuration,
pipeline state, a credential-free health reader, an optional shutdown sender, and
an optional owned join handle or terminal outcome. No registry value contains
encoded media.

The supervisor accepts a bounded command protocol with a bounded one-shot reply:

- `Add` inserts a configured, stopped camera and rejects duplicate identity.
- `Start` changes `Configured`, `Stopped`, or `Degraded` to `Starting` and creates
  exactly one owned pipeline task. Starting an already active camera is idempotent.
- `Stop` requests cancellation and joins the pipeline before replying. Repeated
  stop of a non-running configured entry is idempotent.
- `Remove` first performs the same cancellation and join, then removes health and
  configuration ownership. Unknown identities are typed errors.
- `Shutdown` closes admission, requests every child to stop, joins every child,
  and returns only after the registry contains no running task.

Callers receive explicit `Full`, `Closed`, and response-timeout failures. Caller
cancellation drops only its reply receiver; the supervisor completes the accepted
command and safely discards an undeliverable reply. Commands are serialized by the
owner, while unexpected child completion is observed concurrently with command
receipt and retained as bounded terminal evidence.

Pipeline ownership state is separate from RTSP connection state:

```text
Configured -> Starting -> Running -> Stopping -> Stopped
                              `----> Degraded or Terminal
```

A recorder-first exit stops that camera's ingest and marks only that pipeline
degraded. A terminal camera configuration/media failure marks only that entry
terminal. Neither is automatically restarted until an explicit start; transient
RTSP reconnect remains inside the camera owner. Health/API listener exits are
sibling outcomes owned by `ServerRuntime` and never send camera shutdown.

Health is a bounded snapshot registry. Configured and stopped entries remain
visible; removed entries disappear after join. Metrics use only validated
`camera_id` and fixed consumer/state labels. URLs, paths, recording IDs, and error
text are never labels or response fields. Actionable failures use a bounded enum,
not retained arbitrary strings.

## Child-exit policy

| Exit | Owning entry | Other cameras | API/health |
|---|---|---|---|
| requested stop | stopped after join | unaffected | unaffected |
| terminal camera failure | terminal | unaffected | continues |
| recorder failure/backpressure | degraded/stopped | unaffected | continues |
| API/health listener failure | unchanged | unchanged | server records/restarts or exposes listener failure |
| process shutdown | cancelled and joined | cancelled and joined | joined independently |

## Acceptance test inventory

- `owned_pipeline_rolls_back_startup_without_detaching_children`
- `owned_pipeline_requested_stop_is_repeatable_and_joined`
- `owned_pipeline_contains_camera_first_exit`
- `owned_pipeline_contains_recorder_first_exit`
- `supervisor_controls_two_entries_independently`
- `supervisor_rejects_duplicate_and_unknown_operations`
- `supervisor_serializes_concurrent_stop_and_remove`
- `supervisor_shutdown_cancels_backoff_and_joins_every_child`
- `health_registry_reads_and_removes_concurrently`
- `api_listener_failure_does_not_cancel_pipelines`
- `two_camera_source_failure_preserves_other_recording`
- `ten_camera_synthetic_orchestration_is_bounded_and_joined`

## Alternatives

- Let HTTP handlers own a concurrent map and spawn tasks: rejected because task
  ownership, removal ordering, and child exits would have multiple informal owners.
- One supervisor task per operation or detached pipeline tasks: rejected because
  accepted commands could outlive their ownership and shutdown could not prove join.
- External process/broker orchestration: deferred until measurements justify the
  changed deployment and delivery semantics.

## Reversal criteria

Shard or extract the supervisor only when measured command latency, task count,
resource placement, or process-failure isolation cannot meet the accepted 10/20
camera workload. Revisit explicit-start semantics if persistent configuration
requires desired-state reconciliation. Revisit recorder restart only after storage
failure categories and a bounded retry/reconciliation policy are specified.

