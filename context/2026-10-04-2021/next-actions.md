# Next Actions — 2026-10-04 20:21 +06

1. Stand up a LiveKit server in the development container. It closes the one
   joint the publication-recovery test leaves faked — `connect` and `disconnect`,
   because a `Session` holds a connected `Room` — and four Phase 5 items that
   have been open for months: real server stop/restart while recording,
   duplicate-track recovery after restart, startup-outage isolation, and
   deterministic kill/restart. `make run` already requires a local
   `livekit-server` and `scripts/phase5/outage-fixture.sh` already exists, so
   this is containerising something that works rather than building it.
2. Install `promtool` in the development image. Phase 8 ships thirteen alert
   rules and a firing and silent case for each critical one, and none has ever
   been executed. An alert that never fires looks exactly like a healthy system.
3. Measure what input geometry does to small-subject recall. It is 36.9% at
   416x416 against 95.3% for large subjects, and for a camera watching a
   doorway at twenty metres that figure is the product.
   `scripts/detection/build-eval-set.py` takes the geometry as an argument, so
   this is one regeneration and one run.
4. Decide whether `camera.detection.enabled` should default to false until
   accuracy criteria exist. A deployment can now configure a real model, which
   changes what an enabled-by-default detector means.
5. Measure the gate's duty cycle on real footage and tune the threshold,
   cooldown and heartbeat from it. Watch the heartbeat counter against admitted
   frames: a ratio near one means the threshold is doing no work.
6. Obtain the Phase 0 answers that gate everything downstream, starting with
   target hardware. An Intel target would reopen ADR 015's runtime choice.
7. Add the accuracy dataset and criteria. It is the only route to closing Phase
   6's last item and nothing in the code substitutes for it.
8. Consider a pre-flight check of the ONNX Runtime library before `ort` touches
   it, so the common deployment error becomes a refusal rather than a panic.
