import assert from "node:assert/strict";
import test from "node:test";

import type * as ThreadActivityStatusModule from "./threadActivityStatus";

const {
  isThreadActivityProcessing,
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
