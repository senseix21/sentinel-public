# ADR 012 — Viewer-gated transcode publication

Status: Accepted (2026-08-17)
Supersedes: ADR 011 Decision 1 as the *default* publication path. ADR 011 is
otherwise still in force, and its Decision 5 is the evidence this one rests on.

## Context

ADR 011 chose zero-transcode encoded pass-through: hand the camera's own H.264
access units to LiveKit untouched. That is the correct design and it is
implemented in `crates/livekit/src/native.rs`.

It does not run. ADR 011 Decision 5 records the mechanism and the measurement:
`webrtc-sys` 0.3.42 builds the *first* encoder from the negotiated SDP format,
which carries no backend tag, so `IsAutomaticFallbackBackend` — which excludes
only `PreEncoded` — routes creation to a real H.264 encoder instead of the
pass-through one. On Apple that is VideoToolbox, which dereferences the encoded
payload as a pixel buffer: exactly 9 access units, then `SIGSEGV`, measured four
times.

The correction dated 2026-08-17 removes the escape route we thought we had.
OpenH264 is registered under `WEBRTC_USE_H264`, which is not platform
conditional and *is* present in the prebuilt this repository links. A real
encoder beats pass-through on every platform LiveKit ships. Only the crash
signature is Apple-specific.

So live video does not work anywhere, and the fix is not a different machine.

## Options

**A. Vendor and patch `webrtc-sys`.** Make the initial encoder honour the
requested backend. Restores true zero-transcode. Cost: a forked C++ build with
its own toolchain, a prebuilt binary per target platform, and an upgrade tax on
every libwebrtc bump — permanently, for a team that does not otherwise build
C++.

**B. Relocate to Linux.** Withdrawn. The premise was false; see above.

**C. Wait for upstream.** `livekit` 0.8.3 is the latest release. Live video stays
broken for an unbounded interval, and nothing about the roadmap can proceed past
P5N.6.

**D. Decode and re-encode, gated on viewer presence.** Feed I420 frames to an
ordinary `NativeVideoSource` and let libwebrtc encode. libwebrtc's *encoders*
work correctly; only pass-through is broken. Uses released APIs on every
platform and adds no dependency surface.

## Decision

**Option D, with option A held open behind a measurement.**

The economics decide it, not the aesthetics. Option A is architecturally purer
and it is the more expensive mistake:

- Live viewing is priority 3 of 5 in the ordering this repository already
  committed to. Recording and camera connectivity outrank it, and both are
  unaffected — proved by the run in `docs/phase-5/local-evidence-2026-08-16.md`,
  where 8 segments completed while publication went terminal.
- Transcode cost scales with **concurrent or recently disconnected viewers**,
  bounded by the configured idle grace, not with total camera count. The
  scaling risk this project actually carries is 10 → 20 → 200 cameras. A cost
  that is zero when nobody is watching does not sit on that axis.
- H.265 cameras cannot be passed through to a browser at all — WebRTC has no
  H.265 path in general browser deployment. Any H.265 camera needs this code
  regardless of what `webrtc-sys` does. Option D is therefore work that has to
  exist; option A is work *in addition*.

Forking libwebrtc to save CPU that nobody has yet measured as scarce inverts the
project's own rule that optimisation follows evidence.

### Decision 1 — the transcode path is a `PublicationAdapter`, not a new pipeline

`TranscodePublicationAdapter` implements the existing trait. The owner, health
model, failure categories, backoff, metrics, and lifecycle in
`crates/livekit/src/owner.rs` are untouched. Swapping publication strategy is a
constructor change in `apps/server/src/pipeline.rs`.

This is the payoff of having defined the adapter contract before the
implementation. `native.rs` is kept, not deleted: it is the path we return to if
option A is ever taken, and it still compiles and is still tested.

### Decision 2 — decoding is gated on viewer presence, and that is the load-bearing property

No viewer beyond the configured idle grace means no FFmpeg process, no decode,
no encode, and no published track. The adapter stays connected to the room so
it can observe arrivals, which is idle signalling only.

Viewer presence is derived from remote participant count, maintained from the
room event stream that `native.rs` previously discarded. **This is a deliberate
approximation**: a participant who joins the room and never subscribes still
costs one decode. The rooms are per camera (`<ns>-<camera>-live`), so a
participant in one is there to watch that camera, and the approximation is tight
in practice. If it ever stops being tight, the refinement is to track
`TrackSubscribed`/`TrackUnsubscribed` instead; the seam is one type.

