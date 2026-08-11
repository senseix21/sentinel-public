# Reproducible Media Spikes

Status: file-backed, synthetic RTSP, and webcam bridge executed on 2026-08-10; representative cameras remain pending

These commands validate narrow media mechanics. They do not validate RTSP, camera interoperability, LiveKit, or production performance.

## Observed toolchain

- FFmpeg/FFprobe 8.0 from `/usr/local/bin`, including libx264, libx265, VideoToolbox, and AVFoundation support.
- GStreamer executable exists at `/usr/local/bin/gst-launch-1.0`, but its version probe behaved abnormally in the execution environment and remains unqualified.
- MediaMTX was upgraded through Homebrew from 1.15.2 to 1.20.0 and qualified as a local RTSP fixture server.

## H.264 30-second segmentation

```bash
ffmpeg -hide_banner -loglevel warning \
  -f lavfi -i testsrc2=size=320x180:rate=10 \
  -t 125 -an -c:v libx264 -preset ultrafast \
  -g 300 -keyint_min 300 -sc_threshold 0 \
  -force_key_frames 'expr:gte(t,n_forced*30)' \
  -f segment -segment_time 30 -reset_timestamps 1 \
  h264-30s-%03d.mkv
```

Observed segment durations: 30.0, 30.0, 30.0, 30.0, and 5.0 seconds. All decoded fully with FFmpeg without reported errors.

## H.265 60-second segmentation

```bash
ffmpeg -hide_banner -loglevel warning \
  -f lavfi -i testsrc2=size=320x180:rate=5 \
  -t 125 -an -c:v libx265 -preset ultrafast \
  -x265-params keyint=300:min-keyint=300:scenecut=0:log-level=error \
  -force_key_frames 'expr:gte(t,n_forced*60)' \
  -f segment -segment_time 60 -reset_timestamps 1 \
  h265-60s-%03d.mkv
```

Observed segment durations: 59.8, 59.4, and 5.0 seconds. All decoded fully, but the muxer warned that it failed to avoid negative PTS values and the first stream began at 0.4 seconds. This result is deliberately not classified as clean segmentation.

## Probe and decode validation

```bash
ffprobe -v error \
  -show_entries format=filename,duration,size \
  -show_entries stream=codec_name,width,height,avg_frame_rate,start_time \
  -of json segment.mkv

ffmpeg -v error -i segment.mkv -map 0:v:0 -f null -
```

## Synthetic live RTSP copy path

MediaMTX 1.20.0 was run locally with its Homebrew configuration. Live H.264 and H.265 sources were generated, published over RTSP/TCP, consumed through a second RTSP/TCP session, and written with `-c:v copy`.

```bash
mediamtx /usr/local/etc/mediamtx/mediamtx.yml

ffmpeg -hide_banner -loglevel warning -re \
  -f lavfi -i testsrc2=size=640x360:rate=15 -an \
  -c:v libx264 -preset ultrafast -tune zerolatency \
  -g 30 -keyint_min 30 -sc_threshold 0 \
  -f rtsp -rtsp_transport tcp \
  rtsp://127.0.0.1:8554/fixture-h264-live

ffmpeg -hide_banner -loglevel warning \
  -rtsp_transport tcp -i rtsp://127.0.0.1:8554/fixture-h264-live \
  -t 12 -an -c:v copy rtsp-h264-live-remux.mkv
```

Observed H.264 result: 640×360, 15 FPS, 12.000 seconds, 1,776,091 bytes, start time 0.533 seconds, and a clean full decode.

The equivalent H.265 experiment produced a clean full decode at 640×360/15 FPS, 12.134 seconds, and a 1.8-second start offset. Repeating with `bframes=0` produced exactly 12.000 seconds but retained a 1.2-second start offset. B-frame reordering therefore does not fully explain the offset; joining mid-GOP and RTSP/RTP timestamp anchoring remain candidates.

Looping a previously segmented H.264 file as the publisher produced an available H.264 RTSP track but an empty downstream remux. That failed fixture is useful evidence: loop/discontinuity semantics must be tested deliberately rather than assumed from successful file decode.

## Mac webcam RTSP bridge

AVFoundation enumerated the built-in FaceTime HD Camera as video device 0. The camera supplies UYVY 4:2:2 at the tested mode, so explicit conversion to `yuv420p` is required for a broadly compatible H.264 fixture.

```bash
ffmpeg -hide_banner -loglevel info \
  -f avfoundation -framerate 15 -video_size 1280x720 -i 0:none \
  -an -vf format=yuv420p \
  -c:v libx264 -preset ultrafast -tune zerolatency \
  -g 30 -keyint_min 30 -sc_threshold 0 \
  -f rtsp -rtsp_transport tcp \
  rtsp://127.0.0.1:8554/webcam
```

The downstream ten-second copy remux was H.264 Constrained Baseline, `yuv420p`, 1280×720, approximately 15 FPS, 10.011 seconds, 2,224,648 bytes, and decoded fully. The bridge is suitable for manual development mechanics, not camera interoperability or production capacity evidence.

On 2026-08-10 the same webcam source was carried onward through an RTSP stream-copy adapter into LiveKit. The first multi-slice encoder run was paced incorrectly by the CLI/socket reader and fell to about 2.8 FPS. Forcing one H.264 slice per frame and setting `lk --fps 15` sustained approximately 15 FPS; LiveKit forwarded 10,464 packets with zero drops. Startup still dropped RTSP frames while the socket listener lacked a consumer. See `docs/phase-0/livekit-publication.md` for the access-unit and startup-order consequences.

## Deterministic reconnect and timestamp-reset fixture

Run:

```bash
scripts/phase0/rtsp-reconnect-fixture.sh
```

The harness starts MediaMTX in a temporary working directory, publishes generation 1, records/probes it, stops the publisher, verifies the path is offline, starts a fresh generation 2 with reset source timestamps, records/probes it, validates both outputs by decoding, and stops only its owned process IDs through cleanup traps.

Observed on 2026-08-10:

- Both generations were H.264 640×360 at 15 FPS and decoded successfully.
- The offline boundary was observed between publishers.
- Both fresh generations began on a keyframe at PTS/DTS 1.867 seconds and reset identically.
- Packet duration was approximately 0.066 seconds; keyframes appeared at 1.867 and 3.867 seconds.
- `-t 4` produced only 32 packets ending at PTS 3.933 because the input timeline was already 1.867 seconds when the reader joined.
- MediaMTX generated optional MoQ certificates only beneath the temporary fixture directory, never in the repository.

This proves that source PTS cannot drive wall-clock segment duration and that reconnect requires a new epoch/discontinuity. See `docs/phase-0/timestamp-contract.md`.

## Container crash-finalization fixture

Run:

```bash
scripts/phase0/container-crash-fixture.sh
```

The fixture compares normally finalized and forcibly interrupted H.264/H.265 Matroska, fragmented MP4, and classic MP4 outputs. Fragmented MP4 and Matroska retained decodable media after interruption; classic MP4 failed for both codecs. See `docs/phase-0/container-finalization.md` for the measured matrix and MVP decision.

## Required next spikes

1. Repeat H.264/H.265 with representative production RTSP sources without transcoding.
2. Extend the deterministic fixture to keep an ingestion client alive across controlled disconnect/reconnect once the Rust state machine exists.
3. Measure CPU, memory, output bitrate, and wall time with the accepted 4 MP/15 FPS qualification profile.

Do not commit generated media unless a small fixture has a documented license and test purpose.
