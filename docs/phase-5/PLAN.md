# Phase 5 Rework — Implementation Plan

Contract: `docs/phase-5/SPEC.md`. Decision: `docs/adr/011-native-encoded-livekit-publication.md`.

Slices are ordered so that the highest-risk unknown is settled first and each
slice leaves the workspace compiling. Item IDs match the SPEC's requirement IDs
where a slice closes one.

## Slice 0 — P5N.1, prove the dependency builds (done)

Build a throwaway crate outside the workspace that depends on `livekit` 0.8 and
calls the encoded passthrough surface, so that the build cost and the API shape
are established before any code in this repository depends on them.

Result, 2026-08-16, macOS 24.6.0 arm64: `livekit` **0.8.3** resolved and built
clean. `libwebrtc-sys` 0.3.45 pulled a prebuilt native artefact. The spike binary
constructed `NativeVideoSource::new_encoded` and called `capture_encoded_frame`
without panicking, printing `encoded passthrough API compiles`.

The build half of P5N.1 is closed. The `cargo deny` / `cargo audit` half is not,
and is checked in slice 6 once the dependency is actually in the workspace.

## Slice 1 — P5N.5, keyframes survive live-queue eviction

Independent of everything else, and worth doing whether or not the native adapter
lands. The bounded live queue evicts oldest-first with no regard for frame type,
so under overload it can retain four consecutive P-frames whose reference was
discarded. Preserve the most recent keyframe so that whatever survives is
decodable.

Touches `crates/media/src/distribution.rs`. Deterministic test: overflow the queue
with a keyframe followed by more than capacity delta frames, assert the keyframe
is still delivered before the deltas that depend on it.

## Slice 2 — coded dimensions derived from the stream

`NativeVideoSource::new_encoded` requires a resolution at construction, and
`EncodedVideoFrame` carries one per frame. Taking it from configuration would
fabricate metadata, which this repository forbids, so parse it from the SPS the
access unit already carries.

New module in `crates/media` implementing exp-Golomb parsing of the fields that
determine coded size — `pic_width_in_mbs_minus1`, `pic_height_in_map_units_minus1`,
`frame_mbs_only_flag`, and the frame cropping offsets — with the chroma format
handled explicitly rather than assumed 4:2:0. Returns a typed error rather than a
guess when the SPS is absent, truncated, or uses a profile whose SPS this parser
does not fully model.

Tests use SPS byte strings from real captures, checked against the dimensions
`ffprobe` reports for the same stream. A parser that cannot be checked against
actual media is not accepted.

## Slice 3 — extend the adapter's value type

`EncodedAccessUnit` currently carries `annex_b`, a 90 kHz `rtp_timestamp`, and the
publication `epoch`. The native path additionally needs an explicit key/delta
classification and a capture timestamp in microseconds, both of which the owner
already holds.

This amends the value type, not the trait's shape: `connect` / `send_access_unit` /
`disconnect` / `timestamp_authority` are unchanged and `PublicationOwner` keeps
ownership of lifecycle, retry, health and shutdown, which is what SPEC C1 protects
and what makes ADR 011 reversible. The CLI adapter and the in-memory test adapter
absorb the new fields without behaviour change, so this slice is provably neutral:
the existing owner and adapter tests must pass untouched.

## Slice 4 — P5N.2, the native adapter

New `NativePublicationAdapter` in `crates/livekit`, implementing
`PublicationAdapter`:

- `connect` mints a publisher token (`can_publish: true`, `can_subscribe: false`)
  through the existing `token.rs` machinery, connects the `Room`, creates the
  encoded `NativeVideoSource` at the dimensions from slice 2, and publishes the
  track. Resolution is not known until the first parameterized keyframe arrives,
  which the owner already waits for, so track creation is deferred to the first
  send rather than faked at connect time.
- `send_access_unit` calls `capture_encoded_frame` with the access unit's bytes,
  its capture timestamp, and its key/delta classification. No decode, no re-encode,
  no packetisation.