The track is published on the first viewer and unpublished after the last
viewer has been absent for `camera.livekit.viewer_idle_grace` (five seconds by
default; zero restores immediate teardown). During that bounded interval the
same decoder and track continue to consume the existing live lane. This avoids
another process startup and source-keyframe wait during ordinary navigation; it
adds no queue, decoder, room, or upstream camera connection. A returning viewer
cancels teardown. Once the grace expires, an idle camera shows *no track* rather
than a frozen or black one.

Amended 2026-08-19 after the authenticated Chrome fixture measured cold and
warm reconnects. Evidence and limitations are in
`docs/phase-9/livekit-low-latency-2026-08-19.md`.

### Recovery amendment — 2026-10-23

A camera connection epoch change, backward timestamp/timebase change, or pacing
resynchronisation starts a new decodable media run at a parameterised keyframe.
The publication owner allocates a monotonic run identity, clears cached parameter
sets and RTP mapping, and re-establishes pacing. `EncodedAccessUnit::epoch`
identifies that run, rather than only a LiveKit room connection. The transcoding
adapter shuts down the prior decoder before feeding the new run; the room and
published track remain owned by the same session and viewer gating still applies.

Previously pacing reset alone left RTP mapping anchored to old timestamps and
rejected every later frame. The owner now reports Degraded during
resynchronisation/adaptation rejection and clears it only after adapter acceptance.
This changes no upstream connection, queue bounds, codec policy or clock authority.
Reversal must preserve coherent decoder, RTP and pacing resets; reverting only
one boundary reintroduces the demonstrated stall. Unit regressions cover backward
and forward camera epochs and same-epoch timestamp reversal. The local recovery
fixture is `scripts/dev/check-rtsp-recovery.cjs`.

### Decision 3 — the timestamp authority is the adapter clock, and no latency claim follows

Raw-frame capture gives libwebrtc a *capture* time for pacing and rate control.
The camera's media timestamp is not that, and mapping decoded frames back to
their source access units is not reliable in the presence of B-frame reordering.
Rather than invent a mapping, this adapter reports
`TimestampAuthority::AdapterClock`, which the trait already defines as "derives
timestamps internally and cannot support latency qualification."

The consequence is stated rather than hidden: **glass-to-glass latency cannot be
qualified through this path.** P5N.8 (time to first picture) is still
measurable; a p50/p90/p99 media-latency claim is not. Anyone who needs that
number needs option A.

### Decision 4 — output size is forced, which also makes resolution change survivable

FFmpeg is told the exact output frame size, so the reader consumes fixed-length
frames and cannot desynchronise. A camera that changes resolution mid-stream is
then scaled into the published size instead of corrupting the frame boundary.
The track keeps the size it was published with for the life of the session.

The size comes from the stream's own sequence parameter set, capped by an
optional configured maximum height. That cap is the single largest CPU lever
here — publishing 4K to a browser tile is waste — and it defaults to *no cap*,
because choosing a viewing resolution is a product decision this ADR does not
have the authority to make.

### Decision 5 — overload is resolved at decodable-run boundaries

The queues are the FFmpeg stdin pipe and libwebrtc's own encoder queue.

Writes to FFmpeg are bounded by a timeout. On expiry the adapter stops feeding
and waits for the next access unit that starts a decodable run before resuming.
Dropping arbitrary delta frames would corrupt the picture until the next IDR
anyway; aligning the drop to the run boundary makes the recovery deliberate and
countable instead of incidental.

Both the dropped access units and the resynchronisations are counted and
surfaced. Live viewing preferring freshness over completeness is the policy
`AGENTS.md` already sets for this consumer.

Amended 2026-08-17, from measurement: a *single* write timeout conflates two
different conditions. A cold FFmpeg has not reached its input loop, so nothing
drains the stdin pipe, and the first access unit of a stream — the largest one —
does not fit in the kernel pipe buffer. Measured on the local stack: every
subscribe produced exactly one `Overloaded` resynchronisation at startup, then
ran 45 s with none. The viewer paid a keyframe interval of black each time for a
stall that was never backpressure.

The first write of a session therefore gets `SPAWN_WRITE_TIMEOUT` (2 s) and every
write after it gets `WRITE_TIMEOUT` (250 ms). One completed write is the proof
that the decoder is reading. Raising the single budget instead would have blinded
the steady-state overload detector for the whole session, which is the one thing
it exists to catch.

### Decision 6 — the detector's decoder is not reused

`crates/detector/src/decoder.rs` is request/response, one frame at a time, under
a 50 ms timeout, in RGB24, built for *sampled* detection. This path needs
streaming decode at full frame rate in I420. Serving both from one component
would make both worse. They are separate owners of separate concerns, and each
attaches to the media distributor without adding an upstream camera connection —
the rule that actually matters is unbroken.

