# Architecture Context — 2026-08-10 18:30 +06

The accepted MVP qualification workload is 10 continuous 4 MP/15 FPS H.264 High Profile camera streams, planned at 4 Mbps average with a two-second GOP, RTSP/TCP, ONVIF Profile T, no audio/PTZ, 60-second segments, 30-day retention, and 5 FPS detection sampling.

This baseline reinforces an encoded H.264-first vertical slice. H.265 remains a distinct compatibility path because RTSP copy remux works but timestamp anchoring and segment behavior are not yet cleanly understood.

The webcam bridge requires explicit yuv420p conversion. Its output is a manual RTSP mechanics source and must never substitute for representative ONVIF camera qualification.

RTSP readers can join mid-GOP and therefore observe a positive start offset even without B-frames. The packet/segment contract must preserve source PTS/DTS/timebase and distinguish source timeline, segment-relative timeline, and wall-clock capture time.

