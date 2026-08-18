# ADR 011: Native Encoded LiveKit Publication

Status: accepted; supersedes ADR 007 for H.264 publication. Native adapter
qualification is pending the evidence named in `docs/phase-5/SPEC.md`.

Date: 2026-08-16

## Context

ADR 007 chose a child-process boundary: the server writes Annex-B access units
to a Unix socket and the `lk` CLI (`--publish h264:///<socket>`) performs the
WebRTC publication. That choice was correct on its evidence. It was taken because
no released official LiveKit Rust API accepted pre-encoded access units, and it
recorded the conditions under which it should be revisited.

Both of those conditions have now fired. This ADR records what was measured and
what replaces the CLI.

### What was measured

Verified on 2026-08-16 against the running local stack (`make run`, Mac webcam via
MediaMTX, `lk` 2.18.2, livekit-server on loopback).

The operator dashboard showed a live tile that was uniformly green with a thin
band of true image across the top. Sampling the rendered pixels gives
`R=0, G=87, B=0`. That is the arithmetic signature of an all-zero YUV buffer: with
`Y=Cb=Cr=0` the chroma terms drive red and blue below zero and they clamp, leaving
green alone positive (BT.709 full-range predicts `(0, 84, 0)`). It is a frame
buffer the decoder never wrote picture data into, not a colour-space fault — a
matrix or range error shifts the hue of a real picture, it cannot zero the planes.

Server counters for that session (363 s of media):

| Quantity | Value |
|---|---|
| Access units offered to the live consumer | 10842 (29.9/s) |
| Received by the publication owner | 1008 (2.78/s) |
| Evicted by the bounded live queue (capacity 4) | 9834 — **90.7%** |
| Keyframes that reached LiveKit | 66, against ~726 emitted |
| Send failures / stale drops / media rejections | 0 / 0 / 0 |

The source rate was confirmed independently of the metrics: the segment recorded
during the affected minute holds 900 frames in 30.083 s (29.92 fps), and its media
duration matches the wall-clock interval between segment files, so the media clock
tracks real time and the recorder took every access unit (10841 accepted, 10841
received).

The drain rate was then isolated from the server entirely. A standalone harness
reproduced this adapter's exact interface — bind the Unix socket, spawn `lk`, let
it connect, write Annex-B as fast as it will accept — and measured how fast `lk`
consumes it:

| Stream fed to `lk` | Frame size | Drain rate | Effective rate |
|---|---|---|---|
| 3.91 Mbps (the real webcam stream) | 16.3 KB | 44.8 KB/s | 2.82 fps |
| 0.29 Mbps (re-encoded) | 1.2 KB | 3.3 KB/s | 2.75 fps |
| 3.91 Mbps with `--fps 30` | 16.3 KB | 44.8 KB/s | 2.82 fps |

A 13× reduction in bitrate does not move the frame rate. The ceiling is
approximately **2.8 access units per second**, roughly 355 ms each, and it is a
frame ceiling rather than a byte ceiling. Nothing available changes it: `--fps 30`
produced a byte-identical result (1839104 bytes in 40.1 s) because that flag
applies only when files are published, and `--h26x-streaming-format` already
defaults to `annex-b`, which is what is written. `lk` held 1.4% CPU and the server
0.6% throughout, so this is not a resource limit.

That ceiling explains the green tile completely. The publisher accepts 2.8 frames
per second from a 30 fps camera, so the bounded live queue evicts the remainder by
age. Eviction is age-ordered and gives keyframes no protection, so IDRs were
culled at the same rate as everything else: a subscriber received a usable
keyframe about once every 5.5 s, and nearly every intermediate frame referenced a
picture that was never transmitted. A decoder without a valid reference emits its
uninitialised buffer.

Nothing in the system reported a fault. There were no send failures and no stale
drops, because frames were discarded upstream of the transport by a policy working
exactly as designed. Health reported `1/1 Publishing`, which was true — publishing
was occurring, at nine percent of frame rate.

### Why ADR 007's reversal criteria are met

ADR 007 said to replace the CLI boundary when a released official Rust SDK accepts
complete encoded access units with explicit timestamps, cancellation, keyframe
feedback, and bounded send feedback, and to reconsider if measurements showed the
CLI could not preserve representative streams.

The second condition is established above: the boundary cannot carry a 15 fps
stream, let alone 30.