## Consequences

- Live video works on every platform, using only released APIs.
- CPU is spent only while someone is watching, and the amount is bounded by the
  configured maximum height.
- Zero-transcode publication is lost while this path is the default. `native.rs`
  remains as the return path.
- No media-latency claim can be made through this adapter. This is declared in
  the type system, not in a comment.

## Evidence, 2026-08-17

Local stack (`scripts/dev/local-stack.sh`): MediaMTX, a 1280×720@15 H.264 webcam
bridge, `livekit-server`, PostgreSQL, the trust-boundary proxy, and the
dashboard. Read from `/metrics` on `mac-webcam`.

| | no subscriber | one subscriber, 45 s |
|---|---|---|
| `surveillance_livekit_state` | 3 (connected) | 3 (connected) |
| `surveillance_livekit_viewers` | 0 | 1 |
| `surveillance_livekit_published_frames_total` | 0 | 808 |
| `surveillance_livekit_adapter_drops_total` | 250 | 455 |
| `surveillance_livekit_adapter_restarts_total` | 0 | 1 → 0 after the fix |
| `surveillance_livekit_send_failures_total` | 0 | 0 |
| `surveillance_livekit_reconnects_total` | 0 | 0 |

What this does and does not establish:

- Measured: the room connects once and stays connected; nothing decodes while
  `viewers` is 0; frames are produced once a subscriber arrives — **but not at the
  source rate, see the correction below**; access units are published without a
  send failure or a reconnect;
  publication survives the SIGSEGV that killed the pass-through path at
  access unit 9 (ADR 011 Decision 5) — it is past that by two orders of
  magnitude.
- **Not** measured: CPU per concurrent viewer, behaviour above one subscriber,
  behaviour above one camera, and any latency figure. Reversal criterion 2 below
  still has no number behind it, and no capacity claim follows from this table.

### Correction, 2026-08-17 — the source was 15 fps, not 9

The evidence above was read as "published at the source rate" because
`surveillance_livekit_access_units_total` climbed at 9.05/s and the webcam was
assumed to be delivering about 9 FPS. That assumption was checked against the
recorder, which consumes the *same* stream from the *same* distributor, and it is
false.

`ffprobe -count_frames` over the eight consecutive segments spanning the live
watch: 450 frames and 30.000000 s in every one — 3 600 frames over 240.000 s,
**15.000 fps throughout**. The source was delivering 15 fps while this adapter
published 8.85/s, about 59% of it.

This does not change the decision. It changes what the evidence supports:

- The claim "frames are produced at the source rate" is withdrawn. What is
  measured is that publication is sustained and failure-free, not that it is
  complete.
- The ~6 frames/s are lost *upstream of the adapter*.
  `surveillance_livekit_adapter_drops_total` was flat across the subscriber
  window, which points at the live distributor's freshness-preferring eviction or
  the owner's stale-drop rather than at the FFmpeg write path — reasoning, not a
  measurement. Closing it needs a run that samples
  `surveillance_camera_live_evictions_total` alongside the publish rate.
- Dropping stale media to preserve latency is the policy `AGENTS.md` sets for this
  consumer, so a shortfall here may be correct behaviour rather than a defect. It
  is currently *unattributed*, which is the problem: nobody chose 59%.
- It makes reversal criterion 2 harder, not easier. CPU per concurrent viewer must
  be measured against a known published frame rate; measuring it at an
  unexplained 59% of source would understate the cost of the path.

Recording was unaffected: every segment across the live watch is a full 450-frame
30.000 s file, which is the priority ordering holding under load.

## Reversal criteria

Return to ADR 011 Decision 1 (encoded pass-through) when **either**:

1. An upstream `webrtc-sys` honours the requested backend when the *initial*
   encoder is created. Re-verify exactly as ADR 011 Decision 5 specifies: bring
   up the local stack and watch `surveillance_livekit_access_units_total` climb
   past 9 without the process dying. Past 9 is the whole test.
2. Measured transcode cost at the real deployment's concurrent-viewer count
   exceeds its CPU budget. That measurement — CPU per concurrent viewer at the
   camera's true resolution and frame rate — is the *precondition* for taking
   option A, not a justification to be assembled afterwards. Without the number,
   forking libwebrtc is not authorised by this ADR.

Abandon viewer gating (Decision 2) if a deployment appears that needs every
camera continuously live — a video wall. The gating is then pure overhead and
the correct shape is a persistent transcode with an explicit CPU budget. No such
requirement exists today.
