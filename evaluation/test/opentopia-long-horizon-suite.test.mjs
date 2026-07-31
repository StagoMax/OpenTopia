import assert from "node:assert/strict";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { validateDefinitions } from "../src/runner.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const suiteDirectory = path.resolve(here, "../examples/opentopia-long-horizon-suite");

test("long-horizon recovery suite validates all migrated tasks", async () => {
  const definitions = await validateDefinitions(
    path.join(suiteDirectory, "suite.json"),
    path.join(suiteDirectory, "target.json")
  );

  assert.equal(definitions.suite.id, "opentopia-long-horizon");
  assert.equal(definitions.target.id, "opentopia-http");
  assert.deepEqual(
    definitions.tasks.map((entry) => entry.task.id),
    ["LONG-LEDGER-001", "LONG-CONFIG-001", "LONG-DEPS-001"]
  );
});

test("long-horizon tasks preserve phase graders and controlled restart", async () => {
  const definitions = await validateDefinitions(
    path.join(suiteDirectory, "suite.json"),
    path.join(suiteDirectory, "target.json")
  );

  for (const { task } of definitions.tasks) {
    assert.equal(task.phases.length, 2, `${task.id} must retain both task phases`);
    assert.equal(task.phases[1].restartBefore, true, `${task.id} must restart before phase two`);
    assert.equal(task.phases[0].graders.commands[0].args.at(-1), "library");
    assert.equal(task.phases[1].graders.commands[0].args.at(-1), "full");
    assert.equal(task.graders.trajectory.requireThreadReuse, true);
    assert.deepEqual(task.graders.security.protectedPaths, ["SPEC.md", "test"]);
  }
});
