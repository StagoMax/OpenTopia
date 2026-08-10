import { writeFile } from "node:fs/promises";
import path from "node:path";
import { isDeepStrictEqual } from "node:util";
import { ensureDirectory } from "./utils.mjs";

const EXCLUDED_STATUSES = new Set([
  "infra_error",
  "grader_error",
  "invalid_task",
]);

function finiteRatio(value, fallback = null) {
  return Number.isFinite(value) ? value : fallback;
}

function average(values) {
  const finite = values.filter(Number.isFinite);
  return finite.length === 0
    ? null
    : finite.reduce((sum, value) => sum + value, 0) / finite.length;
}

function rate(results, predicate) {
  return results.length === 0
    ? null
    : results.filter(predicate).length / results.length;
}

function increaseRatio(baseline, candidate) {
  if (!Number.isFinite(baseline) || !Number.isFinite(candidate)) return null;
  if (baseline === 0) return candidate === 0 ? 0 : Infinity;
  return (candidate - baseline) / baseline;
}

export function harnessMetrics(summary) {
  const results = summary.results ?? [];
  const valid = results.filter(
    (result) => !EXCLUDED_STATUSES.has(result.status),
  );
  const passed = valid.filter((result) => result.status === "passed");
  const providerTokens = valid.reduce(
    (sum, result) => sum + (result.metrics?.usage?.providerTotalTokens ?? 0),
    0,
  );
  const uncachedTokens = valid.reduce(
    (sum, result) =>
      sum +
      (result.metrics?.usage?.uncachedInputTokens ?? 0) +
      (result.metrics?.usage?.outputTokens ?? 0),
    0,
  );
  const reportedCosts = valid
    .map((result) => result.metrics?.usage?.estimatedCost)
    .filter(Number.isFinite);
  const taskRates = Object.fromEntries(
    (summary.tasks ?? []).map((task) => [task.taskId, task.passRate]),
  );
  const expectedRecoveries = valid.reduce(
    (sum, result) =>
      sum +
      (result.process?.stages ?? []).filter((stage) => stage.restartBefore)
        .length,
    0,
  );
  const successfulRecoveries = valid.reduce(
    (sum, result) =>
      sum + (result.metrics?.longHorizon?.successfulRecoveries ?? 0),
    0,
  );
  const usageTotal = (field) =>
    valid.reduce(
      (sum, result) => sum + (result.metrics?.usage?.[field] ?? 0),
      0,
    );
  return {
    requestedTrials: results.length,
    validTrials: valid.length,
    passedTrials: passed.length,
    passRate: valid.length === 0 ? null : passed.length / valid.length,
    infrastructureFailures: results.length - valid.length,
    outcomeRate: rate(valid, (result) => result.scores?.outcome === true),
    trajectoryRate: rate(
      valid,
      (result) => result.scores?.trajectory === true,
    ),
    safetyRate: rate(valid, (result) => result.scores?.safety === true),
    efficiencyRate: rate(
      valid,
      (result) => result.scores?.efficiency === true,
    ),
    completionClaimRate: rate(
      valid,
      (result) =>
        (result.metrics?.longHorizon?.completionClaims ?? 0) > 0,
    ),
    recoveryRate:
      expectedRecoveries === 0
        ? null
        : successfulRecoveries / expectedRecoveries,
    expectedRecoveries,
    successfulRecoveries,
    averageProviderTokens:
      valid.length === 0 ? null : providerTokens / valid.length,
    averageUncachedTokens:
      valid.length === 0 ? null : uncachedTokens / valid.length,
    finalizationGuardRejects: usageTotal("finalizationGuardRejectCount"),
    invalidToolLoops: usageTotal("invalidToolLoopCount"),
    noProgressSignals: usageTotal("noProgressSignalCount"),
    duplicatePlans: usageTotal("duplicatePlanCount"),
    tokensPerSuccess:
      passed.length === 0 ? null : providerTokens / passed.length,
    uncachedTokensPerSuccess:
      passed.length === 0 ? null : uncachedTokens / passed.length,
    costPerSuccess:
      passed.length === 0 || reportedCosts.length !== valid.length
        ? null
        : reportedCosts.reduce((sum, value) => sum + value, 0) / passed.length,
    averageElapsedMs: average(valid.map((result) => result.elapsedMs)),
    averageToolCalls: average(valid.map((result) => result.metrics?.toolCalls)),
    taskRates,
  };
}

function comparisonCheck(id, passed, actual, limit, detail) {
  return { id, passed, actual, limit, detail };
}

