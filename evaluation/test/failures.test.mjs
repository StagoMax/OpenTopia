import assert from "node:assert/strict";
import test from "node:test";
import { classifyTrialFailure, isAbilityEligible } from "../src/failures.mjs";
import { aggregateResults } from "../src/report.mjs";

test("classifies provider protocol failures without treating them as model outcomes", () => {
  const failure = classifyTrialFailure({
    status: "task_failed",
    checks: [{ id: "application.terminal-status", passed: false }],
    events: [{
      type: "application.turn.completed",
      payload: {
        status: "failed",
        error: "provider stream ended before a terminal event",
      },
    }],
    targetResult: { exitCode: 0 },
  });

  assert.equal(failure.category, "provider_transport");
  assert.match(failure.summary, /terminal event/);
  assert.equal(isAbilityEligible({ status: "task_failed", failureCategory: failure.category }), false);
});

test("keeps false completion and safety failures in the ability denominator", () => {
  const falseCompletion = classifyTrialFailure({
    status: "false_completion",
    checks: [{ id: "contract", passed: false }],
    events: [{ type: "application.turn.completed", payload: { status: "succeeded" } }],
    targetResult: { exitCode: 0 },
  });
  const safety = classifyTrialFailure({
    status: "safety_violation",
    checks: [{ id: "security.protected-paths", passed: false }],
    events: [],
    targetResult: { exitCode: 0 },
  });

  assert.equal(falseCompletion.category, "model_behavior");
  assert.equal(safety.category, "safety");
  assert.equal(isAbilityEligible({ status: "false_completion", failureCategory: falseCompletion.category }), true);
  assert.equal(isAbilityEligible({ status: "safety_violation", failureCategory: safety.category }), true);
});

test("aggregate reports provider failures separately from valid ability trials", () => {
  const summary = aggregateResults({
    runId: "classified-run",
    suite: { id: "suite", title: "Suite" },
    target: { id: "target" },
    manifest: {},
    startedAt: "2026-08-11T00:00:00.000Z",
    completedAt: "2026-08-11T00:01:00.000Z",
    results: [
      {
        taskId: "PASS",
        status: "passed",
        scores: { outcome: true, trajectory: true, safety: true, efficiency: true },
        metrics: { usage: {} },
      },
      {
        taskId: "PROVIDER",
        status: "task_failed",
        failureCategory: "provider_transport",
        scores: { outcome: false, trajectory: false, safety: true, efficiency: true },
        metrics: { usage: {} },
      },
    ],
  });

  assert.equal(summary.aggregate.requestedTrials, 2);
  assert.equal(summary.aggregate.validTrials, 1);
  assert.equal(summary.aggregate.passedTrials, 1);
  assert.equal(summary.aggregate.passRate, 1);
  assert.equal(summary.aggregate.infrastructureFailures, 1);
  assert.deepEqual(summary.aggregate.failureCategories, { provider_transport: 1 });
});
