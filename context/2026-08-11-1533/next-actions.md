# Next Actions — 2026-08-11 15:33 +06

1. Change `Fmp4SegmentSink` to emit init plus short bounded fragments without
   accumulating the complete recording file.
2. Join emitted chunks to `ActiveSegmentFile` in the owned recorder runtime.
3. Preserve keyframe-aligned 30–60 second file rotation and metadata timing.
4. Re-run direct and composed-server decode tests.
5. Add `SIGKILL` partial recovery qualification.
