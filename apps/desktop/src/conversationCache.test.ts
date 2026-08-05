import assert from "node:assert/strict";
import test from "node:test";

import type * as ConversationCacheModule from "./conversationCache";
import type { AgentEvent, Message } from "./types";

const {
  cacheConversation,
  mergeConversationEvents,
  mergeConversationMessages,
} = (await import("./conversationCache" +
  ".ts")) as typeof ConversationCacheModule;

function event(id: string, seq: number): AgentEvent {
  return {
    id,
    seq,
    threadId: "thread-1",
    createdAt: "2026-08-03T00:00:00Z",
    payload: { type: "turn_finished", summary: id },
  };
}

function modelDelta(id: string, seq: number, text: string): AgentEvent {
  return {
    id,
    seq,
    threadId: "thread-1",
    turnId: "turn-1",
    createdAt: "2026-08-03T00:00:00Z",
    payload: { type: "model_delta", text },
  };
}

function message(id: string, createdAt: string): Message {
  return {
    id,
    threadId: "thread-1",
    role: "user",
    parts: [{ type: "text", text: id }],
    createdAt,
  };
}

test("merges messages without losing a streamed response", () => {
  const assistant = message("assistant", "2026-08-03T00:00:02Z");
  assistant.role = "assistant";
  const merged = mergeConversationMessages(
    [assistant],
    [message("user", "2026-08-03T00:00:01Z")],
  );

  assert.deepEqual(
    merged.map((item) => item.id),
    ["user", "assistant"],
  );
});

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

test("compacts adjacent model deltas without crossing activity boundaries", () => {
  const merged = mergeConversationEvents([], [
    modelDelta("delta-1", 1, "hello "),
    modelDelta("delta-2", 2, "world"),
    event("boundary", 3),
    modelDelta("delta-3", 4, "after"),
  ]);

  assert.equal(merged.length, 3);
  assert.equal(merged[0]?.id, "delta-2");
  assert.deepEqual(merged[0]?.payload, {
    type: "model_delta",
    text: "hello world",
  });
  assert.equal(merged[1]?.id, "boundary");
  assert.equal(merged[2]?.id, "delta-3");
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
