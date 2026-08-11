# LiveKit Rust Encoded-Publisher API Decision

Date: 2026-08-10

## Decision

Do not add the official LiveKit Rust SDK to the production workspace for pre-encoded publication yet.

The released SDK supports publishing tracks through its normal video-source path, but the official repository's direct pre-encoded video ingest work is still draft pull request #1048 as of this review. Depending on an unreleased branch would freeze the platform onto an API that its maintainers are still designing.

Keep the surveillance media contract SDK-neutral. Revisit the released SDK when encoded ingest lands, or evaluate another controlled adapter through an ADR if schedule requires publication earlier.

## Evidence

The official Rust SDK repository describes track publication and hardware/software encoding, but its public examples use raw media sources. The draft encoded-ingest change introduces the missing behavior:

- complete encoded access units rather than raw frames;
- codec, dimensions, capture timestamp, keyframe, and parameter-set metadata;
- passthrough without decode/re-encode;
- bounded queue-full feedback;
- keyframe and target-bitrate feedback;
- SPS/PPS caching/prepending for H.264;
- single-layer publication initially.

Primary sources:

- Official SDK repository: <https://github.com/livekit/rust-sdks>
- Official draft encoded-ingest PR: <https://github.com/livekit/rust-sdks/pull/1048>

## Local contract

`surveillance-media::packet` now provides:

- `MediaTimestamp`, whose timebase cannot be zero;
- validated immutable `H264NalUnit` values after RTP depayloading;
- `H264AccessUnitAssembler`, driven by explicit transport timestamps and end markers;
- complete `H264AccessUnit` values with keyframe and SPS/PPS classification;
- explicit errors for malformed NALs, RTP payload headers that were not depayloaded, timestamp changes before a boundary, and completed units without picture data;
- explicit discontinuity reset returning a discarded-NAL count for future metrics.

This contract deliberately does not infer access-unit boundaries from arbitrary Annex-B start codes. The RTP depayloader owns FU-A/STAP-A reconstruction and marker interpretation; the assembler validates the presentation-unit invariant.

## Required adapter semantics

The future `surveillance-livekit` adapter must:

1. Accept complete timestamped access units.
2. Preserve encoded payloads without decoding when codec/client compatibility permits.
3. Start the LiveKit publisher before attaching its bounded media subscription.
4. Use a freshness-oriented bounded queue and return/measure every drop.
5. Propagate keyframe requests and connection state.
6. Cache or require valid parameter sets before publishing a keyframe.
7. Own shutdown, reconnection, and task joining.

No broad publisher trait is introduced yet. A trait becomes justified when there is a supported SDK adapter plus a real test substitute with identical cancellation and backpressure semantics.

## Reversal criteria

Revisit this decision when an official LiveKit Rust release exposes pre-encoded access-unit ingestion, or when a Phase 5 schedule constraint justifies adopting and maintaining another publisher implementation through a separate ADR.
