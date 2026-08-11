# Next Actions — 2026-08-11 11:11 +06

1. Extend the owned MediaMTX fixture to stop/restart its publisher while the
   Rust supervisor remains alive; prove reconnect, a fresh epoch, resumed access
   units, and cancellation during backoff.
2. Resolve offline-path status classification from evidence: MediaMTX can return
   404 while a configured publisher is temporarily absent, which must not be
   confused silently with a permanent bad camera path.
3. Publish lifecycle snapshots through a bounded-cardinality health interface.
4. Connect the supervisor and existing recorder task under an application owner
   with explicit join/error policy.
5. Implement the fragmented-MP4 sink and prove three consecutive playable,
   truthfully indexed segments.
