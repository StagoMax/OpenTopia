#!/usr/bin/env node

import { mkdir, readdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";

function optionValue(argv, name, fallback = "") {
  const index = argv.indexOf(name);
  return index >= 0 && argv[index + 1] ? argv[index + 1] : fallback;
}

async function loadJson(path) {
  return JSON.parse(await readFile(path, "utf8"));
}

async function findHarnessSummary(directory) {
  const entries = await readdir(directory, { recursive: true, withFileTypes: true });
  const matches = entries
    .filter((entry) => entry.isFile() && entry.name === "summary.json")
    .map((entry) => join(entry.parentPath, entry.name));
  if (matches.length !== 1) {
    throw new Error(`expected exactly one harness summary below ${directory}; found ${matches.length}`);
  }
  return matches[0];
}

function numeric(value) {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function median(sorted) {
  if (sorted.length === 0) return null;
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
}

function percentile(sorted, fraction) {
  if (sorted.length === 0) return null;
  return sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * fraction) - 1)];
}

function distribution(values) {
  const valid = values.filter((value) => numeric(value) !== null).sort((left, right) => left - right);
  if (valid.length === 0) return { count: 0, sum: null, mean: null, median: null, p95: null };
  const sum = valid.reduce((total, value) => total + value, 0);
  return {
    count: valid.length,
    sum,
    mean: sum / valid.length,
    median: median(valid),
    p95: percentile(valid, 0.95),
  };
}

function addNumericFields(target, source, fields) {
  for (const field of fields) target[field] += numeric(source?.[field]) ?? 0;
}

function variantAggregate(records, providerSummaries) {
  const aggregate = {
    trials: records.length,
    passedTrials: 0,
    passRate: null,
    categories: {},
    failureCategories: {},
    elapsedMs: distribution(records.map((record) => record.elapsedMs)),
    toolCalls: distribution(records.map((record) => record.metrics?.toolCalls)),
    modelRequests: distribution(records.map((record) => record.metrics?.usage?.modelRequests)),
    providerTotalTokens: distribution(records.map((record) => record.metrics?.usage?.providerTotalTokens)),
    uncachedTokens: distribution(records.map((record) => {
      const usage = record.metrics?.usage;
      const uncached = numeric(usage?.uncachedInputTokens);
      const output = numeric(usage?.outputTokens);
      return uncached === null || output === null ? null : uncached + output;
    })),
    usage: {
      inputTokens: 0,
      outputTokens: 0,
      providerTotalTokens: 0,
      cachedInputTokens: 0,
      uncachedInputTokens: 0,
      estimatedRetryInputTokens: 0,
      compatibilityRetryCount: 0,
      invalidToolLoopCount: 0,
      finalizationGuardRejectCount: 0,
      noProgressSignalCount: 0,
      duplicatePlanCount: 0,
      compactionRequests: 0,
      compactionTokens: 0,
      inputTokenBreakdown: {},
    },
    providerRaw: {
      providerRequests: 0,
      requestsWithUsage: 0,
      providerFailures: 0,
      inputTokens: 0,
      outputTokens: 0,
      totalTokens: 0,
      cachedInputTokens: 0,
      uncachedInputTokens: null,
      estimatedCostUsd: null,
      cacheTelemetryCoverage: null,
    },
  };
  for (const record of records) {
    if (record.status === "passed") aggregate.passedTrials += 1;
    if (record.failureCategory) {
      aggregate.failureCategories[record.failureCategory] =
        (aggregate.failureCategories[record.failureCategory] ?? 0) + 1;
    }
    for (const [category, passed] of Object.entries(record.scores ?? {})) {
      if (typeof passed !== "boolean") continue;
      const summary = aggregate.categories[category] ?? { passed: 0, total: 0, rate: null };
      summary.total += 1;
      if (passed) summary.passed += 1;
      summary.rate = summary.passed / summary.total;
      aggregate.categories[category] = summary;
    }
    const usage = record.metrics?.usage ?? {};
    addNumericFields(aggregate.usage, usage, [
      "inputTokens",
      "outputTokens",
      "providerTotalTokens",
      "cachedInputTokens",
      "uncachedInputTokens",
      "estimatedRetryInputTokens",
      "compatibilityRetryCount",
      "invalidToolLoopCount",
      "finalizationGuardRejectCount",
      "noProgressSignalCount",
      "duplicatePlanCount",
      "compactionRequests",
      "compactionTokens",
    ]);
    for (const [field, value] of Object.entries(usage.inputTokenBreakdown ?? {})) {
      aggregate.usage.inputTokenBreakdown[field] =
        (aggregate.usage.inputTokenBreakdown[field] ?? 0) + (numeric(value) ?? 0);
    }
  }
  aggregate.passRate = aggregate.trials === 0 ? null : aggregate.passedTrials / aggregate.trials;

  const raw = providerSummaries.map((summary) => summary.aggregate).filter(Boolean);
  for (const summary of raw) {
    addNumericFields(aggregate.providerRaw, summary, [
      "providerRequests",
      "requestsWithUsage",
      "providerFailures",
      "inputTokens",
      "outputTokens",
      "totalTokens",
      "cachedInputTokens",
    ]);
  }
  if (raw.length > 0 && raw.every((summary) => numeric(summary.uncachedInputTokens) !== null)) {
    aggregate.providerRaw.uncachedInputTokens = raw.reduce(
      (total, summary) => total + summary.uncachedInputTokens,
      0,
    );
  }
  if (raw.length > 0 && raw.every((summary) => numeric(summary.estimatedCostUsd) !== null)) {
    aggregate.providerRaw.estimatedCostUsd = raw.reduce(
      (total, summary) => total + summary.estimatedCostUsd,
      0,
    );
  }
  if (aggregate.providerRaw.requestsWithUsage > 0) {
    aggregate.providerRaw.cacheTelemetryCoverage =
      raw.reduce((total, summary) => total + (summary.cacheTelemetryRequests ?? 0), 0) /
      aggregate.providerRaw.requestsWithUsage;
  }
  return aggregate;
}

