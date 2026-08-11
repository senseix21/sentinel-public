# ADR 002: Domain Event Bus

Status: proposed

## Context

Camera, recording, detection, storage, analytics, API, and rules need loose coupling for lightweight domain facts. The first deployment is one modular process; external messaging would introduce delivery and operational complexity without a demonstrated need.

## Proposed decision

- Begin with a bounded in-process event bus.
- Events are immutable, past-tense, camera/time-scoped facts with explicit schema versions when persisted or externally exposed.
- Commands and configuration updates are not events.
- Packets, frames, clips, and other media payloads never travel on this bus.
- Producers do not embed alert, dashboard, webhook, or analytics policy.
- Subscription lifetimes participate in structured application shutdown.

## Semantics to resolve before implementation

- Ordering scope: provisionally per producer/camera, never unqualified global order.
- Delivery: lifecycle events may require a durable outbox/replay path; transient UI updates may tolerate loss.
- Overflow: must be explicit per event class; no silent universal drop policy.
- Slow/dead subscribers: cannot block unrelated publishers indefinitely.
- Persistence and replay: belongs at an adapter boundary, not inside every producer.

## Alternatives

- Direct producer-to-consumer calls: rejected because it embeds downstream policy and failure into producers.
- Unbounded broadcast channel: rejected because memory and lag are uncontrolled.
- Kafka/NATS at 10–20 cameras: deferred because broker delivery semantics and operations are not free substitutions.

## Reversal criteria

Introduce an external adapter only when components are extracted across processes/nodes or a confirmed durable integration requires it. The migration must define delivery, ordering, duplication, replay, and partition behavior explicitly.

