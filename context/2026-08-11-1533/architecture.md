# Architecture — Incremental Persistence Seam

```text
muxer init -> ActiveSegmentFile::append
fragment 1 -> ActiveSegmentFile::append
fragment N -> ActiveSegmentFile::append
             |
             | graceful: sync -> rename -> probe
             ` crash: .partial.mp4 remains
```

The storage handle singularly owns the open file. It is async and does not
block a Tokio worker. The runtime has not yet been switched from whole-file
`persist`; that is the next bounded seam.