function difference(after, before) {
  if (numeric(after) === null || numeric(before) === null) return { absolute: null, relative: null };
  return { absolute: after - before, relative: before === 0 ? null : (after - before) / before };
}

function formatNumber(value, digits = 2) {
  return numeric(value) === null ? "—" : value.toLocaleString("en-US", { maximumFractionDigits: digits });
}

function formatPercent(value) {
  return numeric(value) === null ? "—" : `${(value * 100).toFixed(1)}%`;
}

const experimentPath = optionValue(process.argv, "--experiment");
const outputDirectory = optionValue(process.argv, "--output-dir");
if (!experimentPath || !outputDirectory) {
  throw new Error("usage: build-paired-internal-report.mjs --experiment <experiment.json> --output-dir <dir>");
}

const experiment = await loadJson(experimentPath);
const recordsByVariant = { before: [], after: [] };
const rawUsageByVariant = { before: [], after: [] };
const infrastructureRuns = [];
for (const run of experiment.runs ?? []) {
  const variant = run.variant;
  const wrapper = await loadJson(run.summaryPath);
  let harnessSummary;
  try {
    harnessSummary = await loadJson(await findHarnessSummary(wrapper.harness.outputDirectory));
  } catch (error) {
    infrastructureRuns.push({ runId: run.runId, variant, error: String(error.message ?? error), wrapper });
    continue;
  }
  for (const result of harnessSummary.results ?? []) {
    recordsByVariant[variant].push({ ...result, runId: run.runId, suiteId: harnessSummary.suite?.id ?? null });
  }
  rawUsageByVariant[variant].push(await loadJson(run.providerUsageSummaryPath));
}
const before = variantAggregate(recordsByVariant.before, rawUsageByVariant.before);
const after = variantAggregate(recordsByVariant.after, rawUsageByVariant.after);
const comparison = {
  passRate: difference(after.passRate, before.passRate),
  medianElapsedMs: difference(after.elapsedMs.median, before.elapsedMs.median),
  meanProviderTokens: difference(after.providerTotalTokens.mean, before.providerTotalTokens.mean),
  medianUncachedTokens: difference(after.uncachedTokens.median, before.uncachedTokens.median),
  rawEstimatedCostUsd: difference(after.providerRaw.estimatedCostUsd, before.providerRaw.estimatedCostUsd),
  rawCachedInputTokens: difference(after.providerRaw.cachedInputTokens, before.providerRaw.cachedInputTokens),
  rawUncachedInputTokens: difference(after.providerRaw.uncachedInputTokens, before.providerRaw.uncachedInputTokens),
};
const report = {
  schemaVersion: 1,
  experiment,
  recordsByVariant,
  infrastructureRuns,
  aggregate: { before, after },
  comparison,
};
const out = resolve(outputDirectory);
await mkdir(out, { recursive: true });
await writeFile(join(out, "paired-report.json"), `${JSON.stringify(report, null, 2)}\n`, "utf8");
const markdown = `# OpenTopia Before / After Internal Evaluation\n\n` +
  `- Experiment: \`${experiment.experimentId}\`\n` +
  `- Scope: ${experiment.scope}\n` +
  `- Model: \`${experiment.provider.model}\`\n` +
  `- Repetitions: ${experiment.repetitions}\n` +
  `- Internal Agent tasks: ${experiment.taskCount}\n\n` +
  `| Metric | Before | After | Δ |\n| --- | ---: | ---: | ---: |\n` +
  `| Pass rate | ${formatPercent(before.passRate)} | ${formatPercent(after.passRate)} | ${formatPercent(comparison.passRate.absolute)} |\n` +
  `| Median elapsed time | ${formatNumber(before.elapsedMs.median / 1000)}s | ${formatNumber(after.elapsedMs.median / 1000)}s | ${formatPercent(comparison.medianElapsedMs.relative)} |\n` +
  `| Mean provider tokens / trial | ${formatNumber(before.providerTotalTokens.mean)} | ${formatNumber(after.providerTotalTokens.mean)} | ${formatPercent(comparison.meanProviderTokens.relative)} |\n` +
  `| Median uncached+output tokens / trial | ${formatNumber(before.uncachedTokens.median)} | ${formatNumber(after.uncachedTokens.median)} | ${formatPercent(comparison.medianUncachedTokens.relative)} |\n` +
  `| Raw DeepSeek estimated cost | $${formatNumber(before.providerRaw.estimatedCostUsd, 4)} | $${formatNumber(after.providerRaw.estimatedCostUsd, 4)} | ${formatPercent(comparison.rawEstimatedCostUsd.relative)} |\n` +
  `| Raw cached input tokens | ${formatNumber(before.providerRaw.cachedInputTokens)} | ${formatNumber(after.providerRaw.cachedInputTokens)} | ${formatPercent(comparison.rawCachedInputTokens.relative)} |\n` +
  `| Raw uncached input tokens | ${formatNumber(before.providerRaw.uncachedInputTokens)} | ${formatNumber(after.providerRaw.uncachedInputTokens)} | ${formatPercent(comparison.rawUncachedInputTokens.relative)} |\n\n` +
  `Provider raw-use records are captured through a transparent local proxy and are independent of product telemetry parsing.\n`;
await writeFile(join(out, "paired-report.md"), markdown, "utf8");
console.log(`PAIRED_REPORT_JSON=${join(out, "paired-report.json")}`);
console.log(`PAIRED_REPORT_MD=${join(out, "paired-report.md")}`);
