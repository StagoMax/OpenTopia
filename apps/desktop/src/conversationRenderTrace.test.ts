import assert from "node:assert/strict";
import test from "node:test";
import {
  conversationStreamEventTrace,
  renderedTextChange,
} from "./conversationRenderTrace.ts";
import type { AgentEvent } from "./types.ts";

test("extracts visible model deltas and hidden reasoning deltas", () => {
  const base = {
    id: "event-1",
    threadId: "thread-1",
    turnId: "turn-1",
    seq: 4,
    createdAt: "2026-07-30T10:00:00.000Z",
  };
  const commentary = conversationStreamEventTrace({
    ...base,
    payload: { type: "model_delta", text: "处理中" },
  });
  const reasoning = conversationStreamEventTrace({
    ...base,
    id: "event-2",
    payload: { type: "reasoning_delta", text: "内部推理" },
  });

  assert.equal(commentary?.channel, "commentary");
  assert.equal(commentary?.visible, true);
  assert.equal(reasoning?.channel, "reasoning");
  assert.equal(reasoning?.visible, false);
});

test("ignores events that do not carry streamed text", () => {
  const event: AgentEvent = {
    id: "event-1",
    threadId: "thread-1",
    turnId: "turn-1",
    seq: 1,
    createdAt: "2026-07-30T10:00:00.000Z",
    payload: { type: "turn_started", user_message_id: "message-1" },
  };

  assert.equal(conversationStreamEventTrace(event), null);
});

test("records the exact appended text and falls back to replacement snapshots", () => {
  assert.deepEqual(renderedTextChange("正在", "正在处理"), {
    change: "append",
    text: "处理",
    textLength: 2,
  });
  assert.deepEqual(renderedTextChange("旧内容", "新内容"), {
    change: "replace",
    text: "新内容",
    textLength: 3,
  });
  assert.equal(renderedTextChange("相同", "相同"), null);
});
