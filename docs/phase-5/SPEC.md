# Phase 5 Rework — Native Encoded LiveKit Publication

Status: in progress. Opened 2026-08-16.

Governing decision: `docs/adr/011-native-encoded-livekit-publication.md`, which
supersedes ADR 007 for H.264 publication.

## Why this exists

Phase 5 delivered LiveKit publication as an independently degradable consumer, and
ROADMAP recorded subscriber decode as an unqualified external gate. That gate was
failing. The `lk` CLI socket boundary accepts approximately 2.8 access units per
second irrespective of bitrate or flags, so a 30 fps camera lost 90.7% of its
frames — keyframes included, because the bounded live queue evicts by age — and the
browser rendered an all-zero YUV buffer. ADR 011 records the measurements.

This rework replaces the publication adapter with the released LiveKit Rust SDK's
encoded passthrough path. It does not redesign the pipeline around it.

## Scope

In scope: the implementation behind `PublicationAdapter`; the live queue's
eviction policy; publication health and metrics sufficient to detect this class of
failure; the encoder and queue settings that were tuned against a starved path.

Out of scope: decoding, transcoding, or re-packetising media; H.265; simulcast;
recording; the ingestion path; the dashboard's client, which ADR 010 governs.

## Definitions

**Seamless**, used as an acceptance term rather than a description, means the
subscriber receives the camera's frame rate continuously, with a decodable
reference chain, and recovers after interruption. It is decomposed into P5N.6
through P5N.9 and is not claimed except from measurement.

**Access unit** keeps its existing meaning from `H264AccessUnit`: one complete
picture, all slice NALs grouped, with parameter sets attached to keyframes.

## Contracts

### C1 — The adapter contract is unchanged

`PublicationAdapter` keeps its existing shape. `PublicationOwner` retains
ownership of lifecycle, health, counters, retry, backoff, shutdown and the media
subscription. A conforming adapter accepts a complete access unit with its media
timestamp and reports a typed failure category; it does not own reconnect policy.

This is the boundary that makes ADR 011 reversible. An implementation that needs
the owner to change is out of contract and requires the ADR to be revisited.

#### C1a — Amended 2026-08-16: offering is not publishing

C1 as originally written was wrong in one respect, and the amendment is recorded
here rather than applied silently.

`NativeVideoSource::capture_encoded_frame` returns `bool`. A `false` means the
transport was momentarily not ready — it is a dropped frame, not a failed
session. The original signature, `send_access_unit(..) -> Result<(), _>`, could
only express that refusal as an error, and the owner treats an error as a
retryable session failure. A momentary refusal would therefore have torn down and
rebuilt the room, which is a worse outcome than the dropped frame it was
reacting to.

The trait method now returns `SendOutcome::{Published, Dropped}`:

- `Ok(Published)` — the bytes reached the transport. Counted in `access_units`,
  `bytes` and, for a run start, `keyframes`.
- `Ok(Dropped)` — the adapter accepted the access unit and did not put it on the
  wire. Counted in `adapter_drops`. The session continues.
- `Err(_)` — the session failed. Unchanged: counted in `send_failures` and
  classified for retry by the owner.

The owner's ownership of lifecycle, health, retry, backoff, shutdown and the
media subscription is unchanged, so the reversibility argument above still holds:
the CLI adapter satisfies the new signature by returning `Published` after a
completed socket write. The amendment also supplies P5N.10's offered-versus-
published visibility, which the original contract had no way to express.

### C2 — Media is passed through, never re-encoded

The adapter hands the access unit's bytes to the SDK with an explicit capture
timestamp and an explicit key/delta classification. This project performs no
decode, no re-encode, and no RTP packetisation. One upstream connection per camera
is unaffected: LiveKit remains an attached consumer of the media distributor.

### C3 — Timestamps come from media, not from arrival

Publication timestamps derive from the access unit's media timestamp under the
existing `TimestampAuthority`, so that publication timing is not a function of how
busy the process is.

## Requirements

Each requirement names the evidence that closes it. A requirement that cannot be
measured locally is marked as such rather than assumed.

### P5N.1 — The native dependency builds on this machine

`livekit` 0.8 builds in this workspace, and `cargo deny check` and `cargo audit`
still pass with it present. Build time cost is recorded.

If this fails, ADR 011 does not take effect and the fallback in its option 2
applies. This requirement is deliberately first.

