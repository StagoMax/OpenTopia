# Worker pool starts work after cancellation

`runPool(items, worker, options)` should process at most `concurrency` items at
once and return results in input order. In production, aborting a request still
starts queued jobs, and synchronous worker exceptions sometimes leave the
returned promise pending.

Required behavior:

- `concurrency` is a positive integer.
- Results preserve input order even when jobs finish out of order.
- Both synchronous throws and rejected promises reject the pool promptly.
- Once one job fails or the AbortSignal aborts, no additional queued job starts.
- Abort rejects with an error whose `name` is `AbortError`.
- An already-aborted signal starts no jobs.

Do not add dependencies.
