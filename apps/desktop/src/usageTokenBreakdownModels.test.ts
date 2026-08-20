import assert from "node:assert/strict";
import test from "node:test";

import type * as ModelGroupsModule from "./usageTokenBreakdownModels";
import type * as TokenBreakdownModule from "./usageTokenBreakdown";
import type { UsageCall } from "./usageLogs";

const { modelBreakdownGroups } = (await import(
  "./usageTokenBreakdownModels" + ".ts"
)) as typeof ModelGroupsModule;
const { emptyTokenBreakdown } = (await import(
  "./usageTokenBreakdown" + ".ts"
)) as typeof TokenBreakdownModule;

test("keeps same-named models from separate providers in distinct groups", () => {
  const groups = modelBreakdownGroups([
    call("first", "provider-a", "shared-model", 100, 80),
    call("second", "provider-a", "shared-model", 120, 100, false),
    call("third", "provider-b", "shared-model", 220, 150),
  ]);

  assert.equal(groups.length, 2);
  assert.deepEqual(
    groups.map((group) => ({
      providerId: group.providerId,
      model: group.model,
      callCount: group.calls.length,
      breakdownTotal: group.breakdown.total,
      actualInputTokens: group.actualInputTokens,
      reportedCallCount: group.reportedCallCount,
    })),
    [
      {
        providerId: "provider-a",
        model: "shared-model",
        callCount: 2,
        breakdownTotal: 180,
        actualInputTokens: 100,
        reportedCallCount: 1,
      },
      {
        providerId: "provider-b",
        model: "shared-model",
        callCount: 1,
        breakdownTotal: 150,
        actualInputTokens: 220,
        reportedCallCount: 1,
      },
    ],
  );
});

function call(
  id: string,
  providerId: string,
  model: string,
  inputTokens: number,
  localEstimate: number,
  providerUsageReported = true,
): UsageCall {
  const inputBreakdown = emptyTokenBreakdown();
  inputBreakdown.baseInstructions = localEstimate;
  inputBreakdown.total = localEstimate;
  inputBreakdown.details = [
    {
      id: "base_instructions",
      label: "Base instructions",
      tokens: localEstimate,
      children: [],
    },
  ];

  return {
    id,
    providerId,
    model,
    adapter: "openai-compatible",
    inputTokens,
    inputBreakdown,
    providerUsageReported,
  } as UsageCall;
}
