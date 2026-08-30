import assert from "node:assert/strict";
import test from "node:test";

import type { HumanTask } from "./types";
import type * as HumanTasksModule from "./humanTasks";

const {
  humanTaskActionPresentation,
  humanTaskInputRequest,
  humanTaskStatusLabel,
  humanTaskTypeLabel,
  orderedHumanTaskActions,
  reconcileHumanTaskSelection,
  sortPendingHumanTasks,
} = (await import("./humanTasks" + ".ts")) as typeof HumanTasksModule;

function task(id: string, overrides: Partial<HumanTask> = {}): HumanTask {
  return {
    schemaVersion: 1,
    id,
    revision: 1,
    threadId: "thread-1",
    sourceKind: "flow_run",
    sourceId: "run-1",
    taskType: "approval",
    status: "pending",
    title: `Task ${id}`,
    description: "Needs review",
    allowedActions: ["approve", "reject"],
    payload: {},
    createdAt: "2026-08-17T00:00:00Z",
    updatedAt: "2026-08-17T00:00:00Z",
    ...overrides,
  };
}

test("presents task types, statuses, and actions consistently", () => {
  assert.equal(humanTaskTypeLabel("approval"), "等待审批");
  assert.equal(humanTaskTypeLabel("reconnect"), "重新连接");
  assert.equal(humanTaskTypeLabel("reconciliation"), "副作用对账");
  assert.equal(humanTaskStatusLabel("pending"), "待处理");
  assert.deepEqual(humanTaskActionPresentation("retry"), {
    label: "检查后重试",
    pendingLabel: "正在重试…",
    variant: "secondary",
  });
  assert.equal(
    humanTaskActionPresentation("acknowledge").label,
    "确认核对结果并继续",
  );
});

test("orders only actions allowed by the task", () => {
  assert.deepEqual(
    orderedHumanTaskActions(
      task("approval", {
        allowedActions: ["cancel", "approve", "reject", "approve"],
      }),
    ),
    ["approve", "reject", "cancel"],
  );
  assert.deepEqual(
    orderedHumanTaskActions(
      task("recovery", {
        taskType: "recovery",
        allowedActions: ["cancel", "approve", "retry"],
      }),
    ),
    ["retry", "approve", "cancel"],
  );
  assert.deepEqual(
    orderedHumanTaskActions(
      task("reconciliation", {
        taskType: "reconciliation",
        allowedActions: ["cancel", "acknowledge"],
      }),
    ),
    ["acknowledge", "cancel"],
  );
});

test("reads structured input requests without guessing from other tasks", () => {
  const request = {
    requestId: "request-1",
    questions: [],
  };
  assert.deepEqual(
    humanTaskInputRequest(
      task("input", {
        taskType: "input_request",
        payload: { request },
      }),
    ),
    request,
  );
  assert.equal(
    humanTaskInputRequest(task("approval", { payload: { request } })),
    null,
  );
});

test("keeps list and detail selected by the same task id", () => {
  const tasks = [task("first"), task("second")];
  assert.equal(reconcileHumanTaskSelection(tasks, "second"), "second");
  assert.equal(
    reconcileHumanTaskSelection(tasks, "missing", "second"),
    "second",
  );
  assert.equal(reconcileHumanTaskSelection(tasks, "missing"), "first");
  assert.equal(reconcileHumanTaskSelection([], "first"), null);
});

test("keeps only pending tasks and orders the oldest first", () => {
  const tasks = sortPendingHumanTasks([
    task("newer", { createdAt: "2026-08-17T01:00:00Z" }),
    task("completed", { status: "completed" }),
    task("older", { createdAt: "2026-08-16T23:00:00Z" }),
  ]);
  assert.deepEqual(
    tasks.map((item) => item.id),
    ["older", "newer"],
  );
});
