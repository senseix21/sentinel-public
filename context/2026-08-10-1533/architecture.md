# Architecture Context — 2026-08-10 15:33 +06

The target crate/module boundaries now exist as a compile-safe scaffold. The protected runtime topology remains:

```text
Camera -> one ingestion owner -> bounded media distributor
                                  |-> recorder
                                  |-> detector -> domain event bus
                                  `-> LiveKit publisher
```

Recorder, detector, and LiveKit are direct media consumers. The event bus carries lightweight facts and never encoded packets or decoded frames.

The legacy root `src/` tree is outside the workspace. Reuse its GStreamer/FFmpeg/H.265 mechanics only through characterization tests and new contracts. Do not reconnect the legacy single-camera modules as a new application architecture.

All newly added crate modules are intentionally behavior-free. Public contracts must be designed during their first vertical slice after packet ownership, timebase, cancellation, queue capacity, overflow, and failure semantics are understood.