- `disconnect` unpublishes, closes the room, and joins whatever the SDK requires
  this project to own. No detached task; no `unwrap`, `expect`, or `panic!`.
- SDK errors map onto the existing `AdapterErrorKind` categories so that the
  owner's retry classification is unchanged. `ChildExited` becomes unreachable on
  this path and is retained only while the CLI adapter still exists.

Unit tests cover error mapping and the deferred-track rule. Publication against a
real server is a fixture, not a unit test.

## Slice 5 — P5N.3, remove the child-process boundary

Once slice 4 is qualified against a live server: delete `CliPublicationAdapter`,
the Unix socket, `ipc_write_timeout`, `cli_path`, `cli_version`, and the RTP
packetiser if nothing else uses it. Remove the corresponding configuration keys
and their persisted defaults rather than leaving them accepted and ignored, and
update `scripts/phase5/livekit-fixture.sh` and the Phase 5 docs that name `lk`.

Ordering matters: nothing is deleted until the replacement has been measured, and
the deletion is a separate slice so a revert is a revert of one change.

## Slice 6 — P5N.1 remainder, supply-chain posture

`cargo deny check` and `cargo audit` with `livekit` 0.8 present. `make check` end
to end, including release build. Record the build-time cost the dependency adds,
and record any `deny.toml` exception it forces with its reason — an exception added
silently to make a gate pass is a defect, not a fix.

## Slice 7 — P5N.10, make this failure class visible

Publication health today can report a healthy state while the live queue discards
most of what it is offered. Expose the offered-versus-published relationship —
the live queue's drop counter alongside the publisher's accepted counter — in
`/metrics` and in the per-camera health snapshot, and add an alert rule to
`docs/phase-8/alerts.yml` firing when the published fraction falls far below the
offered rate while the publisher reports itself healthy.

Metric names are verified with `scripts/phase8/check-metric-names.py`, which
exists because an invented metric name yields an alert that can never fire.

## Slice 8 — P5N.11, re-derive the settings that were never exercised

With frames actually flowing, measure and then set: the live queue capacity, which
is currently 4 and was never load-bearing; and the media-time pacing in
`PublicationOwner`, which currently never sleeps because a starved queue always
yields units whose media time trails wall-clock. Both become configuration with a
documented meaning and a stated default, not constants.

Separately correct the local capture settings, which are development fixtures
rather than product configuration: the webcam publisher emits 3.91 Mbps
unconstrained `ultrafast`, and its configured 15 fps does not take effect on this
device.

## Slice 9 — P5N.4, P5N.6 through P5N.9, measured evidence

Run against the local stack on a clean recording root, and record the results in
`docs/phase-5/local-evidence-2026-08-16.md`:

- P5N.6 published rate versus source rate, from `/metrics` over two intervals,
  with the source rate derived independently from a recorded segment's frame count
  and duration.
- P5N.7 subscriber decode, from a browser session reporting `readyState`,
  `videoWidth`/`videoHeight` and advancing `currentTime`, plus a sampled pixel
  check that rejects the all-zero YUV signature. The pixel check is the part that
  distinguishes this from the failure it replaces; a screenshot is not evidence.
- P5N.8 time to first picture, stated together with the keyframe interval that
  bounds it.
- P5N.9 recovery across a LiveKit server restart and a camera reconnect.
- P5N.4 recording admission uninterrupted throughout, which the child process used
  to guarantee structurally and in-process publication must now demonstrate.

`ROADMAP.md` Phase 5 is then updated against this evidence, using its legend.
Line 98's claim that no released Rust API accepts encoded publication is false as
of `livekit` 0.8 and is corrected. Line 326's `[!]` subscriber-decode gate is
resolved only by P5N.7, and only for this camera on this machine — it stays `[!]`
for representative cameras, browsers, and target concurrency, which remain
external.

## Not in this plan

No camera-count or viewer-count claim; that is Phase 9 and needs a documented
workload. The `RecorderBackpressure` defect that stops the camera on a large
recording root is worked around here with a clean root and remains open.
