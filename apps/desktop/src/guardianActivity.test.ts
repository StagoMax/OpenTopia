import assert from "node:assert/strict";
import test from "node:test";
import type { GuardianReviewCompletedPayload } from "./guardianActivity";

const { guardianActivityPresentation } = await import(
  "./guardianActivity" + ".ts"
);

function review(
  status: GuardianReviewCompletedPayload["status"],
  overrides: Partial<GuardianReviewCompletedPayload> = {},
): GuardianReviewCompletedPayload {
  return {
    type: "automatic_approval_review_completed",
    review_id: "review-1",
    target_item_id: "call-1",
    status,
    risk_level: null,
    user_authorization: null,
    rationale: "原始审核原因",
    action: { type: "command", command: "git status" },
    usage: {
      inputTokens: 1_200,
      outputTokens: 34,
      totalTokens: 1_234,
      cachedInputTokens: 800,
      reasoningTokens: 12,
    },
    attempts: 2,
    tool_rounds: 1,
    decision_source: "guardian",
    failure_kind: null,
    ...overrides,
  };
}

test("guardian activity distinguishes business approval outcomes", () => {
  assert.deepEqual(
    ["approved", "needs_user_approval", "denied_by_policy"].map((status) => {
      const view = guardianActivityPresentation(
        review(status as GuardianReviewCompletedPayload["status"]),
      );
      return [view.title, view.tone, view.icon];
    }),
    [
      ["自动审批已通过", "success", "approved"],
      ["自动审批需要用户决定", "waiting", "waiting"],
      ["操作已被安全策略拒绝", "error", "denied"],
    ],
  );
});

test("guardian technical failures are not presented as risk denials", () => {
  const unavailable = guardianActivityPresentation(
    review("reviewer_unavailable", {
      rationale: "Provider request failed: 504 Gateway Timeout",
      decision_source: "runtime",
      failure_kind: "reviewer_unavailable",
    }),
  );
  const invalid = guardianActivityPresentation(
    review("invalid_reviewer_response", {
      rationale: "The reviewer returned invalid JSON.",
      decision_source: "runtime",
      failure_kind: "invalid_reviewer_response",
    }),
  );

  assert.equal(unavailable.title, "自动审批服务不可用");
  assert.equal(
    unavailable.rationale,
    "Provider request failed: 504 Gateway Timeout",
  );
  assert.equal(invalid.title, "自动审批响应无效");
  assert.equal(invalid.rationale, "The reviewer returned invalid JSON.");
  assert.notEqual(unavailable.title, "操作已被安全策略拒绝");
  assert.notEqual(invalid.title, "自动审批需要用户决定");
});

test("guardian activity exposes compact usage and attempt metrics", () => {
  const view = guardianActivityPresentation(review("approved"));
  assert.deepEqual(view.metrics, [
    { label: "尝试", value: "2 次" },
    { label: "工具", value: "1 轮" },
    { label: "Token", value: "1,234" },
    { label: "输入/输出", value: "1,200 / 34" },
    { label: "缓存", value: "800" },
    { label: "推理", value: "12" },
  ]);
});

test("guardian activity handles aborted and in-progress completion records", () => {
  assert.equal(
    guardianActivityPresentation(review("aborted")).title,
    "自动审批已中止",
  );
  assert.equal(
    guardianActivityPresentation(review("in_progress")).title,
    "正在自动审批",
  );
});
