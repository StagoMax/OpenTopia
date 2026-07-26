import { readFile } from "node:fs/promises";
import path from "node:path";

export class ValidationError extends Error {
  constructor(kind, issues) {
    super(`${kind} validation failed:\n- ${issues.join("\n- ")}`);
    this.name = "ValidationError";
    this.kind = kind;
    this.issues = issues;
  }
}

const isObject = (value) => value !== null && typeof value === "object" && !Array.isArray(value);
const isNonEmptyString = (value) => typeof value === "string" && value.trim().length > 0;

function assertObject(value, field, issues) {
  if (!isObject(value)) {
    issues.push(`${field} must be an object`);
    return false;
  }
  return true;
}

function assertString(value, field, issues, { allowEmpty = false } = {}) {
  if (typeof value !== "string" || (!allowEmpty && value.trim().length === 0)) {
    issues.push(`${field} must be ${allowEmpty ? "a string" : "a non-empty string"}`);
  }
}

function assertStringArray(value, field, issues, { nested = false } = {}) {
  if (!Array.isArray(value)) {
    issues.push(`${field} must be an array`);
    return;
  }
  if (nested) {
    value.forEach((entry, index) => {
      if (!Array.isArray(entry) || entry.length === 0 || entry.some((item) => !isNonEmptyString(item))) {
        issues.push(`${field}[${index}] must be a non-empty array of strings`);
      }
    });
    return;
  }
  if (value.some((item) => !isNonEmptyString(item))) {
    issues.push(`${field} must contain only non-empty strings`);
  }
  if (new Set(value).size !== value.length) {
    issues.push(`${field} must not contain duplicates`);
  }
}

function assertPositiveInteger(value, field, issues, { allowZero = false, maximum } = {}) {
  const minimum = allowZero ? 0 : 1;
  if (!Number.isInteger(value) || value < minimum || (maximum !== undefined && value > maximum)) {
    issues.push(`${field} must be an integer between ${minimum} and ${maximum ?? "infinity"}`);
  }
}

function validateCapabilityPolicy(policy, issues) {
  if (policy === undefined) return;
  if (!assertObject(policy, "capabilityPolicy", issues)) return;
  for (const category of ["tools", "skills", "mcpServers", "plugins"]) {
    const entry = policy[category];
    if (entry === undefined) continue;
    if (!assertObject(entry, `capabilityPolicy.${category}`, issues)) continue;
    for (const field of ["mustUse", "mustNotUse", "optional"]) {
      if (entry[field] !== undefined) {
        assertStringArray(entry[field], `capabilityPolicy.${category}.${field}`, issues);
      }
    }
    if (entry.oneOf !== undefined) {
      assertStringArray(entry.oneOf, `capabilityPolicy.${category}.oneOf`, issues, { nested: true });
    }
    const mustUse = new Set(entry.mustUse ?? []);
    for (const forbidden of entry.mustNotUse ?? []) {
      if (mustUse.has(forbidden)) {
        issues.push(`capabilityPolicy.${category} lists ${forbidden} in both mustUse and mustNotUse`);
      }
    }
  }
}

function validateGraders(graders, issues) {
  if (!assertObject(graders, "graders", issues)) return;
  if (graders.commands !== undefined) {
    if (!Array.isArray(graders.commands)) {
      issues.push("graders.commands must be an array");
    } else {
      graders.commands.forEach((grader, index) => {
        const prefix = `graders.commands[${index}]`;
        if (!assertObject(grader, prefix, issues)) return;
        assertString(grader.id, `${prefix}.id`, issues);
        assertString(grader.command, `${prefix}.command`, issues);
        if (grader.args !== undefined) assertStringArray(grader.args, `${prefix}.args`, issues);
        if (grader.timeoutSeconds !== undefined) {
          assertPositiveInteger(grader.timeoutSeconds, `${prefix}.timeoutSeconds`, issues);
        }
      });
    }
  }
  if (graders.files !== undefined) {
    if (!Array.isArray(graders.files)) {
      issues.push("graders.files must be an array");
    } else {
      graders.files.forEach((grader, index) => {
        const prefix = `graders.files[${index}]`;
        if (!assertObject(grader, prefix, issues)) return;
        assertString(grader.id, `${prefix}.id`, issues);
        assertString(grader.path, `${prefix}.path`, issues);
        if (grader.contains !== undefined && typeof grader.contains !== "string") {
          assertStringArray(grader.contains, `${prefix}.contains`, issues);
        }
        if (grader.notContains !== undefined && typeof grader.notContains !== "string") {
          assertStringArray(grader.notContains, `${prefix}.notContains`, issues);
        }
      });
    }
  }
  if (graders.trajectory !== undefined) {
    assertObject(graders.trajectory, "graders.trajectory", issues);
  }
  if (graders.security !== undefined) {
    if (assertObject(graders.security, "graders.security", issues)) {
      for (const field of ["protectedPaths", "secretCanaries", "forbiddenEventTypes"]) {
        if (graders.security[field] !== undefined) {
          assertStringArray(graders.security[field], `graders.security.${field}`, issues);
        }
      }
    }
  }
}

