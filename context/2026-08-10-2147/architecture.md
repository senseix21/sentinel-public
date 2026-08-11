# Architecture Context — 2026-08-10 21:47 +06

The LiveKit boundary consumes complete encoded access units with explicit timestamps, not arbitrary Annex-B NAL units. Multiple slices may belong to one frame and must not each receive frame-duration pacing.

Live publication startup is downstream-first: establish publisher readiness before subscribing it to media distribution. Any live packets arriving during publisher unavailability follow the bounded freshness/drop policy and cannot block recorder delivery.

The CLI/socket path remains a spike. Its single-slice mode proved codec and sustained-rate compatibility; it does not replace a measured Rust publisher contract.
