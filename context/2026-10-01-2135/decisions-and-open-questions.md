# Decisions and Open Questions — 2026-10-01 21:35 +06

## Decisions

- Own the FFmpeg child through `kill_on_drop` rather than through error paths.
  Cancellation is a normal outcome of supervisor shutdown, and a value that
  owns the process survives cancellation while a code path does not.
- Keep recording media retrieval as its own resource and leave the time-range
  playback endpoint untouched (ADR 014).
- Remove MediaMTX from production and local development (ADR 013), accepting
  that the proxy-backed transport, authentication, and reconnect fixtures are
  gone until an owned RTSP peer replaces them.
- Take transitive security patches as their own commits rather than folding
  them into feature arcs, so the lockfile movement stays legible.
- Split the gate into `verify` and `audit` so pull-request CI is deterministic.
  A newly published advisory is a true failure about the dependency graph, but
  it is not a statement about the change under review, and conflating the two
  trains reviewers to ignore red.
- Run pull-request CI on `macos-latest`, the development platform, accepting the
  higher minute cost. Nothing in `verify` needs a camera: the AVFoundation
  paths are runtime-only and the tests needing FFmpeg or a device are ignored.

## Open questions and external gates

- What happens to the four workspace members that hold no implementation:
  `crates/analytics`, `crates/api`, `crates/rules`, and `apps/worker`?
  `analytics` and `rules` are blocked on unconfirmed heatmap/analytics
  semantics and rule conditions; `apps/worker` is reserved until Phase 9
  measurements justify extraction; `crates/api` is vestigial because the HTTP
  boundary was implemented in `apps/server/src/health_http.rs`. This is the only
  thing preventing Phase 1 from closing, and it is a decision, not a code task.
- Which repository-owned RTSP peer replaces the deleted MediaMTX fixtures?
- Do Linux and Windows deployments need webcam capture as a product feature or
  only as developer smoke support?
- Should source selection become part of the persisted camera API?
- `FfmpegInput::Synthetic` already provides a deterministic camera-free H.264
  source but is reachable only from an ignored test. Should it be selectable
  through configuration so the stack can run without a camera, and is it a
  candidate for part of the replacement fixture set?
- Production camera codecs and profiles, viewer concurrency, latency
  objectives, target hardware, and deployment model remain unconfirmed. These
  are the Phase 0 answers that block Phases 7, 8, and 9.
