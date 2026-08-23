import assert from "node:assert/strict";
import test from "node:test";
import {
  appConversationStateEqual,
  selectAppConversationState,
} from "./appConversationState.ts";
import { createConversationSessionState } from "./conversationSession.ts";
import type { AgentEvent } from "./types";

function event(seq: number, payload: AgentEvent["payload"]): AgentEvent {
  return {
    id: `event-${seq}`,
    threadId: "thread-1",
    turnId: "turn-1",
    seq,
    createdAt: `2026-08-23T00:00:${String(seq).padStart(2, "0")}.000Z`,
    payload,
  };
}

test("ignores tool-only event history changes in the app shell selection", () => {
  const state = createConversationSessionState("thread-1");
  const before = selectAppConversationState(state);
  const after = selectAppConversationState({
    ...state,
    events: [
      event(1, {
        type: "tool_call_started",
        call: { id: "call-1", name: "shell", input: { command: "pwd" } },
      }),
    ],
  });

  assert.equal(appConversationStateEqual(before, after), true);
});

test("updates the app shell selection when an approval becomes pending", () => {
  const state = createConversationSessionState("thread-1");
  const before = selectAppConversationState(state);
  const approval = event(1, {
    type: "approval_requested",
    approval_id: "approval-1",
    action: "run command",
    reason: "requires approval",
  });
  const after = selectAppConversationState({
    ...state,
    events: [approval],
    pendingApprovalIds: ["approval-1"],
  });

  assert.equal(appConversationStateEqual(before, after), false);
  assert.deepEqual(after.pendingApprovalQueue, [approval]);
});
