import assert from "node:assert/strict";
import { Readable } from "node:stream";
import test from "node:test";
import { summarizeLog } from "../src/logs.js";

test("summarizes an NDJSON stream", async () => {
  const rows = [
    { timestamp: "2026-01-01T00:00:00Z", service: "api", level: "info", requestId: "a", durationMs: 10 },
    { timestamp: "2026-01-01T00:01:00Z", service: "api", level: "error", requestId: "b", durationMs: 30 },
  ];
  const result = await summarizeLog(Readable.from(rows.map((row) => `${JSON.stringify(row)}\n`)));
  assert.deepEqual(result.services, [{ service: "api", events: 2, errors: 1, p95DurationMs: 30 }]);
});
