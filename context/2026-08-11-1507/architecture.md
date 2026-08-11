# Architecture — External Qualification and Crash Gap

The graceful path is now proven through the real composition root:

```text
FFmpeg -> MediaMTX -> server -> bounded distributor -> recorder -> durable fMP4
                                      SIGINT -> ordered drain/finalize -> exit 0
```

The crash path is not yet equivalent to the intended ADR. `Fmp4SegmentSink`
owns a bounded `Vec` for the active segment, then `SegmentFileStore` writes a
partial and atomically renames it during finalization. Incremental active bytes
must reach the partial file before forced-exit recovery can preserve evidence.
