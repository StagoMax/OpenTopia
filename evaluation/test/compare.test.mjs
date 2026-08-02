import assert from "node:assert/strict";
import test from "node:test";
import { compareSummaries, harnessMetrics } from "../src/compare.mjs";

function summary(runId, trials) {
  const grouped = new Map();
  for (const trial of trials) {
    const entries = grouped.get(trial.taskId) ?? [];
    entries.push(trial);
    grouped.set(trial.taskId, entries);
  }
  return {
    schemaVersion: 1,
    runId,
    suite: { id: "harness", title: "Harness" },
    results: trials,
    tasks: [...grouped.entries()].map(([taskId, entries]) => {
      const passed = entries.filter((entry) => entry.status === "passed").length;
      return { taskId, passRate: passed / entries.length };
    })
  };
}

function trial(taskId, status, elapsedMs, tokens, toolCalls = 1) {
  return {
    taskId,
    status,
    elapsedMs,
    metrics: {
      usage: { providerTotalTokens: tokens },
      toolCalls
    }
  };
}

test("passes a candidate that preserves outcomes while reducing cost", () => {
  const baseline = summary("base", [
    trial("patch", "passed", 1000, 1000, 4),
    trial("phase", "passed", 1200, 1200, 3)
  ]);
  const candidate = summary("next", [
    trial("patch", "passed", 800, 800, 2),
    trial("phase", "passed", 1000, 900, 3)
  ]);
  const comparison = compareSummaries(baseline, candidate);
  assert.equal(comparison.status, "passed");
  assert.equal(harnessMetrics(candidate).tokensPerSuccess, 850);
});

test("fails a cheaper candidate that regresses a task outcome", () => {
  const baseline = summary("base", [
    trial("patch", "passed", 1000, 1000),
    trial("phase", "passed", 1000, 1000)
  ]);
  const candidate = summary("next", [
    trial("patch", "failed", 500, 100),
    trial("phase", "passed", 500, 100)
  ]);
  const comparison = compareSummaries(baseline, candidate);
  assert.equal(comparison.status, "failed");
  assert.equal(comparison.checks.find((check) => check.id === "task-pass-rate").passed, false);
});

test("rejects summaries from different suites", () => {
  const baseline = summary("base", [trial("patch", "passed", 1000, 1000)]);
  const candidate = summary("next", [trial("patch", "passed", 1000, 1000)]);
  candidate.suite.id = "other";
  assert.throws(() => compareSummaries(baseline, candidate), /different suites/);
});

test("rejects runs when the supposedly fixed evaluation contract changed", () => {
  const baseline = summary("base", [trial("patch", "passed", 1000, 1000)]);
  const candidate = summary("next", [trial("patch", "passed", 1000, 1000)]);
  baseline.target = candidate.target = { id: "opentopia-http" };
  baseline.manifest = {
    suiteSha256: "suite-a",
    targetSha256: "target-a",
    taskHashes: { patch: "task-a" },
    repetitions: 3
  };
  candidate.manifest = {
    ...baseline.manifest,
    taskHashes: { patch: "task-b" }
  };
  assert.throws(() => compareSummaries(baseline, candidate), /different task fixture hashes/);
});
