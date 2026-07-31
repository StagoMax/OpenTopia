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
    for (const phase of entry.task.phases ?? []) {
      for (const grader of phase.graders?.commands ?? []) {
        if (grader.cwd) resolveInside(entry.taskDirectory, grader.cwd, `phase grader cwd for ${entry.task.id}`);
      }
    }
    if (entry.task.phases?.some((phase) => phase.restartBefore) && !definitions.target.lifecycle?.restart) {
      throw new Error(`Task ${entry.task.id} requires a restart, but target ${definitions.target.id} has no lifecycle.restart`);
    }
    if (
      entry.task.graders.trajectory?.requireThreadReuse &&
      !entry.task.phases?.some((phase) => phase.restartBefore)
    ) {
      throw new Error(`Task ${entry.task.id} requires thread reuse, but has no restart phase`);
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

function phasesFor(task) {
  return task.phases ?? [{ id: "default", prompt: task.prompt, restartBefore: false }];
}

function combineTargetResults(results) {
  return {
    exitCode: results.every((result) => result.exitCode === 0) ? 0 : results.find((result) => result.exitCode !== 0)?.exitCode ?? null,
    signal: results.find((result) => result.signal)?.signal ?? null,
    timedOut: results.some((result) => result.timedOut),
    spawnError: results.find((result) => result.spawnError)?.spawnError ?? null,
    stdout: results.map((result, index) => `[phase ${index + 1}]\n${result.stdout}`).join("\n"),
    stderr: results.map((result, index) => `[phase ${index + 1}]\n${result.stderr}`).join("\n"),
    stdoutTruncated: results.some((result) => result.stdoutTruncated),
    stderrTruncated: results.some((result) => result.stderrTruncated),
    elapsedMs: results.reduce((total, result) => total + result.elapsedMs, 0)
  };
}

function phaseVariables(variables, phase, phaseIndex, phaseCount, promptFile, targetEventsFile, targetStateFile) {
  return {
    ...variables,
    phaseId: phase.id,
    phaseIndex: String(phaseIndex),
    phaseCount: String(phaseCount),
    promptFile,
    eventsFile: targetEventsFile,
    targetStateFile
  };
}

function targetEnvironment(target, variables, securitySettings, phase) {
  const environment = {};
  for (const [key, value] of Object.entries(target.env ?? {})) {
    environment[key] = replacePlaceholders(value, variables);
  }
  for (const key of target.passEnvironment ?? []) {
    if (process.env[key] === undefined) throw new Error(`Target requires missing environment variable ${key}`);
    environment[key] = process.env[key];
  }
  Object.assign(environment, {
    AGENT_EVAL_RUN_ID: variables.runId,
    AGENT_EVAL_TRIAL_ID: variables.trialId,
    AGENT_EVAL_TASK_ID: variables.taskId,
    AGENT_EVAL_WORKSPACE: variables.workspace,
    AGENT_EVAL_PROMPT_FILE: variables.promptFile,
    AGENT_EVAL_EVENTS_PATH: variables.eventsFile,
    AGENT_EVAL_TRIAL_DIR: variables.trialDir,
    AGENT_EVAL_TARGET_ID: target.id,
    AGENT_EVAL_PHASE_ID: phase.id,
    AGENT_EVAL_PHASE_INDEX: variables.phaseIndex,
    AGENT_EVAL_PHASE_COUNT: variables.phaseCount,
    AGENT_EVAL_TARGET_STATE_PATH: variables.targetStateFile
  });
  if (securitySettings.exposeCanariesAsEnvironment) {
    environment.AGENT_EVAL_SECRET_CANARIES = JSON.stringify(securitySettings.secretCanaries ?? []);
  }
  return environment;
}

const pause = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));

