# Next actions — 2026-08-18 17:17 +06

## 1. Commit and recheck this measurement slice

Run `bash -n` for the changed fixture, the relevant metric-name check, and the
full repository gate. Commit the harness correction, evidence, roadmap update,
and this snapshot only after they pass.

## 2. Measure viewer-visible reconnect behavior and first picture

Server-side recovery with the same subscriber is proven. Still unknown: time
from initial subscribe to first rendered frame, freeze duration across reconnect,
and time to first rendered post-reconnect frame. Extend the authenticated browser
fixture or use a headed browser harness; server counters cannot answer this.

## 3. Confirm dashboard playback rendering

The API stream is proven decodable. The prior headless Playwright playback run
timed out. Repeat with a headed browser and a longer explicit budget.

## 4. Qualify representative cameras

All current measurements use a synthetic Baseline stream or the built-in webcam.
High Profile, multi-slice, H.265 policy, long GOP behavior, and non-Chromium
browsers require representative hardware or supplied samples.

## 5. Exercise alerts and LiveKit outage recovery

Install/use `promtool` to execute `alerts.yml` and its tests. Choose the product
window for timestamp-discontinuity rate rather than alerting on a correct single
reconnect. Deterministically stop/restart LiveKit while recording and verify
publisher recovery and absence of duplicate tracks.

## 6. Resolve recording evidence one-way doors

Product decision remains required for the audited path out of `Inconsistent`.
Independently add a metric for immutable-evidence rejection during reconnect so
playable but unindexed footage is not silent.

## 7. Begin Phase 9 only against the defined workload

The viewer fan-out curve is useful evidence but is not the Phase 9 one-camera
baseline: detection was disabled and the deployment hardware is unconfirmed.
Do not convert it into a supported-camera claim.