export async function loadJson(filePath) {
  const absolutePath = path.resolve(filePath);
  let source;
  try {
    source = await readFile(absolutePath, "utf8");
  } catch (error) {
    throw new ValidationError("JSON file", [`${absolutePath}: ${error.message}`]);
  }
  try {
    return JSON.parse(source);
  } catch (error) {
    throw new ValidationError("JSON file", [`${absolutePath}: ${error.message}`]);
  }
}

export function validateTask(task) {
  const issues = [];
  if (!assertObject(task, "task", issues)) throw new ValidationError("Task", issues);
  if (task.schemaVersion !== 1) issues.push("schemaVersion must equal 1");
  for (const field of ["id", "title", "suite", "prompt"]) assertString(task[field], field, issues);
  if (task.version !== undefined) assertString(task.version, "version", issues);
  if (task.tags !== undefined) assertStringArray(task.tags, "tags", issues);
  if (task.repetitions !== undefined) assertPositiveInteger(task.repetitions, "repetitions", issues, { maximum: 100 });
  if (task.fixture !== undefined && assertObject(task.fixture, "fixture", issues)) {
    assertString(task.fixture.source, "fixture.source", issues);
  }
  if (task.budgets !== undefined && assertObject(task.budgets, "budgets", issues)) {
    if (task.budgets.watchdogSeconds !== undefined) {
      assertPositiveInteger(task.budgets.watchdogSeconds, "budgets.watchdogSeconds", issues);
    }
    for (const field of ["maxToolCalls", "maxTotalTokens"]) {
      if (task.budgets[field] !== undefined) {
        assertPositiveInteger(task.budgets[field], `budgets.${field}`, issues, { allowZero: true });
      }
    }
  }
  validateCapabilityPolicy(task.capabilityPolicy, issues);
  validateGraders(task.graders, issues);
  if (issues.length > 0) throw new ValidationError("Task", issues);
  return task;
}

export function validateSuite(suite) {
  const issues = [];
  if (!assertObject(suite, "suite", issues)) throw new ValidationError("Suite", issues);
  if (suite.schemaVersion !== 1) issues.push("schemaVersion must equal 1");
  for (const field of ["id", "title"]) assertString(suite[field], field, issues);
  if (!Array.isArray(suite.tasks) || suite.tasks.length === 0) {
    issues.push("tasks must be a non-empty array");
  } else {
    assertStringArray(suite.tasks, "tasks", issues);
  }
  if (suite.repetitions !== undefined) assertPositiveInteger(suite.repetitions, "repetitions", issues, { maximum: 100 });
  if (issues.length > 0) throw new ValidationError("Suite", issues);
  return suite;
}

export function validateTarget(target) {
  const issues = [];
  if (!assertObject(target, "target", issues)) throw new ValidationError("Target", issues);
  if (target.schemaVersion !== 1) issues.push("schemaVersion must equal 1");
  for (const field of ["id", "command"]) assertString(target[field], field, issues);
  if (target.args !== undefined) assertStringArray(target.args, "args", issues);
  if (target.cwd !== undefined) assertString(target.cwd, "cwd", issues);
  if (target.env !== undefined && assertObject(target.env, "env", issues)) {
    for (const [key, value] of Object.entries(target.env)) {
      if (!isNonEmptyString(key) || typeof value !== "string") issues.push("target.env keys and values must be strings");
    }
  }
  if (target.passEnvironment !== undefined) {
    assertStringArray(target.passEnvironment, "passEnvironment", issues);
  }
  if (target.promptTransport !== undefined && !["stdin", "file", "none"].includes(target.promptTransport)) {
    issues.push("promptTransport must be stdin, file, or none");
  }
  if (target.eventTransport !== undefined && target.eventTransport !== "jsonl-file") {
    issues.push("eventTransport must be jsonl-file");
  }
  if (issues.length > 0) throw new ValidationError("Target", issues);
  return target;
}

export function validateEvent(event) {
  const issues = [];
  if (!assertObject(event, "event", issues)) throw new ValidationError("Event", issues);
  if (event.schemaVersion !== 1) issues.push("schemaVersion must equal 1");
  for (const field of ["runId", "trialId", "taskId", "timestamp", "source", "type"]) {
    assertString(event[field], field, issues);
  }
  if (isNonEmptyString(event.timestamp) && Number.isNaN(Date.parse(event.timestamp))) {
    issues.push("timestamp must be a valid ISO date-time");
  }
  if (!isObject(event.payload)) issues.push("payload must be an object");
  if (issues.length > 0) throw new ValidationError("Event", issues);
  return event;
}
