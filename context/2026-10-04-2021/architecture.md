# Architecture — 2026-10-04 20:21 +06

Detection gained a real backend without changing the shape of anything around
it. The pipeline, the event contracts and the API fields are the same; what
changed is what sits behind `PersonInference` and where the claim about it comes
from.

## Two seams, not one

`PersonInference` stays the substitution boundary, and the feature flag makes it
one in the build rather than only in prose: a tree without `onnx-inference`
contains no ONNX code at all and still ships.

Beneath it, `ModelOutputDecoder` isolates the only part that changes with the
model — the output tensor's layout. It receives numbers and geometry, no frame
and no session, which is what lets the riskiest arithmetic in detection be
tested against a tensor written by hand.

## The claim follows the object

A build can carry the ONNX backend and run with no weights, so what the build
contains stopped being evidence of what is running. The descriptor is taken from
the backend each pipeline actually constructed, travels on the detection health
snapshot, and is aggregated at `GET /api/system`. The aggregate under-claims by
construction: any fixture among the running detectors makes the deployment a
fixture deployment.

## Work moved to where it is cheapest

The decoder scales to the model's input geometry, so the resize happens in
swscale rather than Rust and the letterbox factors become constants derived from
configuration. The adapter copies rows and never resizes.

The gate sits after sampling and before inference. It reduces inference only:
H.264 inter-frames cannot be decoded in isolation, so decode runs continuously
however much the gate skips. Moving that saving upstream would require control
of the camera.

## The runtime is loaded, not linked

Forced rather than chosen: ONNX Runtime and libwebrtc each vendor protobuf and
Abseil, and both reach every binary above `crates/config`. Dynamic loading keeps
ONNX Runtime's symbols in its own object, and is what an on-device Jetson
runtime needs in any case. The cost is that the library joins the model as a
deployment artifact, and that its absence is a panic inside `ort` rather than a
refusal.

## Preprocessing belongs to the artifact

Channel order and padding placement are not configuration and not style. A model
answers best to the preprocessing it was trained with, and that is a fact about
one file rather than about a family: YOLOX changed both conventions between
0.1.1rc0 and 0.2.0. Both are therefore constants in the code, each with the
measurement that chose it, and `LetterboxAlignment` emits the `FFmpeg` filter
expression and the inverse offsets from the same value so the frame producer and
the box reader cannot drift apart.

The defect this guards against has no symptom. Centred padding was
self-consistent: forward and inverse agreed, every box fell inside the frame,
and the detections were on the people. It was wrong only against the
distribution the model was trained on, and only a set with ground truth could
see it.

## A start that failed is not a component that is missing

The live decoder distinguishes a decoder binary that is absent from one that
could not be started this time. The first stops publication, because reconnecting
cannot conjure a binary. The second is retried, because a deployment replacing
`ffmpeg` while a camera starts publishing is over in a second and the camera
should come back without an operator. The errno is logged where the
classification discards it, which is the difference between diagnosing that in
minutes and reproducing it under load to find out.
