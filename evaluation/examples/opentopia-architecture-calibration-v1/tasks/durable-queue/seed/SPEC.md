# Leased durable queue

Implement `DurableQueue` in `src/queue.js`. Its constructor takes a JSON state
path and a clock function. State writes must be atomic and recover after a
process restart.

- `enqueue({ id, payload, availableAt })` rejects duplicate IDs.
- `lease(workerId, leaseMs)` first returns expired leases to pending, then
  leases the available pending job ordered by `availableAt`, then ID.
- A leased job records owner, lease expiry, and increments `attempts`.
- `ack(workerId, id)` completes only that worker's active lease.
- `retry(workerId, id, delayMs)` returns that lease to pending at now+delay.
- Invalid ownership/state transitions throw without changing persisted state.
- `snapshot()` returns jobs sorted by ID without exposing mutable internals.

The CLI consumes `scenario.json` actions using their explicit timestamps,
writes `queue-state.json`, and writes `scenario-result.json` containing the
return value of every action. Timestamps are ISO strings in serialized state.
