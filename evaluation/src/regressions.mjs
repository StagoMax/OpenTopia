import { readFile, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import { validateDefinitions } from "./runner.mjs";
import { isAbilityEligible } from "./failures.mjs";
import { ensureDirectory } from "./utils.mjs";
import { loadJson, ValidationError } from "./validation.mjs";

const CASE_KINDS = new Set(["incident", "risk"]);
const CASE_STATES = new Set(["open", "active", "fixed", "monitoring", "retired"]);
const SEVERITIES = new Set(["critical", "high", "medium", "low"]);
const AREAS = new Set([
  "tool_call",
  "cross_tool",
  "provider_transport",
  "recovery",
  "safety",
  "model_behavior",
  "edge_case",
  "grader",
]);
const GATES = new Set(["smoke", "pr", "nightly", "manual"]);
const ROOT_CAUSE_STATES = new Set(["confirmed", "suspected", "unknown", "not_applicable"]);
const COVERAGE_KINDS = new Set(["evaluation-task", "source-test"]);
const COVERAGE_PURPOSES = new Set(["regression", "reproducer", "monitor"]);

function isObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function isText(value) {
  return typeof value === "string" && value.trim().length > 0;
}

function requireText(value, field, issues) {
  if (!isText(value)) issues.push(`${field} must be a non-empty string`);
}

function requireEnum(value, field, allowed, issues) {
  if (!allowed.has(value)) {
    issues.push(`${field} must be one of: ${[...allowed].join(", ")}`);
  }
}

function requireStringArray(value, field, issues, { allowEmpty = true } = {}) {
  if (!Array.isArray(value) || (!allowEmpty && value.length === 0)) {
    issues.push(`${field} must be ${allowEmpty ? "an" : "a non-empty"} array`);
    return;
  }
  if (value.some((entry) => !isText(entry))) {
    issues.push(`${field} must contain only non-empty strings`);
  }
  if (new Set(value).size !== value.length) {
    issues.push(`${field} must not contain duplicates`);
  }
}

function validateOrigin(origin, prefix, issues) {
  if (!isObject(origin)) {
    issues.push(`${prefix} must be an object`);
    return;
  }
  requireEnum(origin.kind, `${prefix}.kind`, new Set(["eval-run", "production", "risk-analysis"]), issues);
  if (origin.kind === "eval-run") {
    requireText(origin.runId, `${prefix}.runId`, issues);
    requireText(origin.trialId, `${prefix}.trialId`, issues);
  }
  if (origin.threadId !== undefined) requireText(origin.threadId, `${prefix}.threadId`, issues);
  if (origin.reference !== undefined) requireText(origin.reference, `${prefix}.reference`, issues);
}

function validateCoverage(coverage, prefix, issues) {
  if (!isObject(coverage)) {
    issues.push(`${prefix} must be an object`);
    return;
  }
  requireEnum(coverage.kind, `${prefix}.kind`, COVERAGE_KINDS, issues);
  requireEnum(coverage.purpose, `${prefix}.purpose`, COVERAGE_PURPOSES, issues);
  if (coverage.kind === "evaluation-task") {
    for (const field of ["suite", "target", "taskId"]) {
      requireText(coverage[field], `${prefix}.${field}`, issues);
    }
    if (coverage.checkIds !== undefined) {
      requireStringArray(coverage.checkIds, `${prefix}.checkIds`, issues, { allowEmpty: false });
    }
  } else if (coverage.kind === "source-test") {
    for (const field of ["file", "anchor", "command"]) {
      requireText(coverage[field], `${prefix}.${field}`, issues);
    }
  }
}

function validateRegistryShape(registry) {
  const issues = [];
  if (!isObject(registry)) {
    throw new ValidationError("Regression registry", ["registry must be an object"]);
  }
  if (registry.schemaVersion !== 1) issues.push("schemaVersion must equal 1");
  for (const field of ["id", "title"]) requireText(registry[field], field, issues);
  if (!Array.isArray(registry.cases) || registry.cases.length === 0) {
    issues.push("cases must be a non-empty array");
  } else {
    const ids = new Set();
    registry.cases.forEach((entry, index) => {
      const prefix = `cases[${index}]`;
      if (!isObject(entry)) {
        issues.push(`${prefix} must be an object`);
        return;
      }
      for (const field of ["id", "title", "firstObservedAt", "observedBehavior", "expectedBehavior"]) {
        requireText(entry[field], `${prefix}.${field}`, issues);
      }
      if (isText(entry.firstObservedAt) && Number.isNaN(Date.parse(entry.firstObservedAt))) {
        issues.push(`${prefix}.firstObservedAt must be a valid ISO date-time`);
      }
      if (entry.id && ids.has(entry.id)) issues.push(`${prefix}.id must be unique`);
      if (entry.id) ids.add(entry.id);
      requireEnum(entry.kind, `${prefix}.kind`, CASE_KINDS, issues);
      requireEnum(entry.state, `${prefix}.state`, CASE_STATES, issues);
      requireEnum(entry.severity, `${prefix}.severity`, SEVERITIES, issues);
      requireEnum(entry.area, `${prefix}.area`, AREAS, issues);
      requireEnum(entry.gate, `${prefix}.gate`, GATES, issues);
      validateOrigin(entry.origin, `${prefix}.origin`, issues);
      if (!isObject(entry.rootCause)) {
        issues.push(`${prefix}.rootCause must be an object`);
      } else {
        requireEnum(entry.rootCause.status, `${prefix}.rootCause.status`, ROOT_CAUSE_STATES, issues);
        requireText(entry.rootCause.summary, `${prefix}.rootCause.summary`, issues);
      }
      requireStringArray(entry.tags, `${prefix}.tags`, issues, { allowEmpty: false });
      if (!Array.isArray(entry.coverage) || entry.coverage.length === 0) {
        issues.push(`${prefix}.coverage must contain at least one executable coverage link`);
      } else {
        entry.coverage.forEach((coverage, coverageIndex) =>
          validateCoverage(coverage, `${prefix}.coverage[${coverageIndex}]`, issues));
      }
      const rootCauseStatus = entry.rootCause?.status;
      const needsNextAction = entry.state === "open" || rootCauseStatus === "unknown" || rootCauseStatus === "suspected";
      if (needsNextAction) requireText(entry.nextAction, `${prefix}.nextAction`, issues);
      if (["active", "fixed", "monitoring"].includes(entry.state)) {
        const hasRegression = entry.coverage?.some((coverage) => coverage.purpose === "regression");
        if (!hasRegression) issues.push(`${prefix} must have regression coverage while state is ${entry.state}`);
      }
    });
  }
  if (issues.length > 0) throw new ValidationError("Regression registry", issues);
}

function taskCheckIds(task) {
  const ids = new Set([
    "target.exit",
    "application.terminal-status",
    "trajectory.valid-events",
    "trajectory.phase-completions",
    "trajectory.completion-claim",
    "trajectory.thread-reuse",
    "security.protected-paths",
    "security.secret-canaries",
    "security.forbidden-events",
    "efficiency.max-tool-calls",
    "efficiency.max-total-tokens",
  ]);
  for (const grader of task.graders?.commands ?? []) ids.add(grader.id);
  for (const grader of task.graders?.files ?? []) ids.add(grader.id);
  for (const phase of task.phases ?? []) {
    for (const grader of phase.graders?.commands ?? []) {
      ids.add(`phase.${phase.id}.${grader.id}`);
    }
    for (const grader of phase.graders?.files ?? []) {
      ids.add(`phase.${phase.id}.${grader.id}`);
    }
  }
  return ids;
}

async function isFile(filePath) {
  try {
    return (await stat(filePath)).isFile();
  } catch (error) {
    if (error.code === "ENOENT") return false;
    throw error;
  }
}

export async function validateRegressionRegistry(registryPath) {
  const absoluteRegistryPath = path.resolve(registryPath);
  const registry = await loadJson(absoluteRegistryPath);
  validateRegistryShape(registry);
  const registryDirectory = path.dirname(absoluteRegistryPath);
  const issues = [];
  const definitionCache = new Map();
  const sourceCache = new Map();
  const resolvedCoverage = new Map();

  for (const entry of registry.cases) {
    const resolved = [];
    for (const coverage of entry.coverage) {
      if (coverage.kind === "evaluation-task") {
        const suitePath = path.resolve(registryDirectory, coverage.suite);
        const targetPath = path.resolve(registryDirectory, coverage.target);
        const cacheKey = `${suitePath}\u0000${targetPath}`;
        let definitions = definitionCache.get(cacheKey);
        if (!definitions) {
          try {
            definitions = await validateDefinitions(suitePath, targetPath);
            definitionCache.set(cacheKey, definitions);
          } catch (error) {
            issues.push(`${entry.id}: cannot load evaluation coverage: ${error.message}`);
            continue;
          }
        }
        const taskEntry = definitions.tasks.find(({ task }) => task.id === coverage.taskId);
        if (!taskEntry) {
          issues.push(`${entry.id}: task ${coverage.taskId} is not in ${coverage.suite}`);
          continue;
        }
        const checks = taskCheckIds(taskEntry.task);
        for (const checkId of coverage.checkIds ?? []) {
          if (!checks.has(checkId)) {
            issues.push(`${entry.id}: check ${checkId} is not defined by task ${coverage.taskId}`);
          }
        }
        resolved.push({
          ...coverage,
          suitePath,
          targetPath,
          suiteId: definitions.suite.id,
          taskTitle: taskEntry.task.title,
        });
      } else {
        const filePath = path.resolve(registryDirectory, coverage.file);
        if (!(await isFile(filePath))) {
          issues.push(`${entry.id}: source test file does not exist: ${coverage.file}`);
          continue;
        }
        let source = sourceCache.get(filePath);
        if (source === undefined) {
          source = await readFile(filePath, "utf8");
          sourceCache.set(filePath, source);
        }
        if (!source.includes(coverage.anchor)) {
          issues.push(`${entry.id}: source test anchor not found in ${coverage.file}: ${coverage.anchor}`);
        }
        resolved.push({ ...coverage, filePath });
      }
    }
    resolvedCoverage.set(entry.id, resolved);
  }

  if (issues.length > 0) throw new ValidationError("Regression coverage", issues);
  return { registryPath: absoluteRegistryPath, registry, resolvedCoverage };
}

function resultAttempts(summary, suiteId, taskId) {
  if (summary.suite?.id !== suiteId || !Array.isArray(summary.results)) return [];
  return summary.results
    .filter((result) => result.taskId === taskId)
    .map((result) => ({
      runId: summary.runId ?? "run",
      trialId: result.trialId ?? taskId,
      status: result.status ?? "unknown",
      valid: isAbilityEligible(result),
    }));
}

function percentage(numerator, denominator) {
  return denominator === 0 ? null : numerator / denominator;
}

function markdown(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll("|", "\\|")
    .replaceAll("\r", " ")
    .replaceAll("\n", " ");
}

function rateText(rate) {
  return rate === null ? "n/a" : `${(rate * 100).toFixed(1)}%`;
}

export async function buildRegressionReport({ registryPath, summaryPaths = [] }) {
  const validated = await validateRegressionRegistry(registryPath);
  const summaries = [];
  for (const summaryPath of summaryPaths) {
    summaries.push(await loadJson(path.resolve(summaryPath)));
  }

  const cases = validated.registry.cases.map((entry) => {
    const coverage = validated.resolvedCoverage.get(entry.id) ?? [];
    const attempts = coverage.flatMap((link) =>
      link.kind === "evaluation-task"
        ? summaries.flatMap((summary) => resultAttempts(summary, link.suiteId, link.taskId))
        : []);
    const validAttempts = attempts.filter((attempt) => attempt.valid);
    const passedAttempts = validAttempts.filter((attempt) => attempt.status === "passed");
    return {
      ...entry,
      coverage,
      attempts,
      validAttempts: validAttempts.length,
      passedAttempts: passedAttempts.length,
      passRate: percentage(passedAttempts.length, validAttempts.length),
      hasRegressionCoverage: coverage.some((link) => link.purpose === "regression"),
    };
  });

  const evaluated = cases.filter((entry) => entry.validAttempts > 0);
  const totalValidAttempts = cases.reduce((sum, entry) => sum + entry.validAttempts, 0);
  const totalPassedAttempts = cases.reduce((sum, entry) => sum + entry.passedAttempts, 0);
  const aggregate = {
    totalCases: cases.length,
    openCases: cases.filter((entry) => entry.state === "open").length,
    regressionCoveredCases: cases.filter((entry) => entry.hasRegressionCoverage).length,
    regressionCoverageRate: percentage(
      cases.filter((entry) => entry.hasRegressionCoverage).length,
      cases.length,
    ),
    evaluatedCases: evaluated.length,
    validAttempts: totalValidAttempts,
    passedAttempts: totalPassedAttempts,
    passRate: percentage(totalPassedAttempts, totalValidAttempts),
  };
  return {
    schemaVersion: 1,
    generatedAt: new Date().toISOString(),
    registry: { id: validated.registry.id, title: validated.registry.title },
    registryPath: validated.registryPath,
    summaries: summaryPaths.map((entry) => path.resolve(entry)),
    aggregate,
    cases,
  };
}

export function renderRegressionReport(report) {
  const lines = [
    `# Regression Registry: ${markdown(report.registry.title)}`,
    "",
    `Generated: ${report.generatedAt}`,
    "",
    `- Cases: ${report.aggregate.totalCases}`,
    `- Open incidents: ${report.aggregate.openCases}`,
    `- Regression coverage: ${report.aggregate.regressionCoveredCases}/${report.aggregate.totalCases} (${rateText(report.aggregate.regressionCoverageRate)})`,
    `- Evaluated cases in supplied summaries: ${report.aggregate.evaluatedCases}`,
    `- Valid attempt pass rate: ${report.aggregate.passedAttempts}/${report.aggregate.validAttempts} (${rateText(report.aggregate.passRate)})`,
    "",
    "| Case | Kind | Area | State | Gate | Root cause | Regression | Observed pass rate |",
    "|---|---|---|---|---|---|---:|---:|",
  ];
  for (const entry of report.cases) {
    lines.push(
      `| ${markdown(entry.id)} | ${markdown(entry.kind)} | ${markdown(entry.area)} | ${markdown(entry.state)} | ${markdown(entry.gate)} | ${markdown(entry.rootCause.status)} | ${entry.hasRegressionCoverage ? "yes" : "no"} | ${entry.validAttempts > 0 ? `${entry.passedAttempts}/${entry.validAttempts} (${rateText(entry.passRate)})` : "n/a"} |`,
    );
  }

  lines.push("", "## Open work", "");
  const open = report.cases.filter((entry) => entry.state === "open");
  if (open.length === 0) {
    lines.push("None.");
  } else {
    for (const entry of open) {
      lines.push(`- **${markdown(entry.id)}**: ${markdown(entry.nextAction)}`);
    }
  }

  lines.push("", "## Executable coverage", "");
  for (const entry of report.cases) {
    lines.push(`### ${markdown(entry.id)} - ${markdown(entry.title)}`, "");
    lines.push(`Observed: ${markdown(entry.observedBehavior)}`, "");
    lines.push(`Expected: ${markdown(entry.expectedBehavior)}`, "");
    for (const coverage of entry.coverage) {
      if (coverage.kind === "evaluation-task") {
        const checks = coverage.checkIds?.length ? `; checks: ${coverage.checkIds.join(", ")}` : "";
        lines.push(`- ${coverage.purpose}: evaluation task \`${coverage.suiteId}/${coverage.taskId}\`${markdown(checks)}`);
      } else {
        lines.push(`- ${coverage.purpose}: \`${markdown(coverage.command)}\` (\`${markdown(coverage.file)}\`, anchor \`${markdown(coverage.anchor)}\`)`);
      }
    }
    lines.push("");
  }
  return `${lines.join("\n")}\n`;
}

export async function writeRegressionReport({ registryPath, summaryPaths = [], outputPath }) {
  const report = await buildRegressionReport({ registryPath, summaryPaths });
  const absoluteOutputPath = path.resolve(outputPath);
  await ensureDirectory(path.dirname(absoluteOutputPath));
  await writeFile(absoluteOutputPath, renderRegressionReport(report), "utf8");
  const jsonPath = absoluteOutputPath.replace(/\.md$/i, ".json");
  await writeFile(jsonPath, `${JSON.stringify(report, null, 2)}\n`, "utf8");
  return { report, markdownPath: absoluteOutputPath, jsonPath };
}
