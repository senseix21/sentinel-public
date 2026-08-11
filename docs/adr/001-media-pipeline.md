# ADR 001: Media Pipeline

Status: topology accepted; timestamp and LiveKit adapter contracts provisional for the H.264 vertical slice

## Context

Recording, live publication, and detection need the same camera stream but have different latency, loss, and representation requirements. The prototype opens multiple upstream connections and demonstrates the coupling this architecture must remove.

## Decision

- Exactly one ingestion owner maintains the active upstream session for a camera.
- The ingestion owner emits encoded packets into a bounded media-distribution boundary.
- Recorder, detector, and LiveKit publisher are independent media consumers.
- Recording consumes encoded media and avoids decode/re-encode when container compatibility permits.
- Detection receives sampled decoded frames through an explicit decode boundary.
- Domain events do not transport packets or frames.
- Slow consumers cannot produce unbounded memory growth; each consumer defines overload behavior and metrics.

## Representation requirements

An encoded packet contract must include camera and logical-stream identity, codec/caps, immutable payload ownership, PTS/DTS or equivalent timestamps with timebase, duration when known, and keyframe/random-access information. The distribution contract must also preserve the boundary between codec units and complete presentation units: an H.264 NAL unit or slice is not necessarily a complete video frame/access unit.

For the H.264 MVP, camera ingestion implements RFC 6184 non-interleaved depayloading before the media-distribution boundary. It accepts single-NAL, STAP-A, and FU-A payloads. It reconstructs aggregation/fragmentation, enforces configured allocation bounds, propagates RTP timestamp/marker semantics, and discards incomplete fragments on loss or discontinuity. Interleaved mode, STAP-B, MTAP, and FU-B are unsupported unless representative cameras require them.

The pipeline distinguishes source media time, connection epoch, monotonic observation time, recording UTC, and segment-relative media time. The normative provisional rules are in `docs/phase-0/timestamp-contract.md`.

Do not finalize the Rust type or trait until representative H.264/H.265 RTSP spikes establish:

- Timestamp source, normalization, reordering, discontinuity, and reconnect behavior.
- Packet/caps changes during a session.
- Payload ownership and copy cost across subscribers.
- Container and LiveKit compatibility.
- Cancellation and channel-closure behavior.

Container/crash evidence now selects fragmented MP4 and a partial-file lifecycle for the H.264 MVP. The following rules are settled for the first H.264 slice:

- Unknown timestamps are optional, never encoded as zero.
- Timestamp ticks always carry/reference a timebase.
- A reconnect or incompatible caps change begins a new connection epoch.
- Segment duration uses monotonic elapsed policy and safe media boundaries, not an assumption that source PTS begins at zero.
- Original source timestamps and explicit discontinuity remain observable after segment-relative normalization.
- Segment fragments and rotation align to safe random-access boundaries; active files remain explicitly partial until validated finalization.

The recorder domain now encodes this lifecycle as consuming
`OpenSegment -> FinalizingSegment -> CompletedSegment` transitions, with typed
failure exits. Completion requires evidence for the expected durable rename
target, a nonzero observed size, known dimensions, and an ordered wall-clock
interval. This is a contract, not an implemented muxer or filesystem durability
claim; see `docs/phase-0/recording-segment-lifecycle.md`.

A local LiveKit spike also established that a compatible H.264 Annex-B stream can be published and subscribed without decoding pixels or opening another camera connection. This settles the topology, not the production adapter:

- The media distributor may feed recording and live publication from the same encoded H.264 packet source.
- Bitstream framing conversion and RTP packetization are media adaptation, not permission to decode/re-encode.
- The production publisher must own bounded buffering, cancellation, reconnect, codec-configuration delivery, keyframe requests, and metrics.
- LiveKit pacing operates on complete timestamped access units. The adapter must assemble multi-slice frames rather than assigning frame duration to each NAL unit.
- Publisher readiness precedes attachment to media distribution; unavailable live publication drops according to its freshness policy and cannot backpressure recording.
- The `lk` CLI Unix-socket publisher is experiment infrastructure only. It is not the production Rust interface.
- Browser rendering, representative High Profile cameras, latency, and outage isolation remain acceptance gates.

## Consumer overload policy

- Recorder: preservation-oriented; queue exhaustion is an explicit critical degradation, not silent loss.
- Detector: freshness-oriented; sample/drop stale frames and report skipped work.
- LiveKit: latency-oriented; drop stale media or reconnect according to publisher contract.

Exact capacities remain configuration derived from measured item sizes and latency budgets.

The first SDK-neutral distributor implementation now makes the recorder and live
queues independent. Publication is nonblocking: recorder saturation preserves
its backlog and returns `Full`, while live saturation replaces only its oldest
item and returns `ReplacedOldest`. Capacity, depth, acceptance, consumption,
policy drops/rejections, and closure are explicit state. This proves overload
semantics but does not yet provide async wake-ups or a production byte budget;
see `docs/phase-0/media-distribution-contract.md`.

Async wake-up now uses a Tokio `watch` generation signal while the bounded
queues remain authoritative. The owned recorder task returns queue/sink
ownership on drain, cancellation, or sink failure. Dropping its owner aborts the
task rather than detaching it. Graceful drain requires upstream producer closure
followed by joining the recorder; see
`docs/phase-0/async-recorder-task-contract.md`.

## Alternatives

- Independent consumer RTSP sessions: rejected due to duplicated network/connection work and divergent failure behavior.
- Decode into a shared raw-frame pipeline: rejected as the primary representation because recording should preserve encoded media and decoded frames are expensive.
- External media broker for MVP: deferred until a measured cross-process requirement exists.

## Current evidence

Synthetic file-backed FFmpeg spikes on 2026-08-10 proved exact 30-second H.264 segmentation. H.265 segments decoded but exposed negative-PTS and boundary drift. A deterministic H.264 reconnect fixture proved that fresh publisher epochs reset source time and readers can join at a non-zero keyframe. Crash experiments selected fragmented MP4 over classic MP4 and retained Matroska as a fallback. LiveKit experiments published and subscribed to synthetic and physical-webcam H.264 tracks. Multi-slice input exposed incorrect NAL-level pacing in the CLI fixture; one-slice access units sustained 15 FPS, and the webcam run forwarded 10,464 packets with zero LiveKit drops. The camera crate now has deterministic single-NAL/STAP-A/FU-A depayloading tests through completed access-unit assembly. Media tests prove independent bounded recorder-preservation and live-freshness queues. See `docs/phase-0/media-spikes.md`, `docs/phase-0/timestamp-contract.md`, `docs/phase-0/container-finalization.md`, `docs/phase-0/livekit-publication.md`, `docs/phase-0/rtp-h264-contract.md`, and `docs/phase-0/media-distribution-contract.md`.

## Reversal criteria

Extract distribution or consumers across processes only when measured resource placement or fault-isolation needs exceed a single-process bounded pipeline.
