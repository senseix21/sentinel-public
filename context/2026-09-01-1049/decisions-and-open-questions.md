# Decisions and open questions — 2026-09-01

## Decisions

- Preserve the current direct-camera Rust architecture; implement qualification
  tooling rather than rewriting a media path that is already direct.
- Keep MediaMTX fixtures. Removing them would discard deterministic evidence and
  would not improve the production topology.
- Treat representative-camera admission as an evidence gate per exact model,
  firmware, and stream profile.
- Use a machine-checked, camera-scoped metric gate for direct-ingest smoke runs;
  retain packet-loss measurements for interpretation rather than silently
  declaring every nonzero packet-loss counter equivalent.
- Escalate signals only for fixture-owned FFmpeg processes and prove termination
  before starting the replacement publisher.

## Open external inputs

- Representative vendor/model/firmware and authorized credentials.
- Authority for reboot, network interruption, invalid credential, and rotation
  tests.
- Confirmed target OS/hardware/storage/network and soak duration.
- Viewer concurrency plus approved recovery, recording-gap, live, and percentile
  objectives.
- Required camera profile mix for the 10-camera run and sources for a 20-camera
  measurement.
- Whether H.265, audio, ONVIF, or PTZ will be introduced as separate slices.

## Important mismatch to retain visibly

The accepted 2026 workload names 60-second segments. The current H.264 MVP
constructor defaults to 30 seconds. A qualification run must configure and record
the resolved value; neither value silently replaces the other.
