# LiveKit H.264 Publication Spike

Date: 2026-08-10

## Question

Can the single-ingest media plane publish a compatible encoded H.264 stream to self-hosted LiveKit without a second camera connection or pixel decode/re-encode?

## Environment

- macOS development host
- LiveKit server 1.13.5 in local development mode
- LiveKit CLI 2.18.2 (reported SDK version 2.18.1)
- FFmpeg synthetic 640x360, 15 FPS, H.264 Baseline, `yuv420p`, two-second GOP, no B-frames
- H.264 Annex-B over a local Unix socket

Development credentials were the LiveKit local-mode placeholders and were not written into repository configuration. This setup is local test infrastructure, not a production security design.

## Reproduction

Run each long-lived command in a separate terminal from a temporary directory:

```bash
livekit-server --dev
```

```bash
ffmpeg -hide_banner -loglevel warning \
  -re -f lavfi -i testsrc2=size=640x360:rate=15 -an \
  -c:v libx264 -preset ultrafast -tune zerolatency \
  -profile:v baseline -pix_fmt yuv420p \
  -x264-params keyint=30:min-keyint=30:scenecut=0 -bf 0 \
  -listen 1 -f h264 unix:/private/tmp/surveillance-livekit-h264.sock
```

```bash
lk --dev room join \
  --identity camera-fixture \
  --publish h264:///private/tmp/surveillance-livekit-h264.sock \
  surveillance-spike
```

Inspect and subscribe:

```bash
lk --dev room participants list surveillance-spike
lk --dev room participants get \
  --room surveillance-spike \
  --identity camera-fixture camera-fixture
lk --dev room join \
  --identity observer --auto-subscribe surveillance-spike
```

Stop the observer, publisher, FFmpeg, and server using their owning terminals. A publisher disconnect produces an expected broken pipe at the Unix-socket FFmpeg writer.

## Observed result

- Publisher joined `surveillance-spike` and announced track `TR_VCCv6T8hpB7Sax`.
- Server metadata reported one active `VIDEO`/`CAMERA` track with MIME type `video/H264`.
- A second participant auto-subscribed to that same track.
- At shutdown, LiveKit reported `numPacketsForwarded: 9226` and `numPacketsDropped: 0` for the track forwarder.

The experiment passed its narrow question: compatible encoded H.264 can enter LiveKit and reach a subscriber without pixel transcoding. The Unix socket is merely the spike boundary; the production path must consume the media distributor's bounded encoded-packet output.

## Physical webcam follow-up

A second run used the built-in FaceTime HD Camera through the production-shaped development path:

```text
AVFoundation -> H.264 encode -> MediaMTX RTSP/TCP
             -> FFmpeg stream copy -> H.264 Unix socket
             -> LiveKit publisher -> LiveKit subscriber
```

Observed facts:

- AVFoundation supplied 1280x720 UYVY 4:2:2 at approximately 15 FPS.
- The source adapter converted to `yuv420p` and encoded H.264 Constrained Baseline with a two-second GOP and no B-frames.
- MediaMTX exposed one H.264 RTSP track.
- The RTSP-to-LiveKit FFmpeg leg reported `copy`; it did not decode or re-encode pixels.
- LiveKit published track `TR_VCHqUJnMK7vA4E` as `video/H264`, and participant `webcam-observer` subscribed successfully.

The first run did **not** pass real-time throughput qualification. The stream-copy bridge processed only about 2.8 FPS and 8.74 seconds of media during roughly 36 seconds of wall time. MediaMTX reported repeated slow-reader frame discards and ultimately closed the RTSP reader on an I/O timeout.

### Pacing isolation and correction

A deterministic 1280x720/15 FPS run set `lk --fps 15` but retained x264's `zerolatency` default of 11 sliced threads. It delivered about 1.4 frames per second. The ratio, 15 configured units per second divided by 11 slices per frame, predicted the observed output. This supports the conclusion that the CLI raw Annex-B reader paces slice NAL units as frames.

The correction forced one slice per encoded frame and retained explicit CLI pacing:

```text
FFmpeg: -threads 1 -x264-params sliced-threads=0:slices=1:...
lk:     --fps 15
```

The deterministic correction produced exactly 150 frames/10.00 media seconds in 9.97 wall seconds. LiveKit reported 4,701 forwarded RTP packets and zero dropped.

The physical webcam was then retested with the same one-slice constraint. After the LiveKit socket consumer connected, the RTSP stream-copy leg sustained approximately 15 FPS for about 59 wall seconds. A subscriber received track `TR_VCgw7iM76uZXPH`, and LiveKit reported 10,464 forwarded packets with zero dropped.

There was a separate startup-order failure: the RTSP reader was started roughly 11 seconds before the CLI connected to its listening Unix socket. During that interval MediaMTX reported slow-reader drops. No further slow-reader warnings occurred after the publisher attached. Production startup must therefore make the live publisher ready before enabling its upstream subscription, or deliberately shed live startup packets without blocking other media consumers.

Consequently:

- Webcam-to-LiveKit format compatibility and sustained 15 FPS publication are demonstrated for single-slice H.264.
- Multi-slice Annex-B cannot be passed blindly through the CLI file/socket reader with frame-rate pacing.
- No glass-to-glass/browser latency number may be reported from these runs.
- Production code must parse complete H.264 access units and use their timestamps; it must not treat an arbitrary NAL unit as a frame.

## Architectural consequences

- Keep exactly one upstream camera owner.
- Fan the same encoded packet source to recorder and LiveKit publisher.
- Permit explicit H.264 framing conversion and RTP packetization at the publisher boundary.
- Do not classify bitstream adaptation as a reason to decode and re-encode pixels.
- Do not embed `lk` as the production architecture without a separate lifecycle/security/operability decision.

## Not established

- Representative camera High Profile, SPS/PPS, timestamp, and reconnect behavior.
- Browser decode/render compatibility.
- End-to-end live latency or concurrent-viewer capacity.
- Real-time throughput through the production Rust publisher adapter and with representative camera access-unit structure.
- Publisher restart/backoff and recording isolation during LiveKit outage.
- H.265 live publication policy or fallback/transcode requirements.
- Suitability of a specific Rust LiveKit publishing API.

## Primary references

- LiveKit local server setup: <https://docs.livekit.io/transport/self-hosting/local/>
- LiveKit CLI encoded-track publication examples: <https://github.com/livekit/livekit-cli>
- LiveKit Rust SDK repository: <https://github.com/livekit/rust-sdks>
