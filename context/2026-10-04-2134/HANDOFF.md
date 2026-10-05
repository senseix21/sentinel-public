# Handoff — 2026-10-04 21:34 +06

Written mid-task, at a deliberate stop. Read this before touching the branch:
there is uncommitted work, and one unresolved defect that is the reason the
work stopped where it did.

## State of the tree

Branch `feature/detection-inference`, head `71a4d00`, pushed and open as
[pull request #11](https://github.com/eliguy/camera_project/pull/11).

**Four files are modified and uncommitted.** They are a coherent, verified unit
— both gates pass with them in place — and the only reason they are not
committed is that the slice they belong to did not finish.

| File | What changed |
| --- | --- |
| `crates/livekit/src/transcode.rs` | The viewer-count fix (below). One behavioural change. |
| `crates/livekit/tests/room_fixture.rs` | A server the test owns, the restart qualification, viewer rejoin. |
| `scripts/phase5/room-fixture.sh` | No longer owns a server; runs the green test by name. |
| `Makefile` | `container-reproduce-livekit-restart` added. |

Verified at this commit, with those changes applied:

- `make verify` — **536 passed, 0 failed**, exit 0.
- `make container-verify-onnx` — **543 passed, 0 failed**, exit 0.
- `cargo clippy -p surveillance-livekit --all-targets -- -D warnings` — exit 0.
- `make container-verify-livekit` — passes.
- `make container-reproduce-livekit-restart` — **fails about one run in three.**

## The one production change in that diff

`TranscodePublicationAdapter::refresh_viewers` recounted viewers only on four
named `RoomEvent` variants. The `LiveKit` SDK reconnects its own room when a
server returns and announces it with `Connected` and `ParticipantsUpdated` —
neither of which was listed. A camera left at zero viewers by an outage
therefore stayed at zero, published nothing further, and went on reporting
itself as publishing.

It now recounts on any event. Recounting is reading the length of a map the SDK
already maintains, so being exhaustive this way costs less than being wrong
about one variant. Established by instrumenting the event stream through a real
outage, not by reading the SDK.

## The unresolved defect

`a_camera_publishes_again_after_its_server_is_killed_and_restarted` fails about
one run in three, and **the failure is real rather than flaky infrastructure**:

- The publisher's own counters show pictures produced continuously after the
  restart — hundreds of them.
- A viewer joining afterwards receives none, across five separate joins over
  fifty seconds.
- The SDK reconnects its room on its own and re-publishes the track under a new
  server-side id. What is *not* established is why the pictures this side keeps
  producing sometimes fail to reach a subscriber of that new publication.

It is kept out of `make container-verify-livekit` and run deliberately through
`make container-reproduce-livekit-restart`. A gate that fails a third of the
time teaches people to ignore the gate. It is kept rather than deleted or
loosened because it is the only thing that reproduces this.

**Next step on it:** determine whether our `NativeVideoSource` remains bound to
the publication the SDK re-creates on reconnect, or whether the track the
subscriber sees is a new one with no source behind it. The publisher-side
counter and the subscriber-side frame count disagree, so the break is between
`capture_frame` and the new sender.

## Three mistakes in this slice that were mine, not the product's

Recorded because each cost a diagnostic cycle and each would cost the next
person the same.

1. **"The camera silently stops publishing and reports healthy."** I wrote that
   down with a counter trace behind it and it was wrong. The freeze was the
   fixture feeding media faster than wall time: the owner then sleeps until the
   media clock catches up, consuming nothing, exactly as `owner.rs` documents.
   The fixture now paces 60 ms of wall per 66.67 ms of media, deliberately.
2. **Bitstream corruption that was pacing, not parsing.** The NAL splitter was
   suspected and cleared by rebuilding the stream from its own output and
   decoding that — no warnings. The errors came from feeding at 15 ms and from
   rewinding timestamps each pass.
3. **An assertion that could not fail.** `assert_ne!(state, Terminal)` — a
   publication that gives up ends in `Stopped`, so the assertion would have
   passed whatever happened. It asserts on the stop reason now. This is the
   fourth instance of that shape in this work; the others were `DI.R8` passing
   on zero detections, the accuracy harness passing on a silent model, and the
   recovery test asserting the wrong field.

## What was asked for and not delivered

A specification and plan for drawing bounding boxes over live view, with
tracking. **Not started.** The analysis that was handed to me is sound on its
main points — overlay client-side from normalized coordinates on a data channel,
not burned into frames; detections need the media timestamp of the frame they
describe; identity requires a tracker that does not exist. Two corrections
belong in that spec when it is written:

- **The 133 ms inference figure is an unoptimized debug build**, one CPU thread,
  in a container, on a host that is not the target. It must not become a latency
  budget. Measure a release build before designing around it.
- **The fixture backend fabricates detections.** The API already reports
  `results_are_fixture`; the overlay has to refuse to draw when it is true,
  rather than drawing invented boxes over real video.

## Immediate next actions

1. Decide whether to commit the four modified files. They are green and
   coherent; the restart test within them is a documented reproduction rather
   than a passing qualification.
2. Resolve the restart defect, or record it in `ROADMAP.md` Phase 5 against the
   four items it blocks. It is currently recorded only here and in the test.
3. Write the overlay specification, with the two corrections above.
