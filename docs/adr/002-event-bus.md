# ADR 002: Domain Event Bus

Status: accepted for the Phase 6 in-process slice (2026-08-12)

## Context

Camera, recording, detection, storage, analytics, API, and rules need loose coupling for lightweight domain facts. The first deployment is one modular process; external messaging would introduce delivery and operational complexity without a demonstrated need.

## Decision

- Begin with a bounded in-process event bus.
- Events are immutable, past-tense, camera/time-scoped facts with explicit schema versions when persisted or externally exposed.
- Commands and configuration updates are not events.
- Packets, frames, clips, and other media payloads never travel on this bus.
- Producers do not embed alert, dashboard, webhook, or analytics policy.
- Subscription lifetimes participate in structured application shutdown.

Each subscription owns an independent queue with a validated nonzero item
capacity. The current event values are small and capped (including at most 64
person boxes); a measured byte budget is a reversal criterion if schemas grow.
Publish never awaits a subscriber and accepted events retain publication order
within a producer. There is no global order, durable replay, acknowledgement,
or exactly-once claim.

Delivery classes are:

- `CriticalLifecycle`: preserve the existing backlog and visibly reject new
  admission when full. The supervised producer/adapter records the failure;
  durable recording/storage facts remain reconcilable from their owning index.
- `LatestState`: replace an older queued state with the same bounded
  `(camera, state-kind)` key; if no matching key exists and the queue is full,
  evict the oldest state.
- `Telemetry`: discard the new event when full and increment a drop counter.
- `Detection`: evict the oldest queued detection when full so consumers see
  recent observations. Ordering holds only among retained events per producer.

One slow/dead subscriber cannot occupy another subscriber's capacity. Publishing
with no subscribers succeeds with a visible zero-delivery report. Dropping a
subscription releases its retained events and unregisters it. Closing the bus
wakes subscribers, which drain accepted backlog before observing closure.

## Persistence boundary

Phase 6 delivery is at-most-once within one process lifetime. Persistence and
replay belong to explicit adapters. Recording and storage lifecycle producers
may reconstruct durable facts from their existing indexed state after critical
admission failure; camera/detector state is republished from current owned state.
Detection observations are transient and may be dropped under overload.

## Alternatives

- Direct producer-to-consumer calls: rejected because it embeds downstream policy and failure into producers.
- Unbounded broadcast channel: rejected because memory and lag are uncontrolled.
- Kafka/NATS at 10–20 cameras: deferred because broker delivery semantics and operations are not free substitutions.

## Reversal criteria

Introduce an external adapter only when components are extracted across processes/nodes or a confirmed durable integration requires it. The migration must define delivery, ordering, duplication, replay, and partition behavior explicitly.
