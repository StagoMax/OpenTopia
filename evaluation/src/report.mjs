import { writeFile } from "node:fs/promises";
import path from "node:path";
import { ensureDirectory } from "./utils.mjs";
import { isAbilityEligible } from "./failures.mjs";

function percentage(numerator, denominator) {
  if (denominator === 0) return "n/a";
  return `${numerator}/${denominator} (${((numerator / denominator) * 100).toFixed(1)}%)`;
}

export function aggregateResults({
  runId,
  suite,
  target,
  manifest,
  results,
  startedAt,
  completedAt,
}) {
  const validResults = results.filter(isAbilityEligible);
  const passedResults = validResults.filter(
    (result) => result.status === "passed",
  );
  const taskMap = new Map();
  for (const result of results) {
    const entries = taskMap.get(result.taskId) ?? [];
    entries.push(result);
    taskMap.set(result.taskId, entries);
  }

  const tasks = [...taskMap.entries()].map(([taskId, trials]) => {
    const valid = trials.filter(isAbilityEligible);
    const passed = valid.filter((trial) => trial.status === "passed");
    return {
      taskId,
      trials: trials.length,
      validTrials: valid.length,
      passedTrials: passed.length,
      passRate: valid.length === 0 ? null : passed.length / valid.length,
      consistent: valid.length > 0 && passed.length === valid.length,
      statuses: trials.map((trial) => trial.status),
    };
  });

  const usage = results.reduce(
    (sum, result) => {
      const current = result.metrics?.usage ?? {};
      for (const field of [
        "requests",
        "modelRequests",
        "inputTokens",
        "outputTokens",
        "providerTotalTokens",
        "reasoningTokens",
        "cachedInputTokens",
        "cacheWriteTokens",
        "uncachedInputTokens",
        "logicalTotalTokens",
        "cacheTelemetryRequests",
        "localInputEstimate",
        "rawLocalInputEstimate",
        "estimatedRetryInputTokens",
        "compatibilityRetryCount",
        "invalidToolLoopCount",
        "finalizationGuardRejectCount",
        "noProgressSignalCount",
        "duplicatePlanCount",
        "compactionRequests",
        "compactionTokens",
      ])
        sum[field] += current[field] ?? 0;
      if (Number.isFinite(current.estimatedCost)) {
        sum.estimatedCost += current.estimatedCost;
        sum.costTelemetryResults += 1;
      }
      if (current.costCurrency) sum.costCurrencies.add(current.costCurrency);
      if (current.costSource) sum.costSources.add(current.costSource);
      for (const [key, value] of Object.entries(
        current.inputTokenBreakdown ?? {},
      )) {
        sum.inputTokenBreakdown[key] =
          (sum.inputTokenBreakdown[key] ?? 0) + (value ?? 0);
      }
      if (Number.isFinite(current.estimateErrorMean)) {
        const weight = current.requests ?? 0;
        sum.estimateErrorWeighted += current.estimateErrorMean * weight;
        sum.estimateErrorSamples += weight;
      }
      if (Number.isFinite(current.estimateErrorP95)) {
        sum.estimateErrorP95 = Math.max(
          sum.estimateErrorP95 ?? 0,
          current.estimateErrorP95,
        );
      }
      if (Number.isFinite(current.rawEstimateErrorMean)) {
        const weight = current.requests ?? 0;
        sum.rawEstimateErrorWeighted += current.rawEstimateErrorMean * weight;
        sum.rawEstimateErrorSamples += weight;
      }
      if (Number.isFinite(current.rawEstimateErrorP95)) {
        sum.rawEstimateErrorP95 = Math.max(
          sum.rawEstimateErrorP95 ?? 0,
          current.rawEstimateErrorP95,
        );
      }
      return sum;
    },
    {
      requests: 0,
      modelRequests: 0,
      inputTokens: 0,
      outputTokens: 0,
      providerTotalTokens: 0,
      reasoningTokens: 0,
      cachedInputTokens: 0,
      cacheWriteTokens: 0,
      uncachedInputTokens: 0,
      logicalTotalTokens: 0,
      cacheTelemetryRequests: 0,
      localInputEstimate: 0,
      rawLocalInputEstimate: 0,
      estimatedRetryInputTokens: 0,
      compatibilityRetryCount: 0,
      invalidToolLoopCount: 0,
      finalizationGuardRejectCount: 0,
      noProgressSignalCount: 0,
      duplicatePlanCount: 0,
      compactionRequests: 0,
      compactionTokens: 0,
      inputTokenBreakdown: {},
      estimateErrorWeighted: 0,
      estimateErrorSamples: 0,
      estimateErrorP95: null,
      rawEstimateErrorWeighted: 0,
      rawEstimateErrorSamples: 0,
      rawEstimateErrorP95: null,
      estimatedCost: 0,
      costTelemetryResults: 0,
      costCurrencies: new Set(),
      costSources: new Set(),
    },
  );
  usage.cachedInputRatio =
    usage.inputTokens === 0
      ? null
      : usage.cachedInputTokens / usage.inputTokens;
  usage.cacheTelemetryCoverage =
    usage.requests === 0 ? null : usage.cacheTelemetryRequests / usage.requests;
  usage.providerUsageCoverage =
    usage.modelRequests === 0 ? null : usage.requests / usage.modelRequests;
  usage.estimateErrorMean =
    usage.estimateErrorSamples === 0
      ? null
      : usage.estimateErrorWeighted / usage.estimateErrorSamples;
  usage.rawEstimateErrorMean =
    usage.rawEstimateErrorSamples === 0
      ? null
      : usage.rawEstimateErrorWeighted / usage.rawEstimateErrorSamples;
  usage.estimateCalibrationFactor =
    usage.rawLocalInputEstimate === 0
      ? null
      : usage.localInputEstimate / usage.rawLocalInputEstimate;
  usage.tokensPerSuccess =
    passedResults.length === 0
      ? null
      : usage.providerTotalTokens / passedResults.length;
  usage.uncachedTokensPerSuccess =
    passedResults.length === 0
      ? null
      : (usage.uncachedInputTokens + usage.outputTokens) / passedResults.length;
  usage.costTelemetryCoverage =
    validResults.length === 0
      ? null
      : usage.costTelemetryResults / validResults.length;
  usage.costPerSuccess =
    passedResults.length === 0 ||
    usage.costTelemetryResults !== validResults.length
      ? null
      : usage.estimatedCost / passedResults.length;
  usage.costCurrency =
    usage.costCurrencies.size === 1 ? [...usage.costCurrencies][0] : null;
  usage.costSource =
    usage.costSources.size === 1 ? [...usage.costSources][0] : null;
  if (usage.costTelemetryResults === 0) usage.estimatedCost = null;
  delete usage.costCurrencies;
  delete usage.costSources;
  delete usage.estimateErrorWeighted;
  delete usage.estimateErrorSamples;
  delete usage.rawEstimateErrorWeighted;
  delete usage.rawEstimateErrorSamples;

  const categoryPass = {};
  for (const category of ["outcome", "trajectory", "safety", "efficiency"]) {
    const categoryPassed = validResults.filter(
      (result) => result.scores?.[category] === true,
    ).length;
    categoryPass[category] = {
      passed: categoryPassed,
      total: validResults.length,
      rate:
        validResults.length === 0 ? null : categoryPassed / validResults.length,
    };
  }

  const failureCategories = {};
  for (const result of results) {
    const category = result.failureCategory ?? result.failure?.category;
    if (category) failureCategories[category] = (failureCategories[category] ?? 0) + 1;
  }

  return {
    schemaVersion: 1,
    runId,
    suite: { id: suite.id, title: suite.title },
    target: { id: target.id, description: target.description ?? null },
    startedAt,
    completedAt,
    status:
      results.length > 0 && passedResults.length === results.length
        ? "passed"
        : "failed",
    aggregate: {
      requestedTrials: results.length,
      validTrials: validResults.length,
      passedTrials: passedResults.length,
      passRate:
        validResults.length === 0
          ? null
          : passedResults.length / validResults.length,
      infrastructureFailures: results.length - validResults.length,
      failureCategories,
      categoryPass,
      usage,
    },
    tasks,
    results,
    manifest,
  };
}

