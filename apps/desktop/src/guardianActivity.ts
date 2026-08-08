import type { AgentEventPayload } from "./types";

export type GuardianReviewCompletedPayload = Extract<
  AgentEventPayload,
  { type: "automatic_approval_review_completed" }
>;

export type GuardianActivityTone = "neutral" | "waiting" | "success" | "error";

export type GuardianActivityIcon =
  "approved" | "waiting" | "denied" | "unavailable" | "invalid" | "aborted";

export type GuardianActivityMetric = {
  label: string;
  value: string;
};

export type GuardianActivityPresentation = {
  title: string;
  tone: GuardianActivityTone;
  icon: GuardianActivityIcon;
  rationale: string;
  metrics: GuardianActivityMetric[];
};

export function guardianActivityPresentation(
  review: GuardianReviewCompletedPayload,
): GuardianActivityPresentation {
  const status = guardianStatusPresentation(review.status);
  return {
    ...status,
    rationale: review.rationale,
    metrics: guardianReviewMetrics(review),
  };
}

function guardianStatusPresentation(
  status: GuardianReviewCompletedPayload["status"],
): Pick<GuardianActivityPresentation, "title" | "tone" | "icon"> {
  switch (status) {
    case "approved":
      return {
        title: "自动审批已通过",
        tone: "success",
        icon: "approved",
      };
    case "needs_user_approval":
      return {
        title: "自动审批需要用户决定",
        tone: "waiting",
        icon: "waiting",
      };
    case "denied_by_policy":
      return {
        title: "操作已被安全策略拒绝",
        tone: "error",
        icon: "denied",
      };
    case "reviewer_unavailable":
      return {
        title: "自动审批服务不可用",
        tone: "error",
        icon: "unavailable",
      };
    case "invalid_reviewer_response":
      return {
        title: "自动审批响应无效",
        tone: "error",
        icon: "invalid",
      };
    case "aborted":
      return {
        title: "自动审批已中止",
        tone: "neutral",
        icon: "aborted",
      };
    case "in_progress":
      return {
        title: "正在自动审批",
        tone: "waiting",
        icon: "waiting",
      };
  }
}

function guardianReviewMetrics(
  review: GuardianReviewCompletedPayload,
): GuardianActivityMetric[] {
  const metrics: GuardianActivityMetric[] = [];
  if (review.attempts > 0) {
    metrics.push({ label: "尝试", value: `${review.attempts} 次` });
  }
  if (review.tool_rounds > 0) {
    metrics.push({ label: "工具", value: `${review.tool_rounds} 轮` });
  }

  const usage = review.usage;
  if (usage.totalTokens > 0) {
    metrics.push({ label: "Token", value: formatCount(usage.totalTokens) });
    metrics.push({
      label: "输入/输出",
      value: `${formatCount(usage.inputTokens)} / ${formatCount(usage.outputTokens)}`,
    });
  }
  if ((usage.cachedInputTokens ?? 0) > 0) {
    metrics.push({
      label: "缓存",
      value: formatCount(usage.cachedInputTokens ?? 0),
    });
  }
  if ((usage.reasoningTokens ?? 0) > 0) {
    metrics.push({
      label: "推理",
      value: formatCount(usage.reasoningTokens ?? 0),
    });
  }
  return metrics;
}

function formatCount(value: number): string {
  return new Intl.NumberFormat("zh-CN", { maximumFractionDigits: 0 }).format(
    value,
  );
}