function comparableRunContract(summary) {
  return {
    targetId: summary.target?.id ?? null,
    suiteSha256: summary.manifest?.suiteSha256 ?? null,
    targetSha256: summary.manifest?.targetSha256 ?? null,
    taskHashes: summary.manifest?.taskHashes ?? null,
    repetitions: summary.manifest?.repetitions ?? null,
  };
}

function assertComparableExperiments(baseline, candidate) {
  const before = baseline.manifest?.experiment ?? null;
  const after = candidate.manifest?.experiment ?? null;
  if (before === null && after === null) return null;
  if (before === null || after === null) {
    throw new Error(
      "Cannot compare an experiment-tagged run with an untagged run",
    );
  }
  for (const field of ["experimentId", "pairingKey"]) {
    if (before[field] !== after[field]) {
      throw new Error(
        `Cannot compare experiments with different ${field}: ${before[field]} vs ${after[field]}`,
      );
    }
  }
  if (!isDeepStrictEqual(before.controlled, after.controlled)) {
    throw new Error("Cannot compare experiments with different controlled factors");
  }
  return {
    experimentId: before.experimentId,
    pairingKey: before.pairingKey,
    baselineVariant: before.variant,
    candidateVariant: after.variant,
    controlled: before.controlled,
    baselineTreatment: before.treatment,
    candidateTreatment: after.treatment,
  };
}

function assertComparableRuns(baseline, candidate) {
  if (baseline.suite?.id !== candidate.suite?.id) {
    throw new Error(
      `Cannot compare different suites: ${baseline.suite?.id ?? "unknown"} vs ${candidate.suite?.id ?? "unknown"}`,
    );
  }
  const before = comparableRunContract(baseline);
  const after = comparableRunContract(candidate);
  for (const field of [
    "targetId",
    "suiteSha256",
    "targetSha256",
    "repetitions",
  ]) {
    if (
      before[field] !== null &&
      after[field] !== null &&
      before[field] !== after[field]
    ) {
      throw new Error(
        `Cannot compare runs with different ${field}: ${before[field]} vs ${after[field]}`,
      );
    }
  }
  if (before.taskHashes !== null && after.taskHashes !== null) {
    const baselineHashes = JSON.stringify(before.taskHashes);
    const candidateHashes = JSON.stringify(after.taskHashes);
    if (baselineHashes !== candidateHashes) {
      throw new Error("Cannot compare runs with different task fixture hashes");
    }
  }
  return { baseline: before, candidate: after };
}

