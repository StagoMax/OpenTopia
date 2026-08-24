import assert from "node:assert/strict";
import test from "node:test";

import { projectConversationEvents } from "./conversationEventProjection.ts";
import type { AgentEvent } from "./types";

function event(
  id: string,
  seq: number,
  turnId: string,
  payload: AgentEvent["payload"],
): AgentEvent {
  return {
    id,
    seq,
    threadId: "thread-1",
    turnId,
    payload,
    createdAt: `2026-08-24T00:00:${String(seq).padStart(2, "0")}Z`,
  };
}

test("keeps completed Turn event arrays stable when another Turn changes", () => {
  const firstTurn = [
    event("turn-1-start", 1, "turn-1", {
      type: "turn_started",
      user_message_id: "user-1",
    }),
    event("turn-1-finish", 2, "turn-1", {
      type: "turn_finished",
      summary: "done",
    }),
  ];
  const initial = projectConversationEvents(firstTurn);
  const updated = projectConversationEvents(
    [
      ...firstTurn,
      event("turn-2-start", 3, "turn-2", {
        type: "turn_started",
        user_message_id: "user-2",
      }),
    ],
    initial,
  );

  assert.equal(
    updated.eventsByTurn.get("turn-1"),
    initial.eventsByTurn.get("turn-1"),
  );
  assert.notEqual(
    updated.eventsByTurn.get("turn-2"),
    initial.eventsByTurn.get("turn-2"),
  );
});

test("replaces only the Turn whose compacted model delta changed", () => {
  const turn2Start = event("turn-2-start", 2, "turn-2", {
    type: "turn_started",
    user_message_id: "user-2",
  });
  const first = projectConversationEvents([
    event("delta-1", 1, "turn-1", { type: "model_delta", text: "a" }),
    turn2Start,
  ]);
  const updated = projectConversationEvents(
    [
      event("delta-2", 3, "turn-1", { type: "model_delta", text: "ab" }),
      turn2Start,
    ],
    first,
  );

  assert.notEqual(
    updated.eventsByTurn.get("turn-1"),
    first.eventsByTurn.get("turn-1"),
  );
  assert.equal(
    updated.eventsByTurn.get("turn-2"),
    first.eventsByTurn.get("turn-2"),
  );
});
