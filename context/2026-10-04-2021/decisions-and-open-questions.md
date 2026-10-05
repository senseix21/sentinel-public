# Decisions and Open Questions — 2026-10-04 20:21 +06

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
- **Channel order and padding placement are properties of an artifact, not
  operator settings.** Both were decided by measurement. BGR scores at or above
  RGB on every box of the `DI.R8` frame; origin padding beats centred padding by
  2.2 points of recall and 2.6 of precision across the `DI.R10` set, out of the
  same number of boxes. `LetterboxAlignment` drives the `FFmpeg` filter and the
  inverse from one value so the two cannot disagree.
- **A decoder that is missing is distinguished from one that failed to start.**
  Only `ENOENT` and `EACCES` are permanent; everything else, including errors
  the code has not anticipated, is retried. The two mistakes are not equal:
  retrying a permanent fault costs a backed-off reconnect, and giving up on a
  transient one costs a camera its live view silently.
- Thresholds are whole percentages, following the storage watermarks, because
  the settings value model has no float.
- The output decoder is stateless, taking thresholds per call, so classifying
  them as taking effect immediately is true rather than convenient.
- An unset `model_path` is a legitimate state: the fixture runs and the API says
  so. A path that is set and cannot be loaded refuses startup.
- Test harnesses that can pass by doing nothing are treated as defects. `DI.R8`
  requires a minimum detection count, `DI.R10` requires both thresholds as
  inputs, and the publication-recovery test asserts the stop reason rather than
  the state, which would have passed whatever happened.

## Open questions and external gates

- The labeled dataset and the accuracy criteria. Phase 6's remaining item waits
  on these and on nothing technical. The COCO figures are a regression guard; no
  one has agreed that general photographs predict surveillance footage.
- Target hardware and accelerator, open since 2026-08-14. If it turns out to be
  Intel rather than NVIDIA, OpenVINO with a person-only model becomes materially
  cheaper than ONNX with a COCO model, and ADR 015's runtime choice should be
  re-argued rather than inherited.
- Input geometry against small-subject recall. 36.9% at 416x416 is the figure
  that decides whether this detects a person at a useful distance, and the
  evaluation harness can answer it for any geometry.
- Whether detection should default to disabled until accuracy criteria exist.
  `camera.detection.enabled` currently defaults to true, which was harmless when
  every result was visibly a fixture.
- Gate tuning. The threshold, cooldown and heartbeat are defaults nobody has
  measured against real footage.
- Whether `ort`'s panic on an unloadable runtime should be contained by checking
  the library before touching `ort`. It would catch the common deployment error
  and not the rest.