The first condition is now satisfied, by a route ADR 007 did not track. That ADR
followed `livekit/rust-sdks` PR 1048, which remains open and was last touched
2026-06-11. The capability instead landed in **PR 1223, "Add pre-encoded video
passthrough publish path," merged 2026-07-14**, which adds a
`PassthroughVideoEncoder` implementing `webrtc::VideoEncoder` together with an
`EncodedVideoFrameBuffer`, covering H.264, H.265, VP8, VP9 and AV1. It shipped in
the `livekit` 0.8 series; 0.8.3 was published 2026-08-10. ADR 007 evaluated 0.6.3.

The released surface is a direct fit for the existing `H264AccessUnit` contract:

```rust
NativeVideoSource::new_encoded(resolution: VideoResolution) -> Self
NativeVideoSource::capture_encoded_frame(&self, frame: &EncodedVideoFrame<'_>)

pub struct EncodedVideoFrame<'a> {
    pub codec: EncodedVideoCodec,          // H264
    pub payload: &'a [u8],                 // Annex-B access unit
    pub timestamp_us: i64,                 // explicit capture timestamp
    pub frame_type: EncodedFrameType,      // Key / Delta
    pub resolution: VideoResolution,
    pub frame_metadata: Option<FrameMetadata>,
}
```

The access unit is passed whole with an explicit timestamp and an explicit
key/delta classification. That is precisely the contract ADR 007 required and
could not obtain.

## Options

1. **Keep the CLI and reduce the source rate to fit.** Rejected. The ceiling is
   ~2.8 fps regardless of bitrate, so fitting under it means publishing a
   slideshow. Live viewing is a stated product capability.
2. **Keep the CLI and protect keyframes in the live queue.** Rejected as a
   solution, adopted as a mitigation. Preserving IDRs converts an undecodable
   stream into a decodable one at ~2.8 fps. It removes the green frame, not the
   defect, and its value does not depend on this ADR.
3. **Build an in-process WebRTC publisher on a pure-Rust stack** (`webrtc-rs` plus
   WHIP), reusing the existing packetiser in `crates/livekit/src/rtp.rs`.
   Rejected for now. It avoids a native binary dependency but makes this project
   the owner of ICE, DTLS-SRTP, congestion control and their failure modes. ADR
   007 already judged that the highest-risk option, and nothing in these
   measurements changes that judgement.
4. **Publish through the released `livekit` Rust SDK's encoded passthrough path.**
   Chosen.

## Decision

### Decision 1 — Publish H.264 through the released `livekit` SDK's encoded passthrough path

`crates/livekit` depends on `livekit` 0.8 and publishes access units in-process
through `NativeVideoSource::new_encoded` and `capture_encoded_frame`. Media is not
decoded, re-encoded, or re-packetised by this project.

### Decision 2 — The change is confined to one implementation of `PublicationAdapter`

The trait boundary introduced by ADR 007 is what makes this reversible, so it is
kept. `PublicationOwner` keeps ownership of lifecycle, health, counters, retry and
backoff, shutdown, and the media subscription. The child-process adapter is
replaced, not the surrounding structure. LiveKit remains an attached consumer of
the media distributor and adds no upstream camera connection.

### Decision 3 — The native dependency is accepted and stated, not absorbed quietly

The `livekit` crate builds against libwebrtc, obtained as a prebuilt native
artefact rather than from crates.io sources. This is a real cost and it is the
principal argument the CLI boundary had in its favour. It affects build time,
`make check` hermeticity, the `cargo deny` and `cargo audit` posture, and any
future packaging on a deployment target that ADR 009 declined to guess.

It is accepted because the alternative is a live-video capability that does not
work. The build must be proven on this machine before the adapter is written; if
it cannot be, this ADR does not take effect and option 2 stands as the fallback.

Measured on this machine (Darwin 24.6.0, 2026-08-16) after `cargo clean`:
`cargo build --release -p livekit` took **1394 s** wall clock, after which
`cargo build --workspace --release` took a further **63 s**. A clean release
build is therefore about twenty-four minutes and is dominated by this
dependency; incremental builds are unaffected. `cargo deny check` and
`cargo audit` both pass with `ignore = []` — the dependency forced no
exception. The one `cargo audit` warning (`RUSTSEC-2025-0119`,
`number_prefix` unmaintained, reached through `indicatif` → `muxide`)
predates this ADR and is unrelated to it.

