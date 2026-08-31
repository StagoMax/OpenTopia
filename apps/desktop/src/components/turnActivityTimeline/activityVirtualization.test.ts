import assert from "node:assert/strict";
import test from "node:test";
import {
  activityVirtualChunkSize,
  buildActivityVirtualChunks,
  estimateActivityEntryHeight,
  shouldKeepActivityVirtualChunkMounted,
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

test("keeps a bounded activity tail mounted for bottom-pinned jumps", () => {
  assert.equal(shouldKeepActivityVirtualChunkMounted(3, 7), false);
  assert.equal(shouldKeepActivityVirtualChunkMounted(4, 7), true);
  assert.equal(shouldKeepActivityVirtualChunkMounted(5, 7), true);
  assert.equal(shouldKeepActivityVirtualChunkMounted(6, 7), true);
});
