# Next Actions — 2026-10-05 00:00 +06

1. Make the detection thresholds genuinely live. `OnnxPersonInference`
   captures `DecodeThresholds` at construction and reuses it in every
   call to `infer`. Threading a shared config source (an `AtomicU8`
   per field or a reload channel) through the inference path lets the
   classification flip from `PipelineRestart` to `Immediate`
   truthfully. Same shape fits `GatePolicy`; both deserve the same
   slice. The reasoning is already comment-cited at the setting
   definitions in `crates/config/src/classification.rs` so the next
   person sees the trade-off.
2. Replace the env-var ROI mask with a per-camera config field and a
   UI to draw the rectangle. The env var proved the mechanic; it is
   not an operator surface. The config side already exists on
   `OneCameraConfig::detection_roi_exclude`; the gap is the HTTP route
   and a canvas drag on the live tile.
3. Measure what input geometry does to small-subject recall. Still 36.9%
   at 416×416 against 95.3% for large subjects. YOLOX-S at 640×640 is
   now a flipped env var away; the eval set generator takes the input
   geometry as an argument.
4. Decide whether `camera.detection.enabled` should default to false
   until an accuracy criterion exists. A deployment can now configure
   a real model, which changes what enabled-by-default means.
5. Containerise `livekit-server` and install `promtool`. Four Phase 5
   recovery scenarios and all thirteen Phase 8 alert rules are open on
   the same two external pieces.
6. Add the accuracy dataset and criteria that represent this product's
   cameras. COCO photographs are a regression guard and nothing in
   this session changed that.
7. Add an animals class filter. The person-only cut in the overlay is
   right for v1; a deployment that watches a yard will want deer and
   dogs rendered differently from humans. YOLOX returns the class
   already; the detection event schema does not surface it.
8. Phase 0 answers that gate the rest: target hardware (Intel reopens
   ADR 015), deployment model, viewer concurrency, p90 targets.
