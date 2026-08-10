# Compressed incident-log summary

Implement `summarizeLog(readable, options)` in `src/logs.js`. The input is an
NDJSON byte/text stream. Each row is exactly
`{ timestamp, service, level, requestId, durationMs }`.

Validate ISO timestamps, non-empty service/request IDs, level
`info|warn|error`, and non-negative integer duration. Duplicate request IDs are
invalid. With optional `{ since, until }`, include timestamps in the inclusive
range. Return:

```json
{
  "events": 0,
  "services": [{ "service": "api", "events": 0, "errors": 0, "p95DurationMs": 0 }],
  "firstTimestamp": null,
  "lastTimestamp": null
}
```

Use nearest-rank p95 (`ceil(0.95*n)`, one-based). Services sort by name.
The CLI reads `.jsonl` or `.jsonl.gz` using streams:

`node src/cli.js --input <path> --output <json> [--since <iso>] [--until <iso>]`

Write pretty JSON plus a trailing newline. Do not use synchronous whole-file
reads or add dependencies.
