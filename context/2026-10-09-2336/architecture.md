# Architecture

Single ingest and detection decoder, canonical 640 RGB frame with Arc ownership. Separate bounded person and face lanes and global per-lane admission. Monotonic bounded evidence tracker exposes observed, coasting and tentative counts. Face events independently drive boxes; presence never draws bodies. Presence transport exposes counts, not track identities. ADR 017 is accepted for reversible development.
