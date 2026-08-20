import type { UsageCall } from "./usageLogs";
import type { TokenEstimateBreakdown } from "./types";
import {
  addTokenBreakdown,
  emptyTokenBreakdown,
} from "./usageTokenBreakdown.ts";

export type ModelBreakdownGroup = {
  key: string;
  model: string;
  providerId: string | null;
  calls: UsageCall[];
  breakdown: TokenEstimateBreakdown;
  actualInputTokens: number;
  reportedCallCount: number;
};

export function modelBreakdownGroups(
  calls: UsageCall[],
): ModelBreakdownGroup[] {
  const groups = new Map<string, ModelBreakdownGroup>();

  for (const call of calls) {
    if (!call.inputBreakdown) continue;

    const model = call.model || call.adapter || "未记录模型";
    const key = `${call.providerId ?? "unknown-provider"}\u0000${model}`;
    let group = groups.get(key);
    if (!group) {
      group = {
        key,
        model,
        providerId: call.providerId,
        calls: [],
        breakdown: emptyTokenBreakdown(),
        actualInputTokens: 0,
        reportedCallCount: 0,
      };
      groups.set(key, group);
    }

    group.calls.push(call);
    addTokenBreakdown(group.breakdown, call.inputBreakdown);
    if (call.providerUsageReported) {
      group.actualInputTokens += call.inputTokens;
      group.reportedCallCount += 1;
    }
  }

  return [...groups.values()].sort(
    (left, right) => right.breakdown.total - left.breakdown.total,
  );
}
