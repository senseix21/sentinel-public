# Architecture — Phase 6 local implementation

One camera-owned RTSP session publishes immutable complete H.264 access units to
three independent bounded queues. Recorder preserves backlog, LiveKit favors
latency, and detection favors freshness. Every access unit is stamped with its
camera connection epoch before distribution.

Each enabled camera owns one `DetectionOwner`, dedicated FFmpeg decoder worker
and child, timestamp sampler, sequential fake inference adapter, event
publication, cancellation, and join. One process-owned bounded `EventBus` serves
camera, recording, storage, detector, and person facts. Media cannot enter its
closed payload enum.