### Decision 4 — Keyframe cadence is an encoder concern, and is stated as a limitation

Encoding happens upstream in ffmpeg and MediaMTX, so this project cannot produce an
IDR in response to a receiver's Picture Loss Indication. Time to first picture on
subscribe is therefore bounded by the configured keyframe interval, not by
feedback. This is a known limitation of the pass-through design under both ADR 007
and this ADR, and it is why keyframe cadence is configuration rather than a
constant.

## Consequences

The live path can carry the camera's full frame rate, which is the point. Per-camera
child processes, the Unix socket, its write timeout, and their failure modes are
removed, and publication failures surface as typed Rust errors rather than as a
child's discarded stderr.

In exchange, this workspace now carries a large native dependency, and a
libwebrtc-linked publisher shares the process with recording. Failure isolation
between publication and recording, which the child process previously supplied by
construction, becomes something this project must demonstrate rather than assume.
`docs/phase-5/SPEC.md` treats that as an acceptance requirement.

Two adjacent behaviours have never run under real throughput and must be
re-examined once frames actually flow: the live queue's capacity of 4, and the
media-time pacing in `PublicationOwner`. The pacing currently never sleeps —
because the queue drops down to the newest items, each received unit's media time
lands behind wall-clock and the delay computation yields nothing — but that is an
accident of the starved path, not a property that survives the fix.

ROADMAP Phase 5 recorded subscriber decode as an unqualified gate. That gate is now
known to have been failing, and the roadmap entry is corrected rather than
retroactively rewritten.

### Decision 5 — Apple platforms refuse encoded publication rather than crash (added 2026-08-16)

Running the path revealed that encoded pass-through publication cannot work on
Apple platforms with `webrtc-sys` 0.3.42, and that its failure mode is fatal to
the whole process rather than to the publisher.

`TrackPublishOptions::video_encoder` does not select an encoder. It installs an
`EncoderSelectorInterface` on the sender
(`webrtc-sys-0.3.42/src/rtp_sender.cpp`, `FixedVideoEncoderSelector`), and
libwebrtc consults a selector only *after* an encoder exists. The first encoder
is built from the negotiated SDP format, which carries no backend tag, so
`VideoEncoderFactory::InternalFactory::Create`
(`webrtc-sys-0.3.42/src/video_encoder_factory.cpp`) takes its automatic-fallback
path. That path explicitly skips the pass-through factory —
`IsAutomaticFallbackBackend` returns false for `PreEncoded` — and picks the first
real backend supporting H.264. On Apple that is VideoToolbox, registered
unconditionally under `#ifdef __APPLE__`. The real encoder then receives a
pre-encoded access unit, `ToObjCVideoFrame` casts our native
`EncodedVideoFrameBuffer` to an `ObjCFrameBuffer`, and `objc_retain`
dereferences the payload as a pixel buffer.

Measured on this machine, four consecutive runs: exactly 9 access units
published (`surveillance_livekit_access_units_total` polled to the moment of
death), then `SIGSEGV` / `EXC_BAD_ACCESS` on the `EncoderQueue` thread. The
faulting stack is `objc_retain` ← `ToObjCVideoFrameBuffer` ← `ToObjCVideoFrame`
← `ObjCVideoEncoder::Encode` ← `VideoEncoderSoftwareFallbackWrapper::Encode` ←
`SimulcastEncoderAdapter::Encode` ← `VideoStreamEncoder::EncodeVideoFrame`.
Crash reports in `~/Library/Logs/DiagnosticReports/`. An Objective-C-level
segmentation fault is not a Rust panic: nothing catches it, and recording dies
with the process. That inverts the subsystem priority this project has committed
to, in which preserving recordings outranks serving live video.

`VideoEncoderBackend::list_available()` cannot be used to detect this. It is a
static enumeration — `video_encoder_backend_list()` pushes `PreEncoded`
unconditionally — not a probe of whether the backend is reachable for a given
sender. A guard written against it passes and protects nothing; it was removed
rather than kept as reassurance it could not provide.

Where no real H.264 encoder is registered, the same upstream code recovers:
initial creation fails, `OnEncoderBroken` fires, and the selector switches to the
pass-through backend.

**Corrected 2026-08-17: that escape hatch does not exist on any platform LiveKit
ships.** This ADR previously reasoned that the defect was specific to platforms
carrying a hardware H.264 encoder, and recommended relocating to Linux on that
basis. Reading the factory rather than assuming it shows otherwise:

