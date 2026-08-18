import assert from "node:assert/strict";
import test from "node:test";

import type * as ThreadActivityStatusModule from "./threadActivityStatus";
import type { TurnStatus } from "./types";

const {
  isThreadActivityProcessing,
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
