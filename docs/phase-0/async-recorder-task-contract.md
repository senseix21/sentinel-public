# Async Recorder Task Contract

Date: 2026-08-10

## Decision

Use Tokio as the application async runtime and retain the existing bounded
media queues as the source of truth. A `watch` generation counter adds wake-ups;
it does not carry media or alter queue capacity/overflow rules.

Tokio is pinned at the workspace boundary. The selected primitives follow its
documented contracts:

- `watch::Receiver::changed` retains unseen changes and is cancellation-safe.
- Graceful shutdown separates requesting termination from awaiting completion.
- Dropping a Tokio `JoinHandle` detaches its task, so the recorder wraps the
  handle in an owner whose `Drop` aborts instead.

Primary references:

- <https://docs.rs/tokio/latest/tokio/sync/watch/>
- <https://docs.rs/tokio/latest/tokio/macro.select.html#cancellation-safety>
- <https://docs.rs/tokio/latest/tokio/task/struct.JoinHandle.html>
- <https://tokio.rs/tokio/topics/shutdown>

## Ownership and exits

`RecorderTask<S>` owns the only task join handle and cancellation sender. The
spawned task owns `RecorderConsumer<S>`, which in turn owns its subscription and
sink. Every normal terminal result returns the consumer:

- `InputDrained`: producer closed and retained media was consumed.
- `Cancelled`: cooperative cancellation won; queue/sink ownership is returned
  without claiming drain.
- `SinkFailed`: rejected access unit, typed error, queue, and sink are all
  returned to the supervisor.

For orderly application shutdown, ingestion/distribution closes first and the
recorder task is joined. `cancel_and_join` is a non-draining controlled stop.
Dropping `RecorderTask` is an exceptional path that aborts rather than detaches;
it cannot provide finalization guarantees.

## Wake-up correctness

The subscriber checks its bounded queue before awaiting `watch::changed`. If a
publication occurs between those operations, the unseen generation makes the
wait immediately ready. Producer closure updates queue state and the generation,
allowing an empty waiter to observe `Closed`.

Media never enters the `watch` channel. Coalescing notifications is safe because
the queue retains the authoritative items and each wake drains one item at a
time until empty.

## Deferred work

- The real sink's segment flush/finalization on `InputDrained`.
- Supervisor restart/escalation policy after `SinkFailed`.
- Shutdown deadlines and forced-abort metrics at the application boundary.
- Tokio multi-thread runtime configuration and blocking RTSP/FFI isolation.
