import assert from "node:assert/strict";
import test from "node:test";

import type * as ThreadActivityStatusModule from "./threadActivityStatus";
import type { AgentEvent, TurnStatus } from "./types";

const {
  isThreadActivityProcessing,
  resolveThreadActivityEventStatus,
  resolveThreadActivityStatus,
  threadActivityStatusLabel,
  threadActivityStatusPriority,
}: typeof ThreadActivityStatusModule = await import(
  "./threadActivityStatus" + ".ts"
);

test("keeps task lifecycle states in one shared definition", () => {
  assert.equal(threadActivityStatusLabel("processing"), "处理中");
  assert.equal(threadActivityStatusLabel("succeeded"), "已完成");
  assert.equal(threadActivityStatusLabel("failed"), "运行失败");
  assert.equal(threadActivityStatusLabel("approval"), "等待审批");
  assert.equal(threadActivityStatusLabel("user_action"), "等待手动操作");
  assert.ok(
    threadActivityStatusPriority.approval <
      threadActivityStatusPriority.processing,
  );
  assert.equal(isThreadActivityProcessing("processing"), true);
  assert.equal(isThreadActivityProcessing("succeeded"), false);
});

test("projects backend turn statuses into sidebar activity", () => {
  const turnStatus = (status: TurnStatus["status"]): TurnStatus => ({
    turnId: "turn-1",
    threadId: "thread-1",
    userMessageId: "message-1",
    status,
    startedAt: "2026-08-17T00:00:00Z",
    updatedAt: "2026-08-17T00:00:00Z",
  });

  assert.equal(
    resolveThreadActivityStatus(turnStatus("running")),
    "processing",
  );
  assert.equal(
    resolveThreadActivityStatus(turnStatus("waiting_approval")),
    "approval",
  );
  assert.equal(
    resolveThreadActivityStatus(turnStatus("waiting_user_input")),
    "user_action",
  );
  assert.equal(
    resolveThreadActivityStatus(turnStatus("succeeded")),
    "succeeded",
  );
  assert.equal(resolveThreadActivityStatus(turnStatus("failed")), "failed");
  assert.equal(resolveThreadActivityStatus(turnStatus("cancelled")), null);
});

test("projects background lifecycle events without depending on active navigation", () => {
  const event = (payload: AgentEvent["payload"], turnId = "turn-1") =>
    ({
      id: crypto.randomUUID(),
      threadId: "thread-1",
      turnId,
      seq: 1,
      createdAt: "2026-08-23T00:00:00Z",
      payload,
    }) satisfies AgentEvent;

  assert.equal(
    resolveThreadActivityEventStatus(
      event({ type: "turn_started", user_message_id: "message-1" }),
    ),
    "processing",
  );
  assert.equal(
    resolveThreadActivityEventStatus(
      event({ type: "turn_finished", summary: "done" }),
    ),
    "succeeded",
  );
  assert.equal(
    resolveThreadActivityEventStatus(
      event({ type: "turn_cancelled", reason: "cancelled" }),
    ),
    null,
  );
  assert.equal(
    resolveThreadActivityEventStatus(
      event({ type: "model_delta", text: "working" }),
    ),
    undefined,
  );
});
