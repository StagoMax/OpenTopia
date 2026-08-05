import assert from "node:assert/strict";
import test from "node:test";

import type * as ConversationPlanModule from "./conversationPlan";
import type { AgentEvent, TaskPlan } from "./types";

const { resolveRuntimeTaskPlan } = (await import(
  "./conversationPlan" + ".ts"
)) as typeof ConversationPlanModule;

const plan: TaskPlan = {
  planRevision: 1,
  goalId: "runtime-plan",
  steps: [
    {
      id: "edit",
      title: "Edit",
      status: "in_progress",
      dependencies: [],
      acceptanceCriteria: [],
      evidence: [],
    },
  ],
};

function event(
  seq: number,
  payload: AgentEvent["payload"],
  turnId = "turn-1",
): AgentEvent {
  return {
    id: `event-${seq}`,
    seq,
    threadId: "thread-1",
    turnId,
    createdAt: "2026-08-04T00:00:00Z",
    payload,
  };
}

test("keeps the current runtime plan while its turn is active", () => {
  assert.equal(
    resolveRuntimeTaskPlan([
      event(1, { type: "turn_started", user_message_id: "message-1" }),
      event(2, { type: "plan_updated", plan }),
    ]),
    plan,
  );
});

test("clears pending runtime steps after their turn is cancelled", () => {
  assert.equal(
    resolveRuntimeTaskPlan([
      event(1, { type: "plan_updated", plan }),
      event(2, { type: "turn_cancelled", reason: "Cancelled by user." }),
    ]),
    null,
  );
});

test("does not let an older terminal event hide a new turn plan", () => {
  assert.equal(
    resolveRuntimeTaskPlan([
      event(1, { type: "turn_cancelled", reason: "Cancelled" }, "turn-1"),
      event(2, { type: "plan_updated", plan }, "turn-2"),
    ]),
    plan,
  );
});
