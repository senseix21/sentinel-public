# ADR 017: Independent person presence and face-only overlays

Date: 2026-10-09
Status: accepted for reversible local development; production gates remain open.
Supersedes ADR 016's one-mode restriction only when combined mode is implemented
and qualified. Existing person/face modes and wire contracts remain valid.

## Problem

The user observed that face-only detection loses a person who turns away. Human
presence and visible face geometry are different facts. Shrinking person boxes,
keeping an old face box or lowering a face threshold cannot supply hidden-face
evidence. Preserve face-only rendering while deriving presence independently.

## Proposed decision

Add `person_face`: one ingestion and detection decoder, immutable canonical
sampled frames, two bounded independently owned model lanes, and time-based
person tracking with explicit observed/tentative/uncertain states. Person presence
runs independently of motion and viewers. Face results alone drive rectangles.
Retain the existing model adapters as development candidates until representative
footage qualifies their accuracy and preprocessing.

Use distinct bounded presence snapshots and `presence/v1`; preserve `face/v1`
and `det/v1`. Do not imply persistent enter/exit events, recognition, unique people
or synchronized video frames. The precise contracts, limits, provisional settings
and acceptance gates are in [SPEC.md](../person-presence/SPEC.md).

## Alternatives and costs

- Face-only: retains current small slice but cannot meet back-facing presence.
- Person-only: supplies presence evidence but cannot localize face rectangles.
- Person-crop face inference: can save work but gates face recall on person recall
  and adds crop transforms. Defer until measurements justify it.
- Serial dual-model inference: simpler ownership, but a blocked face call also
  blocks person refresh. Separate lanes require two sessions/scratch budgets and
  explicit global admission/reservation; no extra decoder or upstream session.
- Head detection: provides head boxes from behind but changes overlay meaning and
  requires its own artifact/dataset. Not selected in this specification.

Model concurrency increases memory and CPU. Canonical 640 preprocessing may lose
small-subject detail; compare against source-derived tensors before freezing it.
The current tracker/gate cannot be reused unchanged as continuous presence.

## Failure and decision gates

A lane failure affects its capability; decode failure affects both. Recording and
live retain independent consumers. Foreign inference cancellation is not proved:
marking a timeout cannot forcibly stop an in-process ONNX call. If bounded
recovery from hung inference is required, qualify runtime cancellation or propose
an owned model-process boundary with measured cost and bounded media transport.

Production targets, model qualification, target hardware, Firefox and 10/20-camera
capacity remain open. No acceptance or production guarantee is implied by this ADR.

## Reversal

Keep explicit person/face settings. Restart into the prior mode, clear session/
track/UI state and stop/join owned lanes. No recognition/durable-presence schema
makes rollback depend on a storage migration.