export function compareSummaries(
  baseline,
  candidate,
  {
    maxPassRateDrop = 0,
    maxTaskPassRateDrop = 0,
    maxTokenIncreaseRatio = 0.2,
    maxLatencyIncreaseRatio = 0.2,
  } = {},
) {
  const runContract = assertComparableRuns(baseline, candidate);
  const experiment = assertComparableExperiments(baseline, candidate);
  const baselineMetrics = harnessMetrics(baseline);
  const candidateMetrics = harnessMetrics(candidate);
  const passRateDrop =
    Number.isFinite(baselineMetrics.passRate) &&
    Number.isFinite(candidateMetrics.passRate)
      ? baselineMetrics.passRate - candidateMetrics.passRate
      : Infinity;
  const tokenIncrease = increaseRatio(
    baselineMetrics.tokensPerSuccess,
    candidateMetrics.tokensPerSuccess,
  );
  const latencyIncrease = increaseRatio(
    baselineMetrics.averageElapsedMs,
    candidateMetrics.averageElapsedMs,
  );
  const rateDrop = (field) => {
    const before = baselineMetrics[field];
    const after = candidateMetrics[field];
    return Number.isFinite(before) && Number.isFinite(after)
      ? before - after
      : null;
  };
  const sharedTasks = Object.keys(baselineMetrics.taskRates).filter((taskId) =>
    Object.hasOwn(candidateMetrics.taskRates, taskId),
  );
  const taskRegressions = sharedTasks.flatMap((taskId) => {
    const before = baselineMetrics.taskRates[taskId];
    const after = candidateMetrics.taskRates[taskId];
    if (!Number.isFinite(before) || !Number.isFinite(after)) return [];
    const drop = before - after;
    return drop > maxTaskPassRateDrop ? [{ taskId, before, after, drop }] : [];
  });
  const checks = [
    comparisonCheck(
      "pass-rate",
      passRateDrop <= maxPassRateDrop,
      finiteRatio(passRateDrop),
      maxPassRateDrop,
      "candidate pass-rate drop",
    ),
    comparisonCheck(
      "outcome-rate",
      rateDrop("outcomeRate") === null || rateDrop("outcomeRate") <= 0,
      finiteRatio(rateDrop("outcomeRate")),
      0,
      "candidate outcome-rate drop",
    ),
    comparisonCheck(
      "trajectory-rate",
      rateDrop("trajectoryRate") === null ||
        rateDrop("trajectoryRate") <= 0,
      finiteRatio(rateDrop("trajectoryRate")),
      0,
      "candidate trajectory-rate drop",
    ),
    comparisonCheck(
      "safety-rate",
      rateDrop("safetyRate") === null || rateDrop("safetyRate") <= 0,
      finiteRatio(rateDrop("safetyRate")),
      0,
      "candidate safety-rate drop",
    ),
    comparisonCheck(
      "efficiency-rate",
      rateDrop("efficiencyRate") === null || rateDrop("efficiencyRate") <= 0,
      finiteRatio(rateDrop("efficiencyRate")),
      0,
      "candidate efficiency-rate drop",
    ),
    comparisonCheck(
      "recovery-rate",
      rateDrop("recoveryRate") === null || rateDrop("recoveryRate") <= 0,
      finiteRatio(rateDrop("recoveryRate")),
      0,
      "candidate recovery-rate drop",
    ),
    comparisonCheck(
      "task-pass-rate",
      taskRegressions.length === 0,
      taskRegressions,
      maxTaskPassRateDrop,
      "per-task pass-rate regressions",
    ),
    comparisonCheck(
      "infrastructure-failures",
      candidateMetrics.infrastructureFailures <=
        baselineMetrics.infrastructureFailures,
      candidateMetrics.infrastructureFailures,
      baselineMetrics.infrastructureFailures,
      "candidate infrastructure failures",
    ),
    comparisonCheck(
      "tokens-per-success",
      tokenIncrease === null || tokenIncrease <= maxTokenIncreaseRatio,
      finiteRatio(tokenIncrease),
      maxTokenIncreaseRatio,
      "relative token increase",
    ),
    comparisonCheck(
      "average-latency",
      latencyIncrease === null || latencyIncrease <= maxLatencyIncreaseRatio,
      finiteRatio(latencyIncrease),
      maxLatencyIncreaseRatio,
      "relative elapsed-time increase",
    ),
    comparisonCheck(
      "finalization-guard-rejects",
      candidateMetrics.finalizationGuardRejects <=
        baselineMetrics.finalizationGuardRejects,
      candidateMetrics.finalizationGuardRejects,
      baselineMetrics.finalizationGuardRejects,
      "candidate finalization guard rejects",
    ),
    comparisonCheck(
      "invalid-tool-loops",
      candidateMetrics.invalidToolLoops <= baselineMetrics.invalidToolLoops,
      candidateMetrics.invalidToolLoops,
      baselineMetrics.invalidToolLoops,
      "candidate invalid tool loops",
    ),
    comparisonCheck(
      "no-progress-signals",
      candidateMetrics.noProgressSignals <= baselineMetrics.noProgressSignals,
      candidateMetrics.noProgressSignals,
      baselineMetrics.noProgressSignals,
      "candidate no-progress signals",
    ),
    comparisonCheck(
      "duplicate-plans",
      candidateMetrics.duplicatePlans <= baselineMetrics.duplicatePlans,
      candidateMetrics.duplicatePlans,
      baselineMetrics.duplicatePlans,
      "candidate duplicate plans",
    ),
  ];
  return {
    schemaVersion: 1,
    suite: baseline.suite,
    runContract,
    experiment,
    baselineRunId: baseline.runId,
    candidateRunId: candidate.runId,
    status: checks.every((check) => check.passed) ? "passed" : "failed",
    thresholds: {
      maxPassRateDrop,
      maxTaskPassRateDrop,
      maxTokenIncreaseRatio,
      maxLatencyIncreaseRatio,
    },
    baseline: baselineMetrics,
    candidate: candidateMetrics,
    deltas: {
      passRateDrop: finiteRatio(passRateDrop),
      tokenIncreaseRatio: finiteRatio(tokenIncrease),
      latencyIncreaseRatio: finiteRatio(latencyIncrease),
    },
    checks,
  };
}

function renderNumber(value) {
  return value === null || value === undefined
    ? "n/a"
    : Number(value).toFixed(3);
}

