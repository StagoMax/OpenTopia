#!/usr/bin/env node

import process from "node:process";
import path from "node:path";
import { runSuite, validateDefinitions } from "./runner.mjs";

function help() {
  return `Application Agent Evaluation Harness

Usage:
  agent-eval validate --suite <suite.json> --target <target.json>
  agent-eval run --suite <suite.json> --target <target.json> [--output <dir>] [--repetitions <n>]

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
  const result = await runSuite({
    suitePath: options.suite,
    targetPath: options.target,
    outputDirectory,
    repetitions
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
