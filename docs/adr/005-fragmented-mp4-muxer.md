# ADR 005: Fragmented-MP4 Muxer Boundary

Status: accepted for the qualified Phase 2 H.264 recording slice

Date: 2026-08-11

## Context

Recording must consume the one existing encoded-media distribution path. A
second FFmpeg/GStreamer RTSP connection would violate ingest-once. Feeding a
child process through blocking stdin adds another process, buffering, and task
lifetime boundary. The first slice needs video-only H.264 fragmented MP4 with
explicit timestamps, SPS/PPS, keyframe cuts, and bounded memory.

The current media contract exposes presentation timestamps but not decode
timestamps. Therefore arbitrary H.264 B-frame streams are not yet safe to mux;
the initial source must be qualified as decode-order/PTS-equals-DTS or the media
contract must gain truthful DTS.

## Decision

Spike pinned `muxide` 0.2.5 behind `recorder::fmp4`. It accepts complete H.264
access units from the existing distributor, converts NALs to MP4 length-prefix
framing, uses actual SPS/PPS, normalizes the 90 kHz source timestamp per segment,
and cuts only before a keyframe after the configured 30–60 second target.

The adapter bounds both encoded bytes and sample count and permits only one
completed segment awaiting storage. Qualification of the composed server found
that configuring one muxide fragment equal to the 30-second file interval keeps
the complete active file in memory and leaves no recoverable media on abrupt
process loss.

Therefore file duration and internal fragment duration are separate invariants:

- recording files still rotate after the configured 30–60 seconds on a
  keyframe;
- the muxer emits shorter bounded `moof+mdat` fragments (provisional default:
  one second);
- init bytes and each complete fragment append to the active `.partial.mp4`;
- normal completion syncs, probes, and atomically publishes that same file;
- memory contains at most the muxer's current fragment plus one fragment being
  written, not the entire recording file.

The adapter does not claim support for audio, H.265, B-frames, or
representative cameras.

## Alternatives

- GStreamer `appsrc` plus `cmafmux`/`mp4mux`: mature but introduces a native
  dedicated-thread/FFI lifecycle. Retain as reversal path.
- Supervised FFmpeg stdin: broad compatibility but requires a specified async
  packet protocol and child-process backpressure/failure semantics.
- Custom ISO BMFF writer: maximum control but unjustified implementation and
  maintenance cost before a maintained library is qualified.

## Reversal criteria

Replace the adapter if real streams fail conformance/playback, the crate cannot
represent required DTS/composition offsets, output is not recoverable under the
selected durability policy, or its maintenance/API risk outweighs GStreamer.

## Evidence

- The deterministic authenticated RTSP fixture produced and fully decoded three
  consecutive 30-second files through both the recorder and composed server.
- Abrupt `SIGKILL` left an incrementally persisted, decodable partial; restart
  classified it for reconciliation without publishing or deleting it.
- The configured memory budget includes both the current fragment and one
  emitted fragment. File size is independently bounded by the active writer.

## Evidence still required

- Representative High Profile stream with confirmed B-frame/DTS behavior.
- Memory and CPU measurements at 4 MP, 15 FPS, and planning bitrate.

## Primary references

- <https://docs.rs/muxide/0.2.5/muxide/fragmented/index.html>
- <https://github.com/Michael-A-Kuykendall/muxide>
