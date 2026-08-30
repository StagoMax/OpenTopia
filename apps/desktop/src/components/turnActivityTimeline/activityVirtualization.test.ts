import assert from "node:assert/strict";
import test from "node:test";
import {
  activityVirtualChunkSize,
  buildActivityVirtualChunks,
  estimateActivityEntryHeight,
} from "./activityVirtualization.ts";
import type { ActivityEntry } from "./model";

function commentary(seq: number, text = "step"): ActivityEntry {
  return {
    kind: "commentary",
    seq,
    text,
    createdAt: new Date(seq * 1_000).toISOString(),
  };
}

test("splits long timelines into bounded stable chunks", () => {
  const entries = Array.from({ length: 65 }, (_, index) => commentary(index));
  const chunks = buildActivityVirtualChunks(entries);
  const appended = buildActivityVirtualChunks([
    ...entries,
    commentary(entries.length),
  ]);

  assert.equal(chunks.length, 9);
  assert.ok(chunks.every((chunk) => chunk.entries.length <= activityVirtualChunkSize));
  assert.deepEqual(
    chunks.slice(0, -1).map((chunk) => chunk.key),
    appended.slice(0, -1).map((chunk) => chunk.key),
  );
  assert.ok(chunks.every((chunk) => chunk.estimatedHeight > 0));
});

test("estimates narrative placeholders without allowing unbounded height", () => {
  assert.ok(estimateActivityEntryHeight(commentary(1, "short")) >= 44);
  assert.equal(
    estimateActivityEntryHeight(commentary(2, "long".repeat(20_000))),
    320,
  );
});
