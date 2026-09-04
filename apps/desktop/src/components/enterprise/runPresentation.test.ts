import assert from "node:assert/strict";
import test from "node:test";
import {
  checkpointStatusLabel,
  formatDuration,
  formatScalarValue,
  payloadFields,
  runStatusPresentation,
} from "./runPresentation.ts";

test("run statuses use action-oriented Chinese labels", () => {
  assert.deepEqual(runStatusPresentation("succeeded"), {
    description: "所有必要步骤均已完成，最终结果已经生成。",
    label: "运行成功",
    variant: "success",
  });
  assert.equal(runStatusPresentation("waiting_approval").label, "等待审批");
  assert.equal(checkpointStatusLabel("committed"), "已保存");
});

test("durations stay compact and reject invalid ranges", () => {
  assert.equal(
    formatDuration("2026-08-28T15:57:31Z", "2026-08-28T15:58:36Z"),
    "1 分 5 秒",
  );
  assert.equal(
    formatDuration("2026-08-28T15:58:36Z", "2026-08-28T15:57:31Z"),
    "—",
  );
  assert.equal(formatDuration(null, null), "—");
});

test("scalar values are readable without flattening structured data", () => {
  assert.equal(formatScalarValue(true), "是");
  assert.equal(formatScalarValue(false), "否");
  assert.equal(formatScalarValue(null), "—");
  assert.equal(formatScalarValue({ status: "ok" }), null);
});

test("payload fields use the frozen workflow schema without industry guesses", () => {
  const fields = payloadFields(
    {
      runtimeSignal: true,
      record_id: "record-7",
      priority: 2,
    },
    {
      type: "object",
      properties: {
        priority: {
          title: "业务优先级",
          description: "由当前流程定义提供的展示语义",
        },
        record_id: { title: "记录编号" },
      },
    },
  );

  assert.deepEqual(
    fields.map(({ description, key, label }) => ({ description, key, label })),
    [
      {
        description: "由当前流程定义提供的展示语义",
        key: "priority",
        label: "业务优先级",
      },
      { description: null, key: "record_id", label: "记录编号" },
      { description: null, key: "runtimeSignal", label: "Runtime Signal" },
    ],
  );
});

test("platform event payload fields follow the selected interface language", () => {
  const payload = {
    caseId: "case-7",
    payload_ref: "connection://demo/case-7",
    synthetic: true,
    eventKind: "case_submitted",
    summary: {},
  };

  assert.deepEqual(
    payloadFields(payload, undefined).map((field) => field.label),
    ["案件 ID", "载荷引用", "合成数据", "事件类型", "摘要"],
  );
  assert.deepEqual(
    payloadFields(payload, undefined, "en-US").map((field) => field.label),
    ["Case ID", "Payload reference", "Synthetic data", "Event kind", "Summary"],
  );
});
