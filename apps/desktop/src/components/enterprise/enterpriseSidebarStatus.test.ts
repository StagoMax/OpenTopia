import assert from "node:assert/strict";
import test from "node:test";
import { enterpriseSidebarStatus } from "./enterpriseSidebarStatus.ts";

test("maps published Flow Agent templates to the shared success dot", () => {
  assert.deepEqual(enterpriseSidebarStatus("agents", "published"), {
    label: "已发布",
    tone: "success",
  });
});

test("maps Flow runtime states onto shared sidebar status semantics", () => {
  assert.deepEqual(enterpriseSidebarStatus("runs", "running"), {
    label: "运行中",
    loading: true,
    tone: "info",
  });
  assert.deepEqual(enterpriseSidebarStatus("runs", "failed"), {
    label: "运行失败",
    tone: "danger",
  });
  assert.deepEqual(enterpriseSidebarStatus("runs", "waiting_human"), {
    label: "等待人工处理",
    tone: "warning",
  });
});
