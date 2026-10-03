# Next Actions — 2026-10-03 22:37 +06

1. Obtain a YOLOX ONNX artifact and run `DI.R8` in the container, recording the
   model's SHA-256, the resolved execution provider, and the measured preprocess,
   inference and decode durations. Until then the adapter is proven to build and
   to refuse correctly, not to detect.
2. Decide whether `camera.detection.enabled` should default to false until
   accuracy criteria exist. A deployment can now configure a real model, which
   changes what an enabled-by-default detector means.
3. Measure the gate's duty cycle on real footage and tune the threshold,
   cooldown and heartbeat from it. Watch
   `detection_inference_heartbeat_total` against admitted frames: a ratio near
   one means the threshold is doing no work.
4. Obtain the Phase 0 answers that gate everything downstream, starting with
   target hardware. An Intel target would reopen ADR 015's runtime choice.
5. Add the accuracy dataset and criteria, which is the only route to closing
   Phase 6's last item. Nothing in the code can substitute for it.
6. Consider a pre-flight check of the ONNX Runtime library before `ort` touches
   it, so the common deployment error becomes a refusal rather than a panic.
