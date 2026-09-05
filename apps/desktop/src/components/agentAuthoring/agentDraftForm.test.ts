import assert from "node:assert/strict";
import test from "node:test";
import {
  agentDraftFieldErrorFromCreateFailure,
  validateAgentTemplateId,
} from "./agentDraftValidation.ts";

test("validates Agent template IDs with the backend slug rules", () => {
  assert.equal(validateAgentTemplateId("audit.agent_v2-1"), null);
  assert.equal(
    validateAgentTemplateId(" Audit.Agent "),
    "只能使用小写英文字母、数字、点（.）、下划线（_）或连字符（-），且不超过 120 个字符。",
  );
  assert.equal(validateAgentTemplateId(""), "请输入智能体 ID。");
  assert.equal(
    validateAgentTemplateId("a".repeat(121), "en-US"),
    "Use only lowercase letters, numbers, dots (.), underscores (_), or hyphens (-), with no more than 120 characters.",
  );
});

test("maps a backend templateId failure back to the matching field", () => {
  const failure = new Error(
    JSON.stringify({
      error:
        "templateId must be a lowercase slug containing only letters, numbers, '.', '_' or '-'",
    }),
  );
  assert.deepEqual(agentDraftFieldErrorFromCreateFailure(failure), {
    field: "templateId",
    message:
      "只能使用小写英文字母、数字、点（.）、下划线（_）或连字符（-），且不超过 120 个字符。",
  });
  assert.equal(
    agentDraftFieldErrorFromCreateFailure(new Error("network unavailable")),
    null,
  );
});
