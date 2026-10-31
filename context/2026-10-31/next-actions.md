# Next actions

1. Review the two cursor fixes, their stable regressions, ADR 018, and fuzz targets.
   Commands and scope are in `fuzz/README.md`; evidence is in the dated fuzz report.
2. For physical-camera work, obtain/configure the RTSP endpoint and credentials
   locally. Qualify H.264 RTSP/TCP ingest, recording, browser delivery, playback,
   and recovery without adding extra upstream camera sessions.
3. Extend fuzzing only as an explicitly scoped next slice: H.264 depayloading,
   socket framing, or configuration. Do not mark the overall Phase 8 item complete.
4. Carry forward the independent reliability, security, deployment and capacity
   work from [October 23](../2026-10-23/next-actions.md), including the normal
   macOS release failure. A successful unstripped workaround does not close it.
