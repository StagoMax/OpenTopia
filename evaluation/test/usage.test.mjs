import assert from "node:assert/strict";
import test from "node:test";
import { summarizeUsage } from "../src/graders.mjs";

test("summarizes request-correlated estimates, module attribution, and waste signals", () => {
  const plan = {
    goalId: "goal",
    steps: [{ id: "one", status: "in_progress" }],
  };
  const events = [
    { type: "model.request.started", payload: { requestId: "agent" } },
    { type: "model.request.started", payload: { requestId: "compaction" } },
    {
      type: "model.usage",
      payload: {
        requestId: "agent",
        purpose: "agent_round",
        inputTokens: 110,
        outputTokens: 10,
        totalTokens: 120,
        cachedInputTokens: 60,
        localInputEstimate: 100,
        inputBreakdown: { baseInstructions: 40, toolSchemas: 60, total: 100 },
        estimatedCost: 0.01,
        costCurrency: "USD",
        costSource: "provider_invoice",
      },
    },
    {
      type: "model.usage",
      payload: {
        requestId: "compaction",
        purpose: "context_compaction",
        inputTokens: 220,
        outputTokens: 10,
        totalTokens: 230,
        localInputEstimate: 200,
        inputBreakdown: { currentUser: 200, total: 200 },
      },
    },
    {
      type: "model.request.retried",
      payload: {
        requestId: "agent",
        reason: "stored response cursor fallback",
      },
    },
    {
      type: "harness.waste.signal",
      payload: { stage: "invalid_tool_call_circuit_breaker" },
    },
    {
      type: "harness.waste.signal",
      payload: { stage: "finalization_guard" },
    },
    {
      type: "harness.waste.signal",
      payload: { stage: "step_reminder.repeated_tool_calls" },
    },
    { type: "agent.plan.updated", payload: { plan } },
    { type: "agent.plan.updated", payload: { plan } },
  ];

  const usage = summarizeUsage(events);

  assert.equal(usage.providerUsageCoverage, 1);
  assert.equal(usage.estimateErrorP95, 10 / 110);
  assert.equal(usage.rawEstimateErrorP95, 10 / 110);
  assert.equal(usage.estimateCalibrationFactor, 1);
  assert.equal(usage.inputTokenBreakdown.total, 300);
  assert.equal(usage.estimatedRetryInputTokens, 100);
  assert.equal(usage.compatibilityRetryCount, 1);
  assert.equal(usage.invalidToolLoopCount, 1);
  assert.equal(usage.finalizationGuardRejectCount, 1);
  assert.equal(usage.noProgressSignalCount, 1);
  assert.equal(usage.duplicatePlanCount, 1);
  assert.equal(usage.compactionRequests, 1);
  assert.equal(usage.compactionTokens, 230);
  assert.equal(usage.estimatedCost, 0.01);
  assert.equal(usage.costCurrency, "USD");
});
