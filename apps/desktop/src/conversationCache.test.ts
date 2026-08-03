import assert from "node:assert/strict";
import test from "node:test";

import type * as ConversationCacheModule from "./conversationCache";
import type { AgentEvent } from "./types";

const { cacheConversation, mergeConversationEvents } = (await import(
  "./conversationCache" + ".ts"
)) as typeof ConversationCacheModule;

function event(id: string, seq: number): AgentEvent {
  return {
    id,
    seq,
    threadId: "thread-1",
    createdAt: "2026-08-03T00:00:00Z",
    payload: { type: "turn_finished", summary: id },
  };
}

test("merges incremental conversation events by id and sequence", () => {
  const current = [event("one", 1), event("three", 3)];
  const replacement = event("three", 3);
  const merged = mergeConversationEvents(current, [
    event("two", 2),
    replacement,
  ]);

  assert.deepEqual(
    merged.map((item) => item.id),
    ["one", "two", "three"],
  );
  assert.equal(merged[2], replacement);
});

test("conversation cache uses least-recently-used insertion order", () => {
  const cache = new Map();
  cacheConversation(cache, "one", { messages: [], events: [] }, 2);
  cacheConversation(cache, "two", { messages: [], events: [] }, 2);
  cacheConversation(
    cache,
    "one",
    { messages: [], events: [event("one", 1)] },
    2,
  );
  cacheConversation(cache, "three", { messages: [], events: [] }, 2);

  assert.deepEqual([...cache.keys()], ["one", "three"]);
});