async function restartTarget({ target, targetDirectory, variables, phase, context, harnessEvents }) {
  const restart = target.lifecycle?.restart;
  if (!restart) throw new Error(`Phase ${phase.id} requires a target restart, but ${target.id} does not define lifecycle.restart`);
  harnessEvents.push(makeHarnessEvent(context, "application.recovery.restart.started", { phaseId: phase.id, kind: restart.kind }));
  const timeoutMs = (restart.timeoutSeconds ?? 60) * 1000;
  if (restart.kind === "command") {
    const result = await runCommand({
      command: resolveTargetExecutable(replacePlaceholders(restart.command, variables), targetDirectory),
      args: (restart.args ?? []).map((argument) => replacePlaceholders(argument, variables)),
      cwd: restart.cwd
        ? targetCwd({ cwd: restart.cwd }, targetDirectory, variables)
        : targetDirectory,
      env: targetEnvironment(target, variables, {}, phase),
      inheritEnvironment: target.inheritEnvironment ?? false,
      timeoutMs
    });
    if (result.exitCode !== 0 || result.timedOut || result.spawnError) {
      throw new Error(`Target restart command failed: ${result.spawnError ?? result.stderr ?? `exit ${result.exitCode}`}`);
    }
    harnessEvents.push(makeHarnessEvent(context, "application.recovery.restart.completed", {
      phaseId: phase.id,
      kind: restart.kind,
      elapsedMs: result.elapsedMs
    }));
    return result.elapsedMs;
  }

  const controlPath = process.env[restart.pathEnvironment];
  if (!controlPath) throw new Error(`Target restart control environment ${restart.pathEnvironment} is not set`);
  const requestId = makeId("restart");
  await writeFile(controlPath, `${JSON.stringify({
    schemaVersion: 1,
    action: "restart",
    requestId,
    requestedAt: new Date().toISOString(),
    trialId: variables.trialId,
    phaseId: phase.id
  })}\n`, "utf8");
  const startedAt = Date.now();
  const pollMs = restart.pollMs ?? 250;
  let lastError = null;
  while (Date.now() - startedAt < timeoutMs) {
    await pause(pollMs);
    let response;
    try {
      response = JSON.parse(await readFile(controlPath, "utf8"));
    } catch (error) {
      lastError = error;
      continue;
    }
    if (response.requestId !== requestId) continue;
    if (response.status === "completed") {
      const elapsedMs = Date.now() - startedAt;
      harnessEvents.push(makeHarnessEvent(context, "application.recovery.restart.completed", {
        phaseId: phase.id,
        kind: restart.kind,
        elapsedMs
      }));
      return elapsedMs;
    }
    if (response.status === "failed") throw new Error(response.error ?? "server supervisor reported a restart failure");
  }
  throw new Error(`Target restart control timed out${lastError ? `: ${lastError.message}` : ""}`);
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
  const targetStateFile = path.join(controlDirectory, "target-state.json");
  const eventsFile = path.join(trialDirectory, "events.jsonl");
  await ensureDirectory(workspace);
  await ensureDirectory(controlDirectory);
  if (task.fixture) {
    const fixture = resolveInside(taskDirectory, task.fixture.source, `fixture for ${task.id}`, { allowBase: false });
    await copyFixture(fixture, workspace);
  }
  const securitySettings = task.graders.security ?? {};
  const secrets = securitySettings.secretCanaries ?? [];
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
    // Browser suites are started by the evaluation supervisor. These values are deliberately
    // explicit placeholders rather than inherited by graders, which keeps grader environments
    // minimal while still allowing hidden backend-state checks.
    browserFixtureUrl: process.env.OPENTOPIA_EVAL_BROWSER_FIXTURE_URL ?? "",
    browserDataRoot: process.env.OPENTOPIA_EVAL_BROWSER_DATA_ROOT ?? "",
    browserFixtureState: process.env.OPENTOPIA_EVAL_BROWSER_FIXTURE_STATE ?? ""
  };
  const harnessEvents = [makeHarnessEvent(context, "application.launch.started", { targetId: target.id })];
  const parsedErrors = [];
  const phaseChecks = [];
  let rawEventText = "";
  const targetResults = [];
  const stages = [];
  const promptTransport = target.promptTransport ?? "stdin";
  const phases = phasesFor(task);
  for (const [phaseIndex, phase] of phases.entries()) {
    const promptFile = path.join(controlDirectory, `prompt-${phaseIndex + 1}-${phase.id}.txt`);
    const targetEventsFile = path.join(controlDirectory, `target-events-${phaseIndex + 1}-${phase.id}.jsonl`);
    const stageVariables = phaseVariables(
      variables,
      phase,
      phaseIndex + 1,
      phases.length,
      promptFile,
      targetEventsFile,
      targetStateFile
    );
    if (phase.restartBefore) await restartTarget({ target, targetDirectory, variables: stageVariables, phase, context, harnessEvents });
    await writeFile(promptFile, phase.prompt, "utf8");
    harnessEvents.push(makeHarnessEvent(context, "application.phase.started", { phaseId: phase.id, phaseIndex: phaseIndex + 1 }));
    const targetResult = await runCommand({
      command: resolveTargetExecutable(replacePlaceholders(target.command, stageVariables), targetDirectory),
      args: (target.args ?? []).map((argument) => replacePlaceholders(argument, stageVariables)),
      cwd: targetCwd(target, targetDirectory, stageVariables),
      env: targetEnvironment(target, stageVariables, securitySettings, phase),
      inheritEnvironment: target.inheritEnvironment ?? false,
      stdin: promptTransport === "stdin" ? phase.prompt : undefined,
      timeoutMs: (task.budgets?.watchdogSeconds ?? 1800) * 1000
    });
    targetResults.push(targetResult);
    harnessEvents.push(makeHarnessEvent(context, "application.launch.completed", {
      phaseId: phase.id,
      exitCode: targetResult.exitCode,
      signal: targetResult.signal,
      timedOut: targetResult.timedOut,
      elapsedMs: targetResult.elapsedMs,
      spawnError: targetResult.spawnError
    }));
    let stageRawEvents = "";
    try {
      stageRawEvents = await readFile(targetEventsFile, "utf8");
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
    rawEventText += stageRawEvents;
    const parsed = await readTargetEvents(targetEventsFile, context);
    parsedErrors.push(...parsed.errors.map((error) => ({ phaseId: phase.id, ...error })));
    harnessEvents.push(...parsed.events);
    await rm(targetEventsFile, { force: true });
    const phaseGraderChecks = [
      ...await gradeFiles(workspace, phase.graders?.files ?? []),
      ...await gradeCommands({
        graders: phase.graders?.commands ?? [],
        taskDirectory,
        variables: stageVariables,
        secrets
      })
    ].map((check) => ({ ...check, id: `phase.${phase.id}.${check.id}` }));
    phaseChecks.push(...phaseGraderChecks);
    const phasePassed = phaseGraderChecks.every((check) => check.passed);
    const phaseTerminalEvents = parsed.events.filter((event) => event.type === "application.turn.completed");
    const phaseApplicationFailed = phaseTerminalEvents.some((event) => event.payload.status !== "succeeded");
    stages.push({
      id: phase.id,
      restartBefore: phase.restartBefore ?? false,
      elapsedMs: targetResult.elapsedMs,
      exitCode: targetResult.exitCode,
      timedOut: targetResult.timedOut,
      spawnError: targetResult.spawnError,
      invalidEventCount: parsed.errors.length,
      applicationSucceeded: phaseTerminalEvents.length === 0 ? null : !phaseApplicationFailed,
      graderPassed: phasePassed,
      graderCheckCount: phaseGraderChecks.length
    });
    harnessEvents.push(makeHarnessEvent(context, "phase.completed", {
      phaseId: phase.id,
      phaseIndex: phaseIndex + 1,
      exitCode: targetResult.exitCode,
      timedOut: targetResult.timedOut,
      graderPassed: phasePassed
    }));
    if (
      targetResult.exitCode !== 0 ||
      targetResult.timedOut ||
      targetResult.spawnError ||
      phaseApplicationFailed ||
      !phasePassed
    ) break;
  }
  const targetResult = combineTargetResults(targetResults);
  const events = harnessEvents.sort((left, right) => {
    const byTime = Date.parse(left.timestamp) - Date.parse(right.timestamp);
    return byTime !== 0 ? byTime : (left.monotonicMs ?? 0) - (right.monotonicMs ?? 0);
  });

  // Command graders may verify a trajectory in addition to backend state. Materialize the
  // normalized event stream before invoking them; it is rewritten with redaction below.
  await writeEvents(eventsFile, events);

  const checks = [
    {
      id: "target.exit",
      category: "outcome",
      passed: targetResult.exitCode === 0 && !targetResult.timedOut && !targetResult.spawnError,
      hard: false,
      graderError: false,
      detail: stages
    }
  ];
  checks.push(...phaseChecks);
  const applicationTerminalEvents = events.filter((event) => event.type === "application.turn.completed");
  if (applicationTerminalEvents.length > 0) {
    const failedTerminals = applicationTerminalEvents.filter((event) => event.payload.status !== "succeeded");
    checks.push({
      id: "application.terminal-status",
      category: "outcome",
      passed: failedTerminals.length === 0,
      hard: false,
      graderError: false,
      detail: applicationTerminalEvents.map((event) => event.payload.status ?? "unknown")
    });
  }
  checks.push(...await gradeFiles(workspace, task.graders.files ?? []));
  checks.push(...await gradeCommands({
    graders: task.graders.commands ?? [],
    taskDirectory,
    variables,
    secrets
  }));
  const trajectory = gradeTrajectory(events, parsedErrors, task);
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
      invalidEventCount: parsedErrors.length
    },
    process: {
      exitCode: targetResult.exitCode,
      signal: targetResult.signal,
      timedOut: targetResult.timedOut,
      stdoutTruncated: targetResult.stdoutTruncated,
      stderrTruncated: targetResult.stderrTruncated,
      stages
    },
    artifacts: {
      directory: trialDirectory,
      workspace,
      events: eventsFile,
      targetState: targetStateFile,
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