function number(value, digits = 3) {
  return value === null || value === undefined
    ? "n/a"
    : Number(value).toFixed(digits);
}

export function renderMarkdown(summary) {
  const lines = [
    `# Evaluation Report: ${summary.suite.title}`,
    "",
    `- Run: \`${summary.runId}\``,
    `- Target: \`${summary.target.id}\``,
    `- Status: **${summary.status.toUpperCase()}**`,
    `- Started: ${summary.startedAt}`,
    `- Completed: ${summary.completedAt}`,
    "",
    "## Result",
    "",
    `- Strict success: ${percentage(summary.aggregate.passedTrials, summary.aggregate.validTrials)}`,
    `- Infrastructure/grader exclusions: ${summary.aggregate.infrastructureFailures}`,
    `- Provider-reported tokens: ${summary.aggregate.usage.providerTotalTokens}`,
    `- Cached input ratio: ${number(summary.aggregate.usage.cachedInputRatio)}`,
    `- Cache telemetry coverage: ${number(summary.aggregate.usage.cacheTelemetryCoverage)}`,
    `- Provider usage coverage: ${number(summary.aggregate.usage.providerUsageCoverage)}`,
    `- Estimate error P95: ${number(summary.aggregate.usage.estimateErrorP95)}`,
    `- Raw estimate error P95: ${number(summary.aggregate.usage.rawEstimateErrorP95)}`,
    `- Tokens per success: ${number(summary.aggregate.usage.tokensPerSuccess)}`,
    `- Uncached tokens per success: ${number(summary.aggregate.usage.uncachedTokensPerSuccess)}`,
    `- Cost per success: ${number(summary.aggregate.usage.costPerSuccess)}${summary.aggregate.usage.costCurrency ? ` ${summary.aggregate.usage.costCurrency}` : ""}`,
    "",
    "| Task | Passed | Valid | Consistent | Statuses |",
    "|---|---:|---:|---:|---|",
  ];
  for (const task of summary.tasks) {
    lines.push(
      `| ${task.taskId} | ${task.passedTrials} | ${task.validTrials} | ${task.consistent ? "yes" : "no"} | ${task.statuses.join(", ")} |`,
    );
  }

  lines.push(
    "",
    "## Category Scores",
    "",
    "| Category | Passed | Total | Rate |",
    "|---|---:|---:|---:|",
  );
  for (const [category, value] of Object.entries(
    summary.aggregate.categoryPass,
  )) {
    lines.push(
      `| ${category} | ${value.passed} | ${value.total} | ${number(value.rate)} |`,
    );
  }

  lines.push("", "## Failure Classification", "");
  const failureCategories = Object.entries(summary.aggregate.failureCategories ?? {});
  if (failureCategories.length === 0) {
    lines.push("None.");
  } else {
    lines.push("| Category | Trials |", "|---|---:|");
    for (const [category, count] of failureCategories) {
      lines.push(`| ${category} | ${count} |`);
    }
  }

  const failures = summary.results.flatMap((result) =>
    result.checks
      .filter((item) => !item.passed)
      .map((item) => ({
        trialId: result.trialId,
        taskId: result.taskId,
        ...item,
      })),
  );
  lines.push("", "## Failed Checks", "");
  if (failures.length === 0) {
    lines.push("None.");
  } else {
    lines.push(
      "| Trial | Task | Check | Category | Hard |",
      "|---|---|---|---|---:|",
    );
    for (const failure of failures) {
      lines.push(
        `| ${failure.trialId} | ${failure.taskId} | ${failure.id} | ${failure.category} | ${failure.hard ? "yes" : "no"} |`,
      );
    }
  }
  lines.push("");
  return `${lines.join("\n")}\n`;
}

export async function writeReport(runDirectory, summary) {
  await ensureDirectory(runDirectory);
  const jsonPath = path.join(runDirectory, "summary.json");
  const markdownPath = path.join(runDirectory, "report.md");
  await writeFile(jsonPath, `${JSON.stringify(summary, null, 2)}\n`, "utf8");
  await writeFile(markdownPath, renderMarkdown(summary), "utf8");
  return { jsonPath, markdownPath };
}
