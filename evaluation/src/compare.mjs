import { writeFile } from "node:fs/promises";
import path from "node:path";
import { ensureDirectory } from "./utils.mjs";

const EXCLUDED_STATUSES = new Set(["infra_error", "grader_error", "invalid_task"]);

function finiteRatio(value, fallback = null) {
  return Number.isFinite(value) ? value : fallback;
}

function average(values) {
  const finite = values.filter(Number.isFinite);
  return finite.length === 0 ? null : finite.reduce((sum, value) => sum + value, 0) / finite.length;
}

function increaseRatio(baseline, candidate) {
  if (!Number.isFinite(baseline) || !Number.isFinite(candidate)) return null;
  if (baseline === 0) return candidate === 0 ? 0 : Infinity;
  return (candidate - baseline) / baseline;
}

export function harnessMetrics(summary) {
  const results = summary.results ?? [];
  const valid = results.filter((result) => !EXCLUDED_STATUSES.has(result.status));
  const passed = valid.filter((result) => result.status === "passed");
  const providerTokens = valid.reduce(
    (sum, result) => sum + (result.metrics?.usage?.providerTotalTokens ?? 0),
    0
  );
  const taskRates = Object.fromEntries(
    (summary.tasks ?? []).map((task) => [task.taskId, task.passRate])
  );
  return {
    requestedTrials: results.length,
    validTrials: valid.length,
    passedTrials: passed.length,
    passRate: valid.length === 0 ? null : passed.length / valid.length,
    infrastructureFailures: results.length - valid.length,
    tokensPerSuccess: passed.length === 0 ? null : providerTokens / passed.length,
    averageElapsedMs: average(valid.map((result) => result.elapsedMs)),
    averageToolCalls: average(valid.map((result) => result.metrics?.toolCalls)),
    taskRates
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
    repetitions: summary.manifest?.repetitions ?? null
  };
}

function assertComparableRuns(baseline, candidate) {
  if (baseline.suite?.id !== candidate.suite?.id) {
    throw new Error(
      `Cannot compare different suites: ${baseline.suite?.id ?? "unknown"} vs ${candidate.suite?.id ?? "unknown"}`
    );
  }
  const before = comparableRunContract(baseline);
  const after = comparableRunContract(candidate);
  for (const field of ["targetId", "suiteSha256", "targetSha256", "repetitions"]) {
    if (before[field] !== null && after[field] !== null && before[field] !== after[field]) {
      throw new Error(`Cannot compare runs with different ${field}: ${before[field]} vs ${after[field]}`);
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
    maxLatencyIncreaseRatio = 0.2
  } = {}
) {
  const runContract = assertComparableRuns(baseline, candidate);
  const baselineMetrics = harnessMetrics(baseline);
  const candidateMetrics = harnessMetrics(candidate);
  const passRateDrop = Number.isFinite(baselineMetrics.passRate) && Number.isFinite(candidateMetrics.passRate)
    ? baselineMetrics.passRate - candidateMetrics.passRate
    : Infinity;
  const tokenIncrease = increaseRatio(
    baselineMetrics.tokensPerSuccess,
    candidateMetrics.tokensPerSuccess
  );
  const latencyIncrease = increaseRatio(
    baselineMetrics.averageElapsedMs,
    candidateMetrics.averageElapsedMs
  );
  const sharedTasks = Object.keys(baselineMetrics.taskRates)
    .filter((taskId) => Object.hasOwn(candidateMetrics.taskRates, taskId));
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
      "candidate pass-rate drop"
    ),
    comparisonCheck(
      "task-pass-rate",
      taskRegressions.length === 0,
      taskRegressions,
      maxTaskPassRateDrop,
      "per-task pass-rate regressions"
    ),
    comparisonCheck(
      "infrastructure-failures",
      candidateMetrics.infrastructureFailures <= baselineMetrics.infrastructureFailures,
      candidateMetrics.infrastructureFailures,
      baselineMetrics.infrastructureFailures,
      "candidate infrastructure failures"
    ),
    comparisonCheck(
      "tokens-per-success",
      tokenIncrease === null || tokenIncrease <= maxTokenIncreaseRatio,
      finiteRatio(tokenIncrease),
      maxTokenIncreaseRatio,
      "relative token increase"
    ),
    comparisonCheck(
      "average-latency",
      latencyIncrease === null || latencyIncrease <= maxLatencyIncreaseRatio,
      finiteRatio(latencyIncrease),
      maxLatencyIncreaseRatio,
      "relative elapsed-time increase"
    )
  ];
  return {
    schemaVersion: 1,
    suite: baseline.suite,
    runContract,
    baselineRunId: baseline.runId,
    candidateRunId: candidate.runId,
    status: checks.every((check) => check.passed) ? "passed" : "failed",
    thresholds: {
      maxPassRateDrop,
      maxTaskPassRateDrop,
      maxTokenIncreaseRatio,
      maxLatencyIncreaseRatio
    },
    baseline: baselineMetrics,
    candidate: candidateMetrics,
    deltas: {
      passRateDrop: finiteRatio(passRateDrop),
      tokenIncreaseRatio: finiteRatio(tokenIncrease),
      latencyIncreaseRatio: finiteRatio(latencyIncrease)
    },
    checks
  };
}

function renderNumber(value) {
  return value === null || value === undefined ? "n/a" : Number(value).toFixed(3);
}

export function renderComparisonMarkdown(comparison) {
  const lines = [
    `# Harness Comparison: ${comparison.suite?.title ?? comparison.suite?.id}`,
    "",
    `- Baseline: \`${comparison.baselineRunId}\``,
    `- Candidate: \`${comparison.candidateRunId}\``,
    `- Gate: **${comparison.status.toUpperCase()}**`,
    "",
    "| Metric | Baseline | Candidate |",
    "|---|---:|---:|",
    `| Pass rate | ${renderNumber(comparison.baseline.passRate)} | ${renderNumber(comparison.candidate.passRate)} |`,
    `| Tokens/success | ${renderNumber(comparison.baseline.tokensPerSuccess)} | ${renderNumber(comparison.candidate.tokensPerSuccess)} |`,
    `| Average elapsed ms | ${renderNumber(comparison.baseline.averageElapsedMs)} | ${renderNumber(comparison.candidate.averageElapsedMs)} |`,
    `| Average tool calls | ${renderNumber(comparison.baseline.averageToolCalls)} | ${renderNumber(comparison.candidate.averageToolCalls)} |`,
    `| Infrastructure failures | ${comparison.baseline.infrastructureFailures} | ${comparison.candidate.infrastructureFailures} |`,
    "",
    "| Gate | Passed | Actual | Limit |",
    "|---|---:|---:|---:|"
  ];
  for (const check of comparison.checks) {
    const actual = Array.isArray(check.actual) ? JSON.stringify(check.actual) : renderNumber(check.actual);
    lines.push(`| ${check.id} | ${check.passed ? "yes" : "no"} | ${actual} | ${renderNumber(check.limit)} |`);
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
