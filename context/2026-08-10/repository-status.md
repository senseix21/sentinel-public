# Repository Status — 2026-08-10

## Snapshot identity

- Local date/time zone: 2026-08-10, Asia/Dhaka.
- Branch observed: `main`.
- HEAD observed: `9ec5175` (`more changes`).
- Repository state observed before these planning documents: untracked `analysis.md` and `Multi-Camera_Security_`; `AGENTS.md` was subsequently added locally.
- Implementation shape: one Rust binary crate named `cam_rtsp`, approximately 14 source modules plus manual test/example binaries.

Re-run `git status`, `git rev-parse HEAD`, and build commands before using this as current fact.

## Verified baseline

The checked-in manifest inherits 15 dependencies with `workspace = true`, but this repository has no workspace root and no parent workspace manifest at the expected location. The reported baseline command is:

```text
cargo check
error: failed to parse manifest
error inheriting `anyhow` from workspace root manifest
failed to find a workspace root
```

No implementation path should be described as compiling until Phase 1 repairs this and the relevant checks pass.

## What the prototype currently represents

- A single-camera RTSP record-and-forward experiment.
- One CLI `--stream-url`, with a public RTSP URL used as a default/example.
- A GStreamer H.264 recording path.
- FFmpeg/FFprobe-based forwarding, codec inspection, and clip mechanics.
- MediaMTX-related code that is disabled/unreachable from the current startup path.
- A flat Axum API without camera resources or an `/api` prefix.

This is reference material for media mechanics, not Phase 1 of the proposed multi-camera architecture.

## Known architecture gaps

- No stable camera identity or camera collection exists; `default_stream` is used.
- Recording, forwarding, and detection open independent connections to the same camera.
- No owned media-distribution boundary exists.
- No explicit reconnecting state machine or automatic recovery loop is implemented.
- Recording creates a continuous flat file rather than camera/time-partitioned segments.
- No recording database, PostgreSQL integration, retention, or storage-pressure policy exists.
- No real detector, tracking, ROI, heatmap, or analytics implementation exists.
- No LiveKit implementation exists.
- Task orchestration allows one major task finishing to terminate the whole process.
- No authentication, TLS boundary, metrics endpoint, percentile measurements, or automated Rust tests exist.
- Configuration is CLI/environment based; configuration APIs are placeholders and settings are not persisted.
- No first-class domain event bus exists.

## Known defects and misleading behavior

1. The recorder hardcodes `./recordings`, while clip lookup uses the configured recordings directory.
2. The GStreamer bus loop waits for Ctrl+C after the first idle `timed_pop`, preventing later bus messages from being drained.
3. Recording metadata assumes one hour, H.264, and AAC rather than inspecting actual media.
4. Clip extraction selects one nearby file, so it cannot serve a range spanning segments/files.
5. `-ss` before `-i` with stream copy produces keyframe-aligned rather than exact clip boundaries.
6. Forwarding start ignores supplied configuration and uses a hardcoded public source URL.
7. `StreamProcessor`, `FileRecorder`, and MediaMTX paths include dead or unreachable code; one clip API is a stub and clip-info is a placeholder.
8. Forwarding types and implementations are duplicated and have drifted.
9. The recorder is H.264-specific while a separate processor recognizes HEVC, producing inconsistent codec behavior.

## Potentially reusable knowledge

- GStreamer RTSP element wiring in `stream_receiver.rs`.
- FFmpeg/FFprobe invocation and process-management lessons.
- H.265 encoder configuration in `h265_encoder.rs`.
- Failure cases discovered by the prototype.

Reuse only behind new contracts and characterization/integration tests. Do not move dead code wholesale into the target workspace.

## Documentation added in this snapshot

- Root `AGENTS.md`: standing engineering constraints and workflow.
- Root `ROADMAP.md`: phased delivery plan and exit gates.
- `context/README.md`: context snapshot protocol.
- This dated context snapshot.

No production source behavior was changed by these documents.

