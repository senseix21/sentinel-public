# Architecture Context — 2026-08-10 21:35 +06

The one-ingest/many-consumer topology remains accepted. Webcam source conversion is a development-source adapter concern; the RTSP-to-LiveKit leg must remain encoded packet adaptation.

The LiveKit CLI Unix-socket publisher is now explicitly rejected as performance evidence and remains spike-only. Its tested path backpressured a 15 FPS source to about 2.8 FPS and caused upstream slow-reader drops.

The production publisher boundary must expose bounded queue capacity, input/output rate, consumer lag, drop count, and reconnect state. Live publication remains freshness-oriented and must never block the preservation-oriented recorder.
