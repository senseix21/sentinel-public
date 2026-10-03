# Decisions and Open Questions — 2026-10-03 22:37 +06

## Decisions

- In-process ONNX Runtime through `ort`, behind `PersonInference`, with YOLOX as
  a reversible documented default rather than a confirmed requirement (ADR 015).
  Apache-2.0 removes the AGPL obligation the Ultralytics families carry.
- `ort` is optional behind `onnx-inference`, because it has no prebuilt runtime
  for the development host. The gate splits into a deterministic half that runs
  anywhere and a feature half that needs a platform `ort` supports.
- `load-dynamic` rather than static linking, forced by the protobuf and Abseil
  collision with libwebrtc. The runtime becomes a deployment artifact.
- The production claim is runtime-sourced and under-claims when mixed.
- The detector decodes at the model's input geometry; the camera's own geometry
  survives in configuration, which is what detections are mapped back through.
- Thresholds are whole percentages, following the storage watermarks, because
  the settings value model has no float.
- The output decoder is stateless, taking thresholds per call, so classifying
  them as taking effect immediately is true rather than convenient.
- An unset `model_path` is a legitimate state: the fixture runs and the API says
  so. A path that is set and cannot be loaded refuses startup.

## Open questions and external gates

- The labeled dataset and the accuracy criteria. Phase 6's remaining item waits
  on these and on nothing technical.
- Target hardware and accelerator, open since 2026-08-14. If it turns out to be
  Intel rather than NVIDIA, OpenVINO with a person-only model becomes materially
  cheaper than ONNX with a COCO model, and ADR 015's runtime choice should be
  re-argued rather than inherited.
- Whether detection should default to disabled until accuracy criteria exist.
  `camera.detection.enabled` currently defaults to true, which was harmless when
  every result was visibly a fixture and is worth revisiting now that a
  deployment can configure a real model.
- Gate tuning. The threshold, cooldown and heartbeat are defaults nobody has
  measured against real footage; the heartbeat counter is the signal that the
  threshold is set too high to be doing work.
- Whether `ort`'s panic on an unloadable runtime should be contained by checking
  the library before touching `ort`. It would catch the common deployment error
  and not the rest.
