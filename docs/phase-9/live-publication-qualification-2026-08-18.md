# Live publication qualification — 2026-08-18

This evidence closes frame accounting for the current local H.264 publication
path and supplies the first viewer fan-out CPU curve. It is not a camera-count,
latency, browser-rendering, or production-hardware claim.

## Workloads

All controlled runs used one synthetic RTSP/TCP H.264 Baseline camera at
1280x720, 15 FPS, a 15-frame GOP, recording enabled, detection disabled, and a
60-second measurement window after subscribers attached. The server, MediaMTX,
LiveKit, sources, and `lk` subscribers shared the same Mac.

The camera-specific check used the built-in Mac webcam through the repository's
AVFoundation -> FFmpeg -> MediaMTX bridge at 1280x720, configured 15 FPS and
1.5 Mbit/s, with one `lk` subscriber.

## Frame accounting

Controlled runs:

| viewers | access units | published frames | adapter drops | stale drops | live evictions | decoder restarts |
|---:|---:|---:|---:|---:|---:|---:|
| 1 | 926 | 926 | 0 | 0 | 0 | 0 |
| 2 | 927 | 927 | 0 | 0 | 0 | 0 |
| 4 | 928 | 928 | 0 | 0 | 0 | 0 |

The exact webcam path advanced `access_units_total` and
`published_frames_total` by 301 between Unix timestamps 1787050061 and
1787050081: 15.05 frames/s, with zero stale drops and zero live evictions.
Three contemporaneous recorder segments each contained 450 frames over exactly
30.000 seconds according to `ffprobe`, independently proving 15.000 FPS source
rate. The earlier 8.85 FPS observation is not reproducible on the current
pacing implementation; no unsupported retrospective root cause is assigned.

## Viewer fan-out CPU

Average CPU is consumed CPU-seconds divided by measured wall seconds; 100% is
one saturated core. Values are process-local, not host-wide.

| viewers | server | LiveKit | MediaMTX | synthetic source | each `lk` viewer |
|---:|---:|---:|---:|---:|---:|
| 1 | 14.5% | 4.7% | 1.7% | 15.0% | 1.9% |
| 2 | 15.4% | 5.8% | 1.7% | 15.9% | 2.7% |
| 4 | 15.2% | 8.3% | 1.8% | 16.2% | 4.3–4.4% |

The server's decode-and-encode cost is approximately flat as viewers increase;
fan-out cost appears in the SFU and subscribers. The samples are too short and
the host too unlike the unconfirmed deployment target to establish capacity.

## Reconnect with viewer attached

The webcam bridge was stopped and restarted while one subscriber remained in
the room. The camera advanced to connection epoch 2 and counted one reconnect;
publication counted exactly one timestamp discontinuity, remained state 3 with
one viewer, and counted zero decoder restarts and zero stale drops. After
recovery, access units and published frames each advanced 733 over 49 seconds
(14.96/s); live evictions stayed fixed at five. The subscriber connection did
not close or resubscribe.

This proves server-side continuity and resumed delivery to the existing
subscriber. It does not measure the viewer's freeze duration or first rendered
post-reconnect picture; those require a viewer-side rendering harness.

## Harness correction

`scripts/phase9/livekit-load-fixture.sh` used `seq 2 "$CAMERAS"` for added
cameras. BSD `seq 2 1` emits `2 1`, so a legitimate one-camera baseline tried to
add camera 02 and then duplicate camera 01. Arithmetic loops now make the empty
range explicit. `bash -n` passes, and the corrected one-camera workload ran to
completion four times.
