# Architecture — direct-camera qualification slice

## Accepted production topology

```text
physical IP camera -> one Retina RTSP/TCP owner -> bounded distributor
                                                    |-> recorder
                                                    |-> detector
                                                    `-> LiveKit publisher
```

MediaMTX remains a deterministic RTSP peer for integration tests and the macOS
webcam bridge. It is not a production hop and is not replaced by LiveKit Ingress.

## Ownership and failure boundaries

- Camera owns the one upstream authenticated session, reconnect epoch, media
  capability, timestamps, and teardown.
- Recorder, detector, and LiveKit own bounded subscriptions only.
- LiveKit and detector failure do not change camera connectivity or recording
  ownership.
- Qualification tooling owns the release server and sampler processes it starts;
  its evidence is private by default and contains no configured RTSP URL.
- A production RTSP proxy would be a new topology decision requiring measured
  incompatibility and an ADR; a failed camera test does not insert one implicitly.