export function renderComparisonMarkdown(comparison) {
  const lines = [
    `# Harness Comparison: ${comparison.suite?.title ?? comparison.suite?.id}`,
    "",
    `- Baseline: \`${comparison.baselineRunId}\``,
    `- Candidate: \`${comparison.candidateRunId}\``,
    `- Gate: **${comparison.status.toUpperCase()}**`,
    ...(comparison.experiment
      ? [
          `- Experiment: \`${comparison.experiment.experimentId}\``,
          `- Variants: \`${comparison.experiment.baselineVariant}\` -> \`${comparison.experiment.candidateVariant}\``,
        ]
      : []),
    "",
    "| Metric | Baseline | Candidate |",
    "|---|---:|---:|",
    `| Pass rate | ${renderNumber(comparison.baseline.passRate)} | ${renderNumber(comparison.candidate.passRate)} |`,
    `| Outcome rate | ${renderNumber(comparison.baseline.outcomeRate)} | ${renderNumber(comparison.candidate.outcomeRate)} |`,
    `| Trajectory rate | ${renderNumber(comparison.baseline.trajectoryRate)} | ${renderNumber(comparison.candidate.trajectoryRate)} |`,
    `| Safety rate | ${renderNumber(comparison.baseline.safetyRate)} | ${renderNumber(comparison.candidate.safetyRate)} |`,
    `| Efficiency rate | ${renderNumber(comparison.baseline.efficiencyRate)} | ${renderNumber(comparison.candidate.efficiencyRate)} |`,
    `| Recovery rate | ${renderNumber(comparison.baseline.recoveryRate)} | ${renderNumber(comparison.candidate.recoveryRate)} |`,
    `| Completion claim rate | ${renderNumber(comparison.baseline.completionClaimRate)} | ${renderNumber(comparison.candidate.completionClaimRate)} |`,
    `| Average provider tokens | ${renderNumber(comparison.baseline.averageProviderTokens)} | ${renderNumber(comparison.candidate.averageProviderTokens)} |`,
    `| Average uncached tokens | ${renderNumber(comparison.baseline.averageUncachedTokens)} | ${renderNumber(comparison.candidate.averageUncachedTokens)} |`,
    `| Tokens/success | ${renderNumber(comparison.baseline.tokensPerSuccess)} | ${renderNumber(comparison.candidate.tokensPerSuccess)} |`,
    `| Uncached tokens/success | ${renderNumber(comparison.baseline.uncachedTokensPerSuccess)} | ${renderNumber(comparison.candidate.uncachedTokensPerSuccess)} |`,
    `| Cost/success | ${renderNumber(comparison.baseline.costPerSuccess)} | ${renderNumber(comparison.candidate.costPerSuccess)} |`,
    `| Average elapsed ms | ${renderNumber(comparison.baseline.averageElapsedMs)} | ${renderNumber(comparison.candidate.averageElapsedMs)} |`,
    `| Average tool calls | ${renderNumber(comparison.baseline.averageToolCalls)} | ${renderNumber(comparison.candidate.averageToolCalls)} |`,
    `| Infrastructure failures | ${comparison.baseline.infrastructureFailures} | ${comparison.candidate.infrastructureFailures} |`,
    `| Finalization guard rejects | ${comparison.baseline.finalizationGuardRejects} | ${comparison.candidate.finalizationGuardRejects} |`,
    `| Invalid tool loops | ${comparison.baseline.invalidToolLoops} | ${comparison.candidate.invalidToolLoops} |`,
    `| No-progress signals | ${comparison.baseline.noProgressSignals} | ${comparison.candidate.noProgressSignals} |`,
    `| Duplicate plans | ${comparison.baseline.duplicatePlans} | ${comparison.candidate.duplicatePlans} |`,
    "",
    "| Gate | Passed | Actual | Limit |",
    "|---|---:|---:|---:|",
  ];
  for (const check of comparison.checks) {
    const actual = Array.isArray(check.actual)
      ? JSON.stringify(check.actual)
      : renderNumber(check.actual);
    lines.push(
      `| ${check.id} | ${check.passed ? "yes" : "no"} | ${actual} | ${renderNumber(check.limit)} |`,
    );
  }
  lines.push("");
  return `${lines.join("\n")}\n`;
}

export async function writeComparison(outputDirectory, comparison) {
  await ensureDirectory(outputDirectory);
  const jsonPath = path.join(outputDirectory, "comparison.json");
  const markdownPath = path.join(outputDirectory, "comparison.md");
  await writeFile(jsonPath, `${JSON.stringify(comparison, null, 2)}\n`, "utf8");
  await writeFile(markdownPath, renderComparisonMarkdown(comparison), "utf8");
  return { jsonPath, markdownPath };
}
