#!/usr/bin/env node

import process from "node:process";
import path from "node:path";
import { writeEvaluationCatalog } from "./catalog.mjs";
import { compareSummaries, writeComparison } from "./compare.mjs";
import { writeRegressionReport } from "./regressions.mjs";
import { runSuite, validateDefinitions } from "./runner.mjs";
import { loadJson } from "./validation.mjs";

function help() {
  return `Application Agent Evaluation Harness

Usage:
  agent-eval validate --suite <suite.json> --target <target.json>
  agent-eval run --suite <suite.json> --target <target.json> [--output <dir>] [--repetitions <n>] [--tasks <id,id,...>] [--experiment <profile.json>]
  agent-eval compare --baseline <summary.json> --candidate <summary.json> [--output <dir>]
  agent-eval catalog [--root <evaluations-dir>] [--output <catalog.md>]
  agent-eval regressions [--registry <registry.json>] [--summaries <summary.json,...>] [--output <report.md>]

The target is a black-box process adapter. The harness passes the prompt through
stdin or a file, exposes trial paths through AGENT_EVAL_* environment variables,
and consumes normalized JSONL events without importing product code.
`;
}

function parseArguments(argv) {
  const [command, ...rest] = argv;
  const options = {};
  for (let index = 0; index < rest.length; index += 1) {
    const argument = rest[index];
    if (!argument.startsWith("--")) throw new Error(`Unexpected argument ${argument}`);
    const key = argument.slice(2);
    const value = rest[index + 1];
    if (!value || value.startsWith("--")) throw new Error(`Missing value for ${argument}`);
    options[key] = value;
    index += 1;
  }
  return { command, options };
}

async function main() {
  const { command, options } = parseArguments(process.argv.slice(2));
  if (!command || command === "help" || command === "--help") {
    process.stdout.write(help());
    return;
  }
  if (command === "catalog") {
    const rootDirectory = path.resolve(
      options.root ?? path.join(process.cwd(), ".opentopia", "evaluations")
    );
    const outputPath = path.resolve(options.output ?? path.join(rootDirectory, "index.md"));
    const catalog = await writeEvaluationCatalog(rootDirectory, outputPath);
    process.stdout.write(`${JSON.stringify({
      runs: catalog.runs.length,
      skipped: catalog.warnings.length,
      catalog: catalog.outputPath
    }, null, 2)}\n`);
    return;
  }
  if (command === "regressions") {
    const registryPath = path.resolve(
      options.registry ?? path.join(process.cwd(), "evaluation", "regressions", "registry.json")
    );
    const outputPath = path.resolve(
      options.output ?? path.join(path.dirname(registryPath), "index.md")
    );
    const summaryValue = options.summaries ?? options.summary ?? "";
    const summaryPaths = summaryValue
      .split(",")
      .map((value) => value.trim())
      .filter(Boolean);
    const result = await writeRegressionReport({ registryPath, summaryPaths, outputPath });
    process.stdout.write(`${JSON.stringify({
      cases: result.report.aggregate.totalCases,
      open: result.report.aggregate.openCases,
      regressionCoverageRate: result.report.aggregate.regressionCoverageRate,
      passRate: result.report.aggregate.passRate,
      report: result.markdownPath,
      json: result.jsonPath
    }, null, 2)}\n`);
    return;
  }
  if (command === "compare") {
    if (!options.baseline || !options.candidate) {
      throw new Error("--baseline and --candidate are required for compare");
    }
    const numericOption = (name, fallback) => {
      if (options[name] === undefined) return fallback;
      const value = Number(options[name]);
      if (!Number.isFinite(value) || value < 0) throw new Error(`--${name} must be a non-negative number`);
      return value;
    };
    const comparison = compareSummaries(
      await loadJson(options.baseline),
      await loadJson(options.candidate),
      {
        maxPassRateDrop: numericOption("max-pass-rate-drop", 0),
        maxTaskPassRateDrop: numericOption("max-task-pass-rate-drop", 0),
        maxTokenIncreaseRatio: numericOption("max-token-increase-ratio", 0.2),
        maxLatencyIncreaseRatio: numericOption("max-latency-increase-ratio", 0.2)
      }
    );
    const outputDirectory = options.output ?? path.dirname(path.resolve(options.candidate));
    const reports = await writeComparison(outputDirectory, comparison);
    process.stdout.write(`${JSON.stringify({
      status: comparison.status,
      comparison: reports.jsonPath,
      report: reports.markdownPath
    }, null, 2)}\n`);
    if (comparison.status !== "passed") process.exitCode = 1;
    return;
  }
  if (!options.suite || !options.target) throw new Error("--suite and --target are required");
  if (command === "validate") {
    const definitions = await validateDefinitions(options.suite, options.target);
    process.stdout.write(`${JSON.stringify({
      valid: true,
      suite: definitions.suite.id,
      target: definitions.target.id,
      tasks: definitions.tasks.map((entry) => entry.task.id)
    }, null, 2)}\n`);
    return;
  }
  if (command !== "run") throw new Error(`Unknown command ${command}`);
  let repetitions;
  if (options.repetitions !== undefined) {
    repetitions = Number(options.repetitions);
    if (!Number.isInteger(repetitions) || repetitions < 1 || repetitions > 100) {
      throw new Error("--repetitions must be an integer between 1 and 100");
    }
  }
  const outputDirectory = options.output ?? path.join(process.cwd(), "evaluation", ".runs");
  const selectedTaskIds = options.tasks
    ? options.tasks.split(",").map((value) => value.trim()).filter(Boolean)
    : undefined;
  const result = await runSuite({
    suitePath: options.suite,
    targetPath: options.target,
    outputDirectory,
    repetitions,
    experimentPath: options.experiment,
    selectedTaskIds
  });
  process.stdout.write(`${JSON.stringify({
    status: result.summary.status,
    runId: result.summary.runId,
    passRate: result.summary.aggregate.passRate,
    runDirectory: result.runDirectory,
    report: result.reports.markdownPath
  }, null, 2)}\n`);
  if (result.summary.status !== "passed") process.exitCode = 1;
}

main().catch((error) => {
  process.stderr.write(`${error.stack ?? error.message}\n`);
  process.exitCode = 2;
});
