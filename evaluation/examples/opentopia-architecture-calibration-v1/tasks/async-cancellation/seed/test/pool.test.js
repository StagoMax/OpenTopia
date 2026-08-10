import assert from "node:assert/strict";
import test from "node:test";
import { runPool } from "../src/pool.js";

test("preserves input ordering", async () => {
  const result = await runPool([30, 5, 10], async (delay, index) => {
    await new Promise((resolve) => setTimeout(resolve, delay));
    return index;
  }, { concurrency: 2 });
  assert.deepEqual(result, [0, 1, 2]);
});

test("validates concurrency", async () => {
  await assert.rejects(runPool([], async () => null, { concurrency: 0 }), /concurrency/i);
});
