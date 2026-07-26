import { readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { makeHarnessEvent, readTargetEvents, writeEvents } from "./events.mjs";
import {
  gradeBudgets,
  gradeCommands,
  gradeFiles,
  gradeSecurity,
  gradeTrajectory,
  scoresFromChecks,
  summarizeDomainMetrics,
  summarizeUsage
} from "./graders.mjs";
import { runCommand } from "./process.mjs";
import { aggregateResults, writeReport } from "./report.mjs";
import {
  copyFixture,
  ensureDirectory,
  makeId,
  redact,
  replacePlaceholders,
  resolveInside,
  sha256File,
  snapshotPaths
} from "./utils.mjs";
import { loadJson, validateSuite, validateTarget, validateTask } from "./validation.mjs";

function resolveTargetExecutable(command, targetDirectory) {
  if (!command.includes("/") && !command.includes("\\")) return command;
  return resolveInside(targetDirectory, command, "target command");
}

function targetCwd(target, targetDirectory, variables) {
  if (!target.cwd) return variables.workspace;
  const replaced = replacePlaceholders(target.cwd, variables);
  return path.isAbsolute(replaced) ? replaced : resolveInside(targetDirectory, replaced, "target cwd");
}

async function loadDefinitions(suitePath, targetPath) {
  const suiteAbsolute = path.resolve(suitePath);
  const targetAbsolute = path.resolve(targetPath);
  const suite = validateSuite(await loadJson(suiteAbsolute));
  const target = validateTarget(await loadJson(targetAbsolute));
  const suiteDirectory = path.dirname(suiteAbsolute);
  const tasks = [];
  for (const taskReference of suite.tasks) {
    const taskPath = resolveInside(suiteDirectory, taskReference, "suite task");
    const task = validateTask(await loadJson(taskPath));
    if (task.suite !== suite.id) {
      throw new Error(`Task ${task.id} belongs to suite ${task.suite}, expected ${suite.id}`);
    }
    tasks.push({ task, taskPath, taskDirectory: path.dirname(taskPath) });
  }
  const duplicateIds = tasks.map((entry) => entry.task.id)
    .filter((id, index, values) => values.indexOf(id) !== index);
  if (duplicateIds.length > 0) throw new Error(`Suite contains duplicate task IDs: ${[...new Set(duplicateIds)].join(", ")}`);
  return {
    suite,
    suitePath: suiteAbsolute,
    suiteDirectory,
    target,
    targetPath: targetAbsolute,
    targetDirectory: path.dirname(targetAbsolute),
    tasks
  };
}

export async function validateDefinitions(suitePath, targetPath) {
  const definitions = await loadDefinitions(suitePath, targetPath);
  for (const entry of definitions.tasks) {
    if (entry.task.fixture) {
      resolveInside(entry.taskDirectory, entry.task.fixture.source, `fixture for ${entry.task.id}`, { allowBase: false });
    }
    for (const grader of entry.task.graders.commands ?? []) {
      if (grader.cwd) resolveInside(entry.taskDirectory, grader.cwd, `grader cwd for ${entry.task.id}`);
    }
  }
  return definitions;
}

function determineStatus({ targetResult, checks, scores, events }) {
  if (targetResult.timedOut) return "watchdog_timeout";
  if (targetResult.spawnError || events.some((event) => event.type === "application.adapter_error")) {
    return "runtime_dependency_error";
  }
  if (checks.some((item) => item.graderError)) return "grader_error";
  if (checks.some((item) => item.category === "safety" && item.hard && !item.passed)) return "safety_violation";
  if (targetResult.exitCode !== 0) return "application_crash";
  const completionClaimed = events.some((event) => event.type === "agent.completion.claimed");
  if (!scores.outcome && completionClaimed) return "false_completion";
  if (Object.values(scores).every(Boolean)) return "passed";
  return "task_failed";
}

async function runTrial({
  runId,
  runDirectory,
  taskEntry,
  target,
  targetDirectory,
  repetition
}) {
  const { task, taskDirectory } = taskEntry;
  const trialId = `${task.id.replace(/[^A-Za-z0-9_.-]+/g, "-")}_${repetition}`;
  const trialDirectory = path.join(runDirectory, "trials", trialId);
  const workspace = path.join(trialDirectory, "workspace");
  const controlDirectory = path.join(trialDirectory, "control");
  const promptFile = path.join(controlDirectory, "prompt.txt");
  const targetEventsFile = path.join(controlDirectory, "target-events.jsonl");
  const eventsFile = path.join(trialDirectory, "events.jsonl");
  await ensureDirectory(workspace);
  await ensureDirectory(controlDirectory);
  if (task.fixture) {
    const fixture = resolveInside(taskDirectory, task.fixture.source, `fixture for ${task.id}`, { allowBase: false });
    await copyFixture(fixture, workspace);
  }
  await writeFile(promptFile, task.prompt, "utf8");

  const securitySettings = task.graders.security ?? {};
  const protectedBefore = await snapshotPaths(workspace, securitySettings.protectedPaths ?? []);
  const startedAt = new Date().toISOString();
  const context = {
    runId,
    trialId,
    taskId: task.id,
    startedMonotonic: Date.now()
  };
  const variables = {
    runId,
    trialId,
    taskId: task.id,
    workspace,
    trialDir: trialDirectory,
    taskDir: taskDirectory,
    targetDir: targetDirectory,
    promptFile,
    eventsFile: targetEventsFile
  };
  const harnessEvents = [makeHarnessEvent(context, "application.launch.started", { targetId: target.id })];

  const targetEnvironment = {};
  for (const [key, value] of Object.entries(target.env ?? {})) {
    targetEnvironment[key] = replacePlaceholders(value, variables);
  }
  for (const key of target.passEnvironment ?? []) {
    if (process.env[key] === undefined) throw new Error(`Target requires missing environment variable ${key}`);
    targetEnvironment[key] = process.env[key];
  }
  Object.assign(targetEnvironment, {
    AGENT_EVAL_RUN_ID: runId,
    AGENT_EVAL_TRIAL_ID: trialId,
    AGENT_EVAL_TASK_ID: task.id,
    AGENT_EVAL_WORKSPACE: workspace,
    AGENT_EVAL_PROMPT_FILE: promptFile,
    AGENT_EVAL_EVENTS_PATH: targetEventsFile,
    AGENT_EVAL_TRIAL_DIR: trialDirectory,
    AGENT_EVAL_TARGET_ID: target.id
  });
  if (securitySettings.exposeCanariesAsEnvironment) {
    targetEnvironment.AGENT_EVAL_SECRET_CANARIES = JSON.stringify(securitySettings.secretCanaries ?? []);
  }

  const promptTransport = target.promptTransport ?? "stdin";
  const targetResult = await runCommand({
    command: resolveTargetExecutable(replacePlaceholders(target.command, variables), targetDirectory),
    args: (target.args ?? []).map((argument) => replacePlaceholders(argument, variables)),
    cwd: targetCwd(target, targetDirectory, variables),
    env: targetEnvironment,
    inheritEnvironment: target.inheritEnvironment ?? false,
    stdin: promptTransport === "stdin" ? task.prompt : undefined,
    timeoutMs: (task.budgets?.watchdogSeconds ?? 1800) * 1000
  });
  harnessEvents.push(makeHarnessEvent(context, "application.launch.completed", {
    exitCode: targetResult.exitCode,
    signal: targetResult.signal,
    timedOut: targetResult.timedOut,
    elapsedMs: targetResult.elapsedMs,
    spawnError: targetResult.spawnError
  }));

  let rawEventText = "";
  try {
    rawEventText = await readFile(targetEventsFile, "utf8");
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  const parsed = await readTargetEvents(targetEventsFile, context);
  const events = [...harnessEvents, ...parsed.events].sort((left, right) => {
    const byTime = Date.parse(left.timestamp) - Date.parse(right.timestamp);
    return byTime !== 0 ? byTime : (left.monotonicMs ?? 0) - (right.monotonicMs ?? 0);
  });

  const secrets = securitySettings.secretCanaries ?? [];
  const checks = [
    {
      id: "target.exit",
      category: "outcome",
      passed: targetResult.exitCode === 0 && !targetResult.timedOut && !targetResult.spawnError,
      hard: false,
      graderError: false,
      detail: {
        exitCode: targetResult.exitCode,
        timedOut: targetResult.timedOut,
        spawnError: targetResult.spawnError
      }
    }
  ];
  const applicationTerminalEvents = events.filter((event) => event.type === "application.turn.completed");
  if (applicationTerminalEvents.length > 0) {
    const terminal = applicationTerminalEvents.at(-1).payload;
    checks.push({
      id: "application.terminal-status",
      category: "outcome",
      passed: terminal.status === "succeeded",
      hard: false,
      graderError: false,
      detail: { status: terminal.status ?? "unknown" }
    });
  }
  checks.push(...await gradeFiles(workspace, task.graders.files ?? []));
  checks.push(...await gradeCommands({
    graders: task.graders.commands ?? [],
    taskDirectory,
    variables,
    secrets
  }));
  const trajectory = gradeTrajectory(events, parsed.errors, task);
  checks.push(...trajectory.checks);
  checks.push(...await gradeSecurity({
    workspace,
    settings: securitySettings,
    protectedBefore,
    events,
    stdout: targetResult.stdout,
    stderr: targetResult.stderr,
    rawEventText
  }));
  const usage = summarizeUsage(events);
  const budgets = gradeBudgets(task, events, usage);
  checks.push(...budgets.checks);

  const scores = scoresFromChecks(checks);
  const status = determineStatus({ targetResult, checks, scores, events });
  const domainMetrics = summarizeDomainMetrics(events);
  const completedAt = new Date().toISOString();
  const sanitizedEventsText = events
    .map((event) => redact(JSON.stringify(event), secrets))
    .join("\n");
  await writeFile(eventsFile, sanitizedEventsText ? `${sanitizedEventsText}\n` : "", "utf8");
  await rm(targetEventsFile, { force: true });
  await writeFile(path.join(trialDirectory, "stdout.log"), redact(targetResult.stdout, secrets), "utf8");
  await writeFile(path.join(trialDirectory, "stderr.log"), redact(targetResult.stderr, secrets), "utf8");

  const result = {
    schemaVersion: 1,
    runId,
    trialId,
    taskId: task.id,
    taskVersion: task.version ?? "1.0.0",
    targetId: target.id,
    repetition,
    status,
    startedAt,
    completedAt,
    elapsedMs: targetResult.elapsedMs,
    scores,
    checks,
    metrics: {
      usage: {
        ...usage,
        tokensPerSuccess: status === "passed" ? usage.providerTotalTokens : null,
        uncachedTokensPerSuccess: status === "passed"
          ? usage.uncachedInputTokens + usage.outputTokens
          : null
      },
      toolCalls: budgets.toolCalls,
      capabilities: trajectory.capabilityMetrics,
      ...domainMetrics,
      eventCount: events.length,
      invalidEventCount: parsed.errors.length
    },
    process: {
      exitCode: targetResult.exitCode,
      signal: targetResult.signal,
      timedOut: targetResult.timedOut,
      stdoutTruncated: targetResult.stdoutTruncated,
      stderrTruncated: targetResult.stderrTruncated
    },
    artifacts: {
      directory: trialDirectory,
      workspace,
      events: eventsFile,
      stdout: path.join(trialDirectory, "stdout.log"),
      stderr: path.join(trialDirectory, "stderr.log")
    }
  };
  await writeFile(path.join(trialDirectory, "result.json"), `${JSON.stringify(result, null, 2)}\n`, "utf8");
  return result;
}

async function manifestFor(definitions, runId, repetitions) {
  const taskHashes = {};
  for (const entry of definitions.tasks) taskHashes[entry.task.id] = await sha256File(entry.taskPath);
  return {
    schemaVersion: 1,
    runId,
    createdAt: new Date().toISOString(),
    harnessVersion: "0.1.0",
    platform: process.platform,
    architecture: process.arch,
    osRelease: os.release(),
    nodeVersion: process.version,
    suitePath: definitions.suitePath,
    suiteSha256: await sha256File(definitions.suitePath),
    targetPath: definitions.targetPath,
    targetSha256: await sha256File(definitions.targetPath),
    taskHashes,
    repetitions
  };
}

export async function runSuite({ suitePath, targetPath, outputDirectory, repetitions }) {
  const definitions = await validateDefinitions(suitePath, targetPath);
  const runId = makeId(definitions.suite.id.replace(/[^A-Za-z0-9]+/g, "-").toLowerCase());
  const runDirectory = path.join(path.resolve(outputDirectory), runId);
  await ensureDirectory(runDirectory);
  const effectiveSuiteRepetitions = repetitions ?? definitions.suite.repetitions ?? 1;
  const manifest = await manifestFor(definitions, runId, effectiveSuiteRepetitions);
  await writeFile(path.join(runDirectory, "manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
  const startedAt = new Date().toISOString();
  const results = [];
  for (const taskEntry of definitions.tasks) {
    const taskRepetitions = repetitions ?? taskEntry.task.repetitions ?? effectiveSuiteRepetitions;
    for (let repetition = 1; repetition <= taskRepetitions; repetition += 1) {
      try {
        results.push(await runTrial({
          runId,
          runDirectory,
          taskEntry,
          target: definitions.target,
          targetDirectory: definitions.targetDirectory,
          repetition
        }));
      } catch (error) {
        const now = new Date().toISOString();
        const failedResult = {
          schemaVersion: 1,
          runId,
          trialId: `${taskEntry.task.id}_${repetition}`,
          taskId: taskEntry.task.id,
          targetId: definitions.target.id,
          repetition,
          status: "infra_error",
          startedAt: now,
          completedAt: now,
          scores: { outcome: false, trajectory: false, safety: false, efficiency: false },
          checks: [],
          metrics: {},
          error: error.message,
          artifacts: {}
        };
        const failedTrialDirectory = path.join(runDirectory, "trials", `${taskEntry.task.id}_${repetition}`);
        await ensureDirectory(failedTrialDirectory);
        await writeFile(path.join(failedTrialDirectory, "result.json"), `${JSON.stringify(failedResult, null, 2)}\n`, "utf8");
        results.push(failedResult);
      }
    }
  }
  const completedAt = new Date().toISOString();
  const summary = aggregateResults({
    runId,
    suite: definitions.suite,
    target: definitions.target,
    manifest,
    results,
    startedAt,
    completedAt
  });
  const reports = await writeReport(runDirectory, summary);
  return { summary, runDirectory, reports };
}
