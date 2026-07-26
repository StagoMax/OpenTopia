import { writeFile } from "node:fs/promises";
import path from "node:path";
import { ensureDirectory } from "./utils.mjs";

const EXCLUDED_FROM_ABILITY_DENOMINATOR = new Set(["infra_error", "grader_error", "invalid_task"]);

function percentage(numerator, denominator) {
  if (denominator === 0) return "n/a";
  return `${numerator}/${denominator} (${(numerator / denominator * 100).toFixed(1)}%)`;
}

export function aggregateResults({ runId, suite, target, manifest, results, startedAt, completedAt }) {
  const validResults = results.filter((result) => !EXCLUDED_FROM_ABILITY_DENOMINATOR.has(result.status));
  const passedResults = validResults.filter((result) => result.status === "passed");
  const taskMap = new Map();
  for (const result of results) {
    const entries = taskMap.get(result.taskId) ?? [];
    entries.push(result);
    taskMap.set(result.taskId, entries);
  }

  const tasks = [...taskMap.entries()].map(([taskId, trials]) => {
    const valid = trials.filter((trial) => !EXCLUDED_FROM_ABILITY_DENOMINATOR.has(trial.status));
    const passed = valid.filter((trial) => trial.status === "passed");
    return {
      taskId,
      trials: trials.length,
      validTrials: valid.length,
      passedTrials: passed.length,
      passRate: valid.length === 0 ? null : passed.length / valid.length,
      consistent: valid.length > 0 && passed.length === valid.length,
      statuses: trials.map((trial) => trial.status)
    };
  });

  const usage = results.reduce((sum, result) => {
    const current = result.metrics?.usage ?? {};
    for (const field of [
      "requests",
      "inputTokens",
      "outputTokens",
      "providerTotalTokens",
      "reasoningTokens",
      "cachedInputTokens",
      "cacheWriteTokens",
      "uncachedInputTokens",
      "cacheTelemetryRequests"
    ]) sum[field] += current[field] ?? 0;
    return sum;
  }, {
    requests: 0,
    inputTokens: 0,
    outputTokens: 0,
    providerTotalTokens: 0,
    reasoningTokens: 0,
    cachedInputTokens: 0,
    cacheWriteTokens: 0,
    uncachedInputTokens: 0,
    cacheTelemetryRequests: 0
  });
  usage.cachedInputRatio = usage.inputTokens === 0 ? null : usage.cachedInputTokens / usage.inputTokens;
  usage.cacheTelemetryCoverage = usage.requests === 0 ? null : usage.cacheTelemetryRequests / usage.requests;
  usage.tokensPerSuccess = passedResults.length === 0 ? null : usage.providerTotalTokens / passedResults.length;
  usage.uncachedTokensPerSuccess = passedResults.length === 0
    ? null
    : (usage.uncachedInputTokens + usage.outputTokens) / passedResults.length;

  const categoryPass = {};
  for (const category of ["outcome", "trajectory", "safety", "efficiency"]) {
    const categoryPassed = validResults.filter((result) => result.scores?.[category] === true).length;
    categoryPass[category] = {
      passed: categoryPassed,
      total: validResults.length,
      rate: validResults.length === 0 ? null : categoryPassed / validResults.length
    };
  }

  return {
    schemaVersion: 1,
    runId,
    suite: { id: suite.id, title: suite.title },
    target: { id: target.id, description: target.description ?? null },
    startedAt,
    completedAt,
    status: results.length > 0 && passedResults.length === results.length ? "passed" : "failed",
    aggregate: {
      requestedTrials: results.length,
      validTrials: validResults.length,
      passedTrials: passedResults.length,
      passRate: validResults.length === 0 ? null : passedResults.length / validResults.length,
      infrastructureFailures: results.length - validResults.length,
      categoryPass,
      usage
    },
    tasks,
    results,
    manifest
  };
}

function number(value, digits = 3) {
  return value === null || value === undefined ? "n/a" : Number(value).toFixed(digits);
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
    "",
    "| Task | Passed | Valid | Consistent | Statuses |",
    "|---|---:|---:|---:|---|"
  ];
  for (const task of summary.tasks) {
    lines.push(`| ${task.taskId} | ${task.passedTrials} | ${task.validTrials} | ${task.consistent ? "yes" : "no"} | ${task.statuses.join(", ")} |`);
  }

  lines.push(
    "",
    "## Category Scores",
    "",
    "| Category | Passed | Total | Rate |",
    "|---|---:|---:|---:|"
  );
  for (const [category, value] of Object.entries(summary.aggregate.categoryPass)) {
    lines.push(`| ${category} | ${value.passed} | ${value.total} | ${number(value.rate)} |`);
  }

  const failures = summary.results.flatMap((result) => result.checks
    .filter((item) => !item.passed)
    .map((item) => ({ trialId: result.trialId, taskId: result.taskId, ...item })));
  lines.push("", "## Failed Checks", "");
  if (failures.length === 0) {
    lines.push("None.");
  } else {
    lines.push("| Trial | Task | Check | Category | Hard |", "|---|---|---|---|---:|");
    for (const failure of failures) {
      lines.push(`| ${failure.trialId} | ${failure.taskId} | ${failure.id} | ${failure.category} | ${failure.hard ? "yes" : "no"} |`);
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
