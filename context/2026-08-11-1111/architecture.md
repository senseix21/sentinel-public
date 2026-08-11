# Architecture Context — 2026-08-11 11:11 +06

`run_h264_camera` is the singular per-camera runtime owner. Its inputs are a
validated camera identity/source, an owned bounded media distributor, validated
reconnect policy, injected jitter source, and shutdown watch. It returns the
distributor, final credential-free lifecycle snapshot, stop reason, and bounded
operational counters; it does not detach a task.

The recorder and live queues retain distinct overload policies. A full recorder
queue preserves its backlog and stops ingestion visibly because silent recording
loss is unacceptable. A full live queue replaces its oldest item and increments
an eviction counter to preserve freshness without disrupting recording.

Connected lifecycle transitions can return the owned session to the supervisor.
This permits bounded asynchronous RTSP teardown before reconnect or shutdown
while preserving typestate counters and making connected-without-session
unrepresentable.

