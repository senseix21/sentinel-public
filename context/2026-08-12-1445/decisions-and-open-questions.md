# Decisions and open questions — Phase 6

Accepted locally:

- Continuous H.264 decode followed by media-timestamp sampling; arbitrary sparse
  inter-frame decode is not assumed safe.
- FFmpeg process adapter for the first reversible real pixel boundary.
- Deterministic fake-person inference only for systems qualification.
- ADR 002 delivery classes and at-most-once in-process semantics.
- Restart-applied transactional detection configuration with a 5 FPS default.

External gates:

- Production model/version, dataset, precision/recall, and confidence policy.
- Target CPU/GPU/accelerator/driver/Linux and representative 4 MP cameras.
- Accepted freshness p90, tracking outcome, and 10-camera combined soak.
