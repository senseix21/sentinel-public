# Bounded Media Distribution Contract

Date: 2026-08-10

## Scope

This contract defines the first in-process encoded-media distribution boundary.
It is deliberately nonblocking and independent of an async runtime. Async
wake-ups, cancellation, and task supervision belong to the ingestion-to-record
vertical slice and must preserve these overload semantics.

## Ownership

- `MediaDistributor<T>` is the single publication owner.
- The recorder and live-view consumers each own one independent subscription.
- Published media is immutable and shared through `Arc<T>`; distribution does
  not copy the media payload.
- Dropping a subscription closes and drains only that consumer queue.
- Dropping the distributor closes both producer sides; consumers may drain
  retained media before observing closure.

## Bounds and overload

Every queue has a validated nonzero item capacity. Byte bounds remain open
until representative access-unit sizes are measured; an item bound alone must
not be presented as a proven production memory budget.

| Consumer | Full-queue action | Observable outcome | Counter |
|---|---|---|---|
| Recorder | Preserve queued media; reject the new item | `Full` | `rejected_full` |
| Live view | Evict the oldest item; accept the new item | `ReplacedOldest` | `dropped_oldest` |

Publication attempts both queues independently and never waits. Consequently,
a stalled or closed live consumer cannot consume recorder capacity. Recorder
`Full` is a critical degradation signal for the future supervisor; this layer
does not silently retry or choose recording-loss policy.

## Metrics contract

Each queue exposes capacity, current depth, accepted and received counts,
policy-specific rejection/drop counts, close-time drops, and producer/consumer
open state. These in-memory counters are the source contract for later runtime
metrics; metric names and exporter technology are not frozen here.

Counters saturate instead of wrapping. Mutex poisoning is recovered because a
prior observer panic must not turn queue inspection into a second panic.

## Evidence

Deterministic unit tests prove:

- zero capacity is rejected;
- a stalled live consumer retains only recent media without preventing recorder
  delivery;
- recorder overflow preserves the existing backlog and reports rejection;
- one consumer can close without closing the other;
- producer closure permits backlog draining before `Closed`; and
- consumer closure releases retained media and reports the discarded count.

The recorder crate now consumes this subscription through a narrow H.264 sink
boundary. If the sink rejects a dequeued access unit, the consumer returns that
same `Arc<H264AccessUnit>` and the typed sink error to its supervisor. This does
not invent automatic retry semantics, but it prevents the failure from becoming
an unobservable ownership loss.

An integration test now exercises the whole SDK-neutral seam from RTP/H.264
payloads through camera depayloading and access-unit assembly into distribution
and the recorder sink. It proves ordered recorder delivery and isolated live
freshness drops. It does not claim an RTSP session, async task, or real muxer.

## Deferred decisions

- Async notification and cancellation-safe waiting.
- Production capacities and byte-budget enforcement.
- Recorder escalation, pause, reconnect, or terminal-failure policy.
- Detector sampling/drop semantics, which operate on decoded frames.
- Multiple subscribers of one consumer class.