### P5N.2 — The native adapter implements `PublicationAdapter`

Connect, publish, disconnect and shutdown map onto SDK operations with typed
errors. Cancellation is honoured. No `unwrap`, `expect`, or `panic!` on a
production path. No detached task: any task the SDK requires this project to own
has an owner, a shutdown signal, and a join policy.

Evidence: the crate's unit tests, plus the existing owner tests passing unchanged
against the new adapter.

### P5N.3 — The child-process boundary is removed, not left in parallel

The Unix socket, its write timeout, the `lk` child, and their configuration are
removed once the native adapter is qualified. Dead or parallel paths are not
retained. Configuration keys that no longer mean anything are removed rather than
silently ignored.

### P5N.4 — Failure isolation is demonstrated, not inherited

The child process previously supplied isolation between publication and recording
by construction. In-process publication must demonstrate it: a LiveKit outage,
a publication error, and a publisher restart each leave recording admission
uninterrupted.

Evidence: the existing Phase 5 isolation tests, re-run against the native adapter,
plus a fixture run in which the LiveKit server is stopped and restarted while
recording continues.

### P5N.5 — Keyframes survive live-queue eviction

Age-ordered eviction must not discard the reference a subscriber needs. The live
queue preserves the most recent keyframe, so that whatever survives overload is
decodable.

This requirement stands on its own merit and is not conditional on the native
adapter. Evidence: a deterministic queue test that overflows the queue and asserts
a keyframe is retained.

### P5N.6 — Published rate equals source rate

Under nominal single-camera load, published access units per second equal the
camera's frame rate, and live-queue eviction is approximately zero.

Evidence: `/metrics` sampled across two intervals, reporting accepted, received
and dropped counts, alongside the source rate derived independently from a
recorded segment's frame count and duration.

### P5N.7 — The subscriber decodes continuously

A browser subscriber renders moving video with no all-zero frame. This is the gate
that was previously unqualified and silently failing.

Evidence: a browser session against the dashboard reporting `readyState`,
`videoWidth`/`videoHeight`, and an advancing `currentTime`, together with a
sampled pixel check that rejects the all-zero YUV signature. A screenshot alone is
not evidence; the pixel check is what distinguishes this from the failure it
replaces.

### P5N.8 — Time to first picture is bounded and stated

Time from subscribe to first rendered frame is measured and reported. Per ADR 011
Decision 4 it is bounded by the encoder's keyframe interval, because this project
cannot force an IDR on receiver feedback. The measured value is recorded together
with the keyframe interval that produced it.

### P5N.9 — Recovery is proven

Publication recovers from a LiveKit server restart and from a camera reconnect,
returning to P5N.6 and P5N.7 without operator action, with the transition visible
in health and metrics.

### P5N.10 — This failure class is detectable from metrics alone

A publisher accepting a small fraction of offered frames must be visible without a
browser. Health must not report an unqualified healthy publication state while the
live queue is discarding most of what it is offered.

Evidence: a metric or health field expressing the offered-versus-published
relationship, and an alert rule in `docs/phase-8/alerts.yml` whose metric name is
verified by `scripts/phase8/check-metric-names.py`, which exists precisely because
invented metric names make alerts silently inert.

### P5N.11 — Settings tuned against a starved path are re-examined

The live queue capacity of 4 and the media-time pacing in `PublicationOwner` have
never run under real throughput. The pacing currently never sleeps, because a
starved queue always yields units whose media time lands behind wall-clock. Both
are re-derived from measurement once frames flow, and the resulting values are
configuration with a documented meaning, not constants.

The local encoder settings are also corrected: the webcam publisher emits 3.91 Mbps
unconstrained `ultrafast`, and its configured 15 fps does not take effect on this
device, which delivers 30.

## Non-goals

No capacity claim is made from this work. Frame rate to a single subscriber is not
a camera-count or viewer-count result; those belong to Phase 9 and require a
documented workload.

No accuracy, latency-target, or concurrency claim is introduced here. H.265,
simulcast, and adaptive bitrate remain out of scope and unclaimed.

## Known blockers

`RecorderBackpressure` stops the camera within minutes when the recording root is
large, because startup reconciliation starves the recorder. It is unrelated to
publication but will interfere with any sustained measurement, and P5N.6 through
P5N.9 need sustained runs. Validation uses a clean recording root; the defect
remains open and is not closed by this work.
