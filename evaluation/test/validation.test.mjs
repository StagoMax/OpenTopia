import assert from "node:assert/strict";
import test from "node:test";
import {
  ValidationError,
  validateEvent,
  validateSuite,
  validateTarget,
  validateTask
} from "../src/validation.mjs";

const validTask = () => ({
  schemaVersion: 1,
  id: "TASK-001",
  title: "Task",
  suite: "suite",
  prompt: "Do work",
  capabilityPolicy: {
    tools: { mustUse: ["read"], mustNotUse: ["delete"] }
  },
  graders: {
    files: [{ id: "result", path: "result.json", exists: true }]
  }
});

test("accepts valid task, suite, target, and event contracts", () => {
  assert.equal(validateTask(validTask()).id, "TASK-001");
  assert.equal(validateSuite({ schemaVersion: 1, id: "suite", title: "Suite", tasks: ["task.json"] }).id, "suite");
  assert.equal(validateTarget({ schemaVersion: 1, id: "target", command: "node" }).id, "target");
  assert.equal(validateEvent({
    schemaVersion: 1,
    runId: "run",
    trialId: "trial",
    taskId: "task",
    timestamp: new Date().toISOString(),
    source: "test",
    type: "tool.call.completed",
    payload: {}
  }).type, "tool.call.completed");
});

test("rejects contradictory capability requirements", () => {
  const task = validTask();
  task.capabilityPolicy.tools.mustNotUse.push("read");
  assert.throws(() => validateTask(task), (error) => {
    assert.ok(error instanceof ValidationError);
    assert.match(error.message, /both mustUse and mustNotUse/);
    return true;
  });
});

test("rejects malformed suite and target definitions", () => {
  assert.throws(
    () => validateSuite({ schemaVersion: 1, id: "suite", title: "Suite", tasks: [] }),
    /non-empty array/
  );
  assert.throws(
    () => validateTarget({ schemaVersion: 1, id: "target", command: "node", promptTransport: "socket" }),
    /promptTransport/
  );
});
