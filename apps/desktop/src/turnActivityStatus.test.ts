import assert from "node:assert/strict";
import test from "node:test";
import {
  activeTurnPhase,
  activeTurnStatusLabel,
} from "./turnActivityStatus.ts";
import type { AgentEvent } from "./types.ts";

function event(
  seq: number,
  payload: AgentEvent["payload"],
): AgentEvent {
  return {
    id: `event-${seq}`,
    threadId: "thread-1",
    turnId: "turn-1",
    seq,
    createdAt: "2026-07-30T10:00:00.000Z",
    payload,
  };
}

test("keeps the active label in thinking while only model setup and reasoning arrive", () => {
  const events = [
    event(1, { type: "turn_started", user_message_id: "message-1" }),
    event(2, { type: "reasoning_delta", text: "检查上下文" }),
  ];

  assert.equal(activeTurnPhase(events), "thinking");
  assert.equal(activeTurnStatusLabel(events), "正在思考");
});

test("switches the active label to processing when visible output or work begins", () => {
  assert.equal(
    activeTurnStatusLabel([event(1, { type: "model_delta", text: "开始处理" })]),
    "处理中",
  );
  assert.equal(
    activeTurnPhase([
      event(1, {
        type: "file_changed",
        path: "README.md",
        summary: "updated",
      }),
    ]),
    "processing",
  );
});
