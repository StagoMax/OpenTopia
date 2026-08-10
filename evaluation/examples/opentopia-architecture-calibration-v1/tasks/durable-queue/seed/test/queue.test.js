import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { DurableQueue } from "../src/queue.js";

test("enqueues and leases deterministically", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "queue-public-")); let now = new Date("2026-01-01T00:00:00Z");
  const queue = new DurableQueue(path.join(root, "state.json"), () => now);
  queue.enqueue({ id: "b", payload: 2, availableAt: now }); queue.enqueue({ id: "a", payload: 1, availableAt: now });
  assert.equal(queue.lease("worker", 1000).id, "a"); assert.equal(queue.snapshot().length, 2);
});