- `IsAutomaticFallbackBackend` (`video_encoder_factory.cpp:176`) is
  `return backend != VideoEncoderBackend::PreEncoded;`. **Every** backend except
  pass-through is eligible for automatic fallback — `Software` included. Nothing
  privileges hardware here.
- The encoder factory template (`video_encoder_factory.cpp:290`) registers
  `webrtc::OpenH264EncoderTemplateAdapter` under `#if defined(WEBRTC_USE_H264)`,
  which is not platform-conditional.
- `WEBRTC_USE_H264` is scraped from the downloaded prebuilt's `webrtc.ninja`
  (`webrtc-sys-build-0.3.18/src/lib.rs:118`). Verified present in the artifact
  this repository actually links:
  `target/release/build/.../mac-x64-release/webrtc.ninja` carries
  `-DWEBRTC_USE_H264`.

So macOS registers *two* real H.264 encoders, VideoToolbox and OpenH264, and a
Linux prebuilt built from the same GN configuration registers OpenH264. On Linux
the automatic-fallback path would select OpenH264 instead of VideoToolbox — a
real encoder still wins over pass-through. Only the *crash signature* is
Apple-specific (`ToObjCVideoFrame` casting `EncodedVideoFrameBuffer` to
`ObjCFrameBuffer`); the *cause* is not.

What OpenH264 does when handed a native `EncodedVideoFrameBuffer` — return an
error, produce garbage, or fault on a null `ToI420()` — has **not** been
measured, and is not claimed here. What is established is that it will not
silently fall through to pass-through.

Not verified: the Linux prebuilt's own `webrtc.ninja`. That artifact was not
downloaded. The inference rests on the shared GN configuration and on H.264
being required for browser interop, and it should be confirmed by grepping that
file before anyone spends effort on a platform move.

So `NativePublicationAdapter` refuses to publish on `target_vendor = "apple"`,
returning `AdapterErrorKind::Unavailable`. This is a deliberate, visible
capability gap, not a silent degradation: the publisher reports `Terminal` with
category `AdapterUnavailable`, logs once, and the camera keeps recording.

Two defects in this project's own code were found because that refusal had to be
delivered honestly, and both are fixed:

- The send path discarded the adapter error's kind and reported every failure as
  `Transport`, which is retryable. A publisher that could never publish therefore
  reconnected on every backoff interval and reported itself merely `Degraded`.
  It now preserves the kind and stops terminally when reconnecting cannot help.
- The publication lifecycle had no `Publishing → Terminal` edge, because the only
  non-retryable failures had previously been raised at connect time. A publisher
  that had already sent access units can meet one on the send path.

Both are covered by
`owner::tests::a_non_retryable_send_failure_stops_publication_instead_of_reconnecting`.

A separate, unrelated macOS defect was fixed while getting this far: libwebrtc
ships Objective-C categories in archive members that define no other symbol, and
the linker drops them, so the first `PeerConnectionFactory` construction died
with `+[NSString stringForAbslStringView:]: unrecognized selector sent to
class`. `apps/server/build.rs` now passes `-ObjC` on Apple targets.

## Reversal criteria

Return to a process boundary, or to option 3, if the native dependency cannot be
built reproducibly on the confirmed deployment target; if in-process publication is
measured to interfere with recording preservation, which outranks live viewing; if
the encoded passthrough path is withdrawn or proves unable to carry representative
multi-slice High Profile streams; or if per-camera memory or CPU cost at the ten
and twenty camera workloads is measured to be materially worse than the child
process it replaces.

Revisit Decision 4 if encoding moves in-process, which would make PLI-driven
keyframe generation possible and change join latency from a configuration
consequence into a feedback-controlled one.

Reverse Decision 5 when `webrtc-sys` honours the requested encoder backend at
*initial* encoder creation rather than only through the asynchronous selector —
for example by tagging the pass-through factory's advertised formats so the
negotiated `SdpVideoFormat` carries the backend parameter. The check to re-run
is the same one that produced the measurement above: bring up the local stack
and watch `surveillance_livekit_access_units_total` climb past 9 without the
process dying. Vendoring and patching `webrtc-sys` locally would also reverse
it, but that is a material dependency decision and needs its own ADR; it is not
taken here.
