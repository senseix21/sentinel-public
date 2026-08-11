# Decisions and Open Questions — 2026-08-10 15:33 +06

## Decisions made

- The repository root is now a virtual Cargo workspace rather than the legacy `cam_rtsp` package.
- The legacy `src/` tree is preserved unchanged and excluded from compilation.
- Workspace packages share edition 2021, Rust 1.80 minimum, dual MIT/Apache-2.0 licensing metadata, forbidden unsafe code, and strict Clippy defaults.
- Both requested application packages exist, but the worker must remain behavior-free until a measured workload justifies independent deployment.
- ADRs 001–003 remain proposed because their detailed semantics require Phase 0 evidence.

## Still open

All product and media questions in the prior `2026-08-10` snapshot remain unresolved, especially camera codec/resolution/FPS/bitrate, hardware, playback accuracy, LiveKit codec requirements, detection model, heatmap meaning, failure objectives, and p90 targets.

The dependency versions and media libraries for production crates are intentionally not selected yet. Add them with the first real vertical slice, based on its requirements and spikes.

