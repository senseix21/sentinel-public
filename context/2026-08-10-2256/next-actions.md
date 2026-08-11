# Next Actions — 2026-08-10 22:56 +06

1. Wire the recorder subscription to a narrow encoded-access-unit sink test
   boundary and prove saturation/closure outcomes end to end.
2. Add owned async task lifecycle only after the receive/wake/cancellation
   contract is explicit; do not detach recorder tasks.
3. Implement deterministic camera/UTC path construction in `storage` and prove
   it cannot escape the configured recording root.
4. Select and spike the H.264 fragmented-MP4 muxer adapter using representative
   access units.
5. Measure access-unit sizes and close remaining external Phase 0 gates.
