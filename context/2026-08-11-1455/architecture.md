# Architecture — Runnable Phase 2 Composition

```text
environment -> validation -> recovery discovery
                              |
                              v
                    one camera application owner
                      |                   |
                  RTSP ingest       fMP4 recorder
                      `--- bounded media --'
```

The binary owns process policy. Libraries own media/domain invariants. Ctrl-C
signals the application owner; it closes ingest before recorder drain and does
not detach tasks.
