# Architecture — owned runtime boundary

Unchanged from the 2026-08-12 snapshot. One ingestion owner per camera holds a
single Retina RTSP/TCP session; recorder, LiveKit publisher, and detection
attach to the bounded media distributor and add no upstream connections.
Detection is independently degradable and cannot stop recording or live view.

Planned detection insertion points (designed, not implemented):

- A motion gate in `crates/detector/src/motion.rs` between decode and sampling.
  It evaluates every decoded frame; the sampler rate-limits inference within an
  open window. It holds a reference grid, so it resets on `connection_epoch` and
  on geometry change.
- An ONNX adapter in `crates/detector/src/onnx.rs` implementing the existing
  `PersonInference` trait. `DetectionOwner::start<D, I>` is already generic over
  inference, so the owner loop, queue, health surface, and isolation tests are
  unaffected.
- A `Warming` detector state is required because TensorRT builds a
  device-specific engine on first run, taking minutes. Detection must not block
  server startup.

Working assumption: hardware H.264 decode (`h264_nvv4l2dec`) replaces software
decode on Jetson. This changes existing Phase 6 decoder code and is expected to
move the camera ceiling more than the model choice does.
