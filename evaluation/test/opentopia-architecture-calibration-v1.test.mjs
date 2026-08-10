import assert from "node:assert/strict";
import { access } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { validateDefinitions } from "../src/runner.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const suiteDirectory = path.resolve(
  here,
  "../examples/opentopia-architecture-calibration-v1",
);

test("architecture calibration v1 is a frozen public 12-task suite", async () => {
  const definitions = await validateDefinitions(
    path.join(suiteDirectory, "suite.json"),
    path.join(suiteDirectory, "target.json"),
  );

  assert.equal(definitions.suite.id, "opentopia-architecture-calibration-v1");
  assert.equal(definitions.suite.version, "1.0.0");
  assert.equal(definitions.suite.evaluationClass, "calibration");
  assert.equal(definitions.suite.visibility, "public");
  assert.equal(definitions.suite.frozen, true);
  assert.equal(definitions.tasks.length, 12);
  assert.deepEqual(
    definitions.suite.benchmarkReferences.map((entry) => entry.name),
    ["SWE-bench", "Terminal-Bench 2.0", "tau-bench"],
  );
});

test("calibration tasks cover repository, terminal, policy, and recovery work", async () => {
  const definitions = await validateDefinitions(
    path.join(suiteDirectory, "suite.json"),
    path.join(suiteDirectory, "target.json"),
  );
  const tags = definitions.tasks.flatMap(({ task }) => task.tags ?? []);
  const restartTasks = definitions.tasks.filter(({ task }) =>
    task.phases?.some((phase) => phase.restartBefore),
  );
  const multiRestartTasks = restartTasks.filter(
    ({ task }) =>
      task.phases.filter((phase) => phase.restartBefore).length >= 2,
  );

  assert.ok(tags.includes("swe-bench-inspired"));
  assert.ok(tags.includes("terminal-bench-inspired"));
  assert.ok(tags.includes("tau-bench-inspired"));
  assert.ok(restartTasks.length >= 5);
  assert.ok(multiRestartTasks.length >= 1);

  for (const { task, taskDirectory } of definitions.tasks) {
    assert.equal(task.version, "1.0.0", `${task.id} must be versioned`);
    assert.ok(
      task.graders.trajectory?.requireCompletionClaim,
      `${task.id} must grade completion claims`,
    );
    await access(path.join(taskDirectory, "grader.cjs"));
    await access(path.join(taskDirectory, task.fixture.source));
  }
});
