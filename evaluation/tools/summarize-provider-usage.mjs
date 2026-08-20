#!/usr/bin/env node

import { readFile, writeFile } from "node:fs/promises";

function optionValue(argv, name, fallback = "") {
  const index = argv.indexOf(name);
  return index >= 0 && argv[index + 1] ? argv[index + 1] : fallback;
}

function numeric(value) {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function maximum(values) {
  const valid = values.filter((value) => value !== null);
  return valid.length === 0 ? null : Math.max(...valid);
}

function parseJsonLines(text) {
  return text
    .split(/\r?\n/)
    .filter(Boolean)
    .map((line, index) => {
      try {
        return JSON.parse(line);
      } catch (error) {
        throw new Error(`invalid JSONL at line ${index + 1}: ${error.message}`);
      }
    });
}

function requestUsage(record) {
  const usages = Array.isArray(record.usages) ? record.usages : [];
  // Streaming providers may emit cumulative usage in more than one event. The
  // largest total for a single HTTP request is the billed total, not a sum.
  const inputTokens = maximum(usages.map((usage) => numeric(usage.inputTokens)));
  const outputTokens = maximum(usages.map((usage) => numeric(usage.outputTokens)));
  const totalTokens = maximum(usages.map((usage) => numeric(usage.totalTokens)));
  const cachedInputTokens = maximum(
    usages.map((usage) => numeric(usage.cachedInputTokens)),
  );
  const cacheWriteTokens = maximum(usages.map((usage) => numeric(usage.cacheWriteTokens)));
  const reasoningTokens = maximum(usages.map((usage) => numeric(usage.reasoningTokens)));
  if (inputTokens === null && outputTokens === null && totalTokens === null) return null;
  return {
    inputTokens: inputTokens ?? 0,
    outputTokens: outputTokens ?? 0,
    totalTokens: totalTokens ?? (inputTokens ?? 0) + (outputTokens ?? 0),
    cachedInputTokens,
    cacheWriteTokens,
    reasoningTokens,
  };
}

function perMillionToCost(tokens, price) {
  return tokens === null || price === null ? null : (tokens / 1_000_000) * price;
}

const inputPath = optionValue(process.argv, "--input");
const outputPath = optionValue(process.argv, "--output");
const inputPrice = Number(optionValue(process.argv, "--input-price-per-million", ""));
const cacheHitPrice = Number(optionValue(process.argv, "--cache-hit-price-per-million", ""));
const outputPrice = Number(optionValue(process.argv, "--output-price-per-million", ""));
if (!inputPath || !outputPath) {
  throw new Error("usage: summarize-provider-usage.mjs --input <jsonl> --output <json>");
}
const hasPrice = [inputPrice, cacheHitPrice, outputPrice].every(Number.isFinite);
const records = parseJsonLines(await readFile(inputPath, "utf8"));
const aggregate = {
  providerRequests: 0,
  requestsWithUsage: 0,
  providerFailures: 0,
  inputTokens: 0,
  outputTokens: 0,
  totalTokens: 0,
  cachedInputTokens: 0,
  cacheWriteTokens: 0,
  reasoningTokens: 0,
  cacheTelemetryRequests: 0,
  cacheTelemetryCoverage: null,
  uncachedInputTokens: null,
  estimatedCostUsd: null,
};
for (const record of records) {
  if (record.path !== "/v1/chat/completions" && record.path !== "/chat/completions") continue;
  aggregate.providerRequests += 1;
  if (!Number.isInteger(record.upstreamStatus) || record.upstreamStatus >= 400) {
    aggregate.providerFailures += 1;
  }
  const usage = requestUsage(record);
  if (!usage) continue;
  aggregate.requestsWithUsage += 1;
  aggregate.inputTokens += usage.inputTokens;
  aggregate.outputTokens += usage.outputTokens;
  aggregate.totalTokens += usage.totalTokens;
  aggregate.reasoningTokens += usage.reasoningTokens ?? 0;
  aggregate.cacheWriteTokens += usage.cacheWriteTokens ?? 0;
  if (usage.cachedInputTokens !== null) {
    aggregate.cacheTelemetryRequests += 1;
    aggregate.cachedInputTokens += usage.cachedInputTokens;
  }
}
aggregate.cacheTelemetryCoverage =
  aggregate.requestsWithUsage === 0
    ? null
    : aggregate.cacheTelemetryRequests / aggregate.requestsWithUsage;
if (aggregate.cacheTelemetryRequests === aggregate.requestsWithUsage) {
  aggregate.uncachedInputTokens = aggregate.inputTokens - aggregate.cachedInputTokens;
  if (hasPrice) {
    aggregate.estimatedCostUsd =
      perMillionToCost(aggregate.uncachedInputTokens, inputPrice) +
      perMillionToCost(aggregate.cachedInputTokens, cacheHitPrice) +
      perMillionToCost(aggregate.outputTokens, outputPrice);
  }
}
await writeFile(
  outputPath,
  `${JSON.stringify(
    {
      schemaVersion: 1,
      source: "provider-usage-proxy",
      input: inputPath,
      pricing: hasPrice
        ? {
            currency: "USD",
            inputPricePerMillion: inputPrice,
            cacheHitPricePerMillion: cacheHitPrice,
            outputPricePerMillion: outputPrice,
          }
        : null,
      aggregate,
    },
    null,
    2,
  )}\n`,
  "utf8",
);
