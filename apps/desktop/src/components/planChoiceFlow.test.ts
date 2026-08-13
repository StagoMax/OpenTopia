import assert from "node:assert/strict";
import test from "node:test";
import type { UserInputRequest } from "../types";

const { buildPlanChoiceResponse, CUSTOM_OPTION_ID } = await import(
  "./planChoiceFlow" + ".ts"
);

const request: UserInputRequest = {
  requestId: "request-1",
  questions: [
    {
      id: "scope",
      header: "范围",
      question: "这次计划覆盖什么？",
      options: [
        {
          id: "feature",
          label: "单个功能",
          description: "只规划当前功能。",
          recommended: true,
        },
      ],
      allowCustom: false,
    },
    {
      id: "depth",
      header: "深度",
      question: "计划需要多详细？",
      options: [
        {
          id: "detailed",
          label: "详细",
          description: "包含验证步骤。",
          recommended: true,
        },
      ],
      allowCustom: true,
    },
    {
      id: "delivery",
      header: "交付",
      question: "最终如何交付？",
      options: [
        {
          id: "chat",
          label: "对话内",
          description: "直接在对话中交付。",
          recommended: false,
        },
      ],
      allowCustom: false,
    },
  ],
};

test("builds one response from a completed multi-step flow", () => {
  assert.deepEqual(
    buildPlanChoiceResponse(
      request,
      { scope: "feature", depth: "detailed", delivery: "chat" },
      {},
    ),
    {
      answers: [
        { questionId: "scope", optionId: "feature" },
        { questionId: "depth", optionId: "detailed" },
        { questionId: "delivery", optionId: "chat" },
      ],
    },
  );
});

test("trims and includes a custom answer", () => {
  assert.deepEqual(
    buildPlanChoiceResponse(
      request,
      { scope: "feature", depth: CUSTOM_OPTION_ID, delivery: "chat" },
      { depth: "  只列风险和验证步骤  " },
    ),
    {
      answers: [
        { questionId: "scope", optionId: "feature" },
        { questionId: "depth", customText: "只列风险和验证步骤" },
        { questionId: "delivery", optionId: "chat" },
      ],
    },
  );
});

test("does not submit an incomplete or invalid flow", () => {
  assert.equal(
    buildPlanChoiceResponse(
      request,
      { scope: "feature", depth: "detailed" },
      {},
    ),
    null,
  );
  assert.equal(
    buildPlanChoiceResponse(
      request,
      { scope: "unknown", depth: "detailed", delivery: "chat" },
      {},
    ),
    null,
  );
  assert.equal(
    buildPlanChoiceResponse(
      request,
      { scope: "feature", depth: CUSTOM_OPTION_ID, delivery: "chat" },
      { depth: "   " },
    ),
    null,
  );
});

test("rejects a custom marker when the question does not allow custom text", () => {
  assert.equal(
    buildPlanChoiceResponse(
      request,
      {
        scope: CUSTOM_OPTION_ID,
        depth: "detailed",
        delivery: "chat",
      },
      { scope: "扩大到整个项目" },
    ),
    null,
  );
});
