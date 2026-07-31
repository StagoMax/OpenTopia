const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const statePath = path.resolve(process.argv[2] || "");
const manifestPath = path.resolve(process.argv[3] || "");
if (!statePath || !manifestPath) {
  process.stderr.write("usage: grader.cjs <state-path> <task-manifest>\n");
  process.exit(2);
}

const checks = [];

function check(id, action) {
  try {
    action();
    checks.push({ id, passed: true });
  } catch (error) {
    checks.push({
      id,
      passed: false,
      detail: String(error?.message || error).slice(0, 300),
    });
  }
}

let task;
let state;
check("manifest-loads", () => {
  task = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
  assert.ok(task.fixture?.expected, "fixture.expected is required");
});
check("saved-state-exists", () => {
  assert.ok(fs.existsSync(statePath), "Save profile was not completed");
  state = JSON.parse(fs.readFileSync(statePath, "utf8"));
});
check("workspace-name", () => {
  assert.ok(state && task, "state or task is unavailable");
  assert.equal(state.workspaceName, task.fixture.expected.workspaceName);
});
check("operation-mode", () => {
  assert.ok(state && task, "state or task is unavailable");
  assert.equal(state.operationMode, task.fixture.expected.operationMode);
});
check("local-history", () => {
  assert.ok(state && task, "state or task is unavailable");
  assert.equal(state.keepLocalHistory, task.fixture.expected.keepLocalHistory);
});
check("state-schema", () => {
  assert.ok(state, "state is unavailable");
  assert.equal(state.schemaVersion, 1);
  assert.equal(typeof state.savedAt, "string");
});

const passedChecks = checks.filter((checkResult) => checkResult.passed).length;
const result = {
  passed: passedChecks === checks.length,
  passedChecks,
  totalChecks: checks.length,
  checks,
};
process.stdout.write(`${JSON.stringify(result)}\n`);
process.exit(result.passed ? 0 : 1);
