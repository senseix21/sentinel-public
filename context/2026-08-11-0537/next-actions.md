# Next Actions — 2026-08-11 05:37 +06

1. Integrate `RetinaH264Session` into the state-specific camera supervisor so
   one task owns connection, cancellation, failure classification, and epochs.
2. Add bounded exponential reconnect backoff with deterministic jitter injection
   and prove publisher stop/start creates a fresh session without affecting
   unrelated owners.
3. Publish received access units into the existing bounded media distributor,
   with configured capacity and observable rejection/discontinuity counters.
4. Implement the real fragmented-MP4 sink and storage finalization boundary,
   then prove three consecutive playable segments with truthful metadata.
5. Qualify representative 4 MP H.264 cameras before changing adapter status to
   production-supported.
