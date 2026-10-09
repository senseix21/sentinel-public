# ADR 016: Face-only live detection with an explicit inference mode

Status: accepted as a reversible local-development default; production model
qualification remains open.
Date: 2026-10-09

## Context

The user confirmed that the live detection outline must surround only a face,
and selected 70% as the default confidence threshold. YOLOX person output cannot
localize faces. Shrinking its boxes would fabricate spatial information. Face
observations must not change the meaning of existing person events or counts.

## Options and decision

Use direct full-frame YuNet inference through the existing optional ONNX Runtime
boundary. The existing detection owner selects one model per camera: person or
face. It retains one decoder, sampled frames, bounded blocking inference and the
same upstream media distributor. Person-crop face inference was rejected for the
first slice: it needs two models, adds crop transforms and misses faces whenever
the person detector misses. Running both independently is deferred until there
is a requirement and a measured per-camera resource budget.

Choose OpenCV Zoo's **face_detection_yunet_2023mar.onnx** as the reversible default:
MIT-licensed, SHA-256
`8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4`.
Download from pinned upstream commit
`47534e27c9851bb1128ccc0102f1145e27f23f98`; retain the license in
`docs/face-detection/YUNET-LICENSE`. The actual artifact is fixed **1x3x640x640**
under ONNX Runtime, despite OpenCV examples using 320. Warm-up at 320 failed;
640 passed. Input is unnormalized float BGR in CHW order. Existing FFmpeg
aspect-preserving, top-left padding and its inverse supply source-frame boxes;
this is a qualified local preprocessing choice, not a broad camera-accuracy claim.
The default is CPU with one intra/inter-op thread. No face identity is inferred.

A dynamic re-export would permit alternate input sizes but requires separate
artifact qualification. Other model families require their own output adapters
and verified redistribution terms; similarity of file extension is insufficient.

## Contracts and costs

Distinct `FaceDetectionSet` and `FaceDetectionObserved` preserve person event
semantics. The owner dispatches through `DetectionInference`; faces never
implement `PersonInference`. No tracker or coasted face boxes are emitted.
Face mode uses `face/v1`, string u64 epoch/sequence fields, a 16 KiB packet
bound, and at most 64 boxes. Room-scoped relays prevent events from another
camera leaking into a room. Person mode retains `det/v1`.

The model has 8,400 grid locations. The adapter validates the input and all
12 declared output shapes before warm-up. Scratch tensor storage is
3*640*640*4 = 4,915,200 bytes, plus runtime weights/workspace and bounded
candidate results. Decoder RGB frames require 1,228,800 bytes each. These model
costs are distinct from the existing encoded-queue budget; no total-memory or
camera-capacity claim follows from it. Capacity omissions and relay drops have
observable counters. Mode/artifact/threshold changes require pipeline restart.

The browser displays received face observations, clears on empty output or
unavailable/disconnected lifecycle, and drops old sequences/epochs. The existing
three-second receipt-age fallback remains. Source media time does not align to
the live adapter's clock, so frame-accurate positioning is not claimed. Faces
are labeled as faces; there is no unique-person or unique-face total in face mode.

Bad model startup becomes terminal detection health while recording/live
continue. Invalid configuration is rejected before construction. There is no
silent fallback to guessed, fixture or person boxes in face mode.

## Evidence and limits

The actual Rust adapter found one face at score 0.943 in a torso-visible public
astronaut fixture, with a localized normalized rectangle around the face, and
zero faces on blank input. Thirty local CPU iterations initially measured
p50 40,962 us, p90 42,200 us, p99 43,345 us. These are development-build adapter
measurements, not live latency, production accuracy, or supported camera count.
Further validation is recorded in `docs/face-detection/VERIFICATION.md`.

Representative labeled cameras, minimum usable face size, pose/lighting coverage,
accuracy criteria, target hardware and latency objectives remain open. External
model papers and person-model COCO results do not close those gates.

## Reversal

Set `SURVEILLANCE_DETECTION_MODE=person` to restore the YOLOX launcher path.
Person artifact overrides remain separate from face ones. Removing face mode
requires no storage migration: this slice adds no durable face records, crops,
embeddings, recognition service, or video pixel overlays.

## Sources

- [OpenCV Zoo YuNet model and licensing](https://github.com/opencv/opencv_zoo/tree/47534e27c9851bb1128ccc0102f1145e27f23f98/models/face_detection_yunet).
- [OpenCV reference decoder](https://github.com/opencv/opencv/blob/4.x/modules/objdetect/src/face_detect.cpp): stride outputs, BGR blob, sqrt(cls*obj), box decoding and NMS.
- [Specification](../face-detection/SPEC.md) and [implementation plan](../face-detection/PLAN.md).
