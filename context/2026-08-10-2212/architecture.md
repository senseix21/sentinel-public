# Architecture Context — 2026-08-10 22:12 +06

The media distributor has one producer and independent single-owner recorder
and live subscriptions. Items are immutable `Arc<T>` values, so publication
shares ownership without copying payload bytes.

Recorder overflow preserves queued media and rejects the new item with `Full`.
Live overflow evicts only its oldest item and accepts the new item with
`ReplacedOldest`. Publication attempts both queues without waiting, which makes
live-view lag unable to consume recorder capacity.

Queue capacities are nonzero item bounds. They are not yet production memory
bounds because representative maximum access-unit sizes have not been measured.
