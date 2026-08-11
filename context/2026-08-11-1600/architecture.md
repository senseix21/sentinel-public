# Architecture — Phase 2 Gate

```text
one Retina RTSP session
        |
        v
camera supervisor + lifecycle/atomic counters
        |
        v
bounded encoded H.264 distributor
        |
        v
fMP4 muxer (one-second bounded fragments)
        |
        v
active .partial.mp4 -- graceful --> sync/rename/probe/completed fact
        |
        `-------------- crash ----> retained reconciliation evidence

watch/atomic snapshots --> loopback /live, /ready, /metrics
```

The application owns the camera, recorder, health listener, shutdown signal,
and join policy. Health is observational and creates neither a media queue nor
another source connection. The active file has one owner; only finalized and
probed files can become completed segment facts.
