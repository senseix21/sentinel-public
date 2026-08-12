# Architecture — Phase 5 checkpoint

The accepted one-ingest/many-consumer topology is preserved. `CameraPipeline`
creates one `MediaDistributor`; the mandatory recorder owns its preservation
queue and an optional `LivePublicationOwner` owns the freshness queue. Publisher
failure or termination does not stop recording.

`surveillance-livekit` owns validated identities/configuration, parameter-set and
RTP adaptation contracts, child adapter lifecycle, bounded retry/cancellation,
credential-free health, and viewer token issuance. The selected adapter owns one
`lk` 2.18.2 child and short Unix socket per enabled camera and copies each access
unit into one bounded Annex-B buffer. Secrets enter the child environment and do
not appear in arguments, API health, metrics, or `Debug`.

The CLI generates its own RTP timestamps rather than consuming the tested Rust
90 kHz mapping. ADR 007 therefore treats it as reversible and not yet production
qualified.
