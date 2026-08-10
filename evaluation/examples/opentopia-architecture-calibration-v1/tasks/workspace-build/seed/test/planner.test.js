import assert from "node:assert/strict";
import test from "node:test";
import { planBuild } from "../src/planner.js";

test("builds dependency waves", () => {
  const result = planBuild([
    { name: "app", dependencies: ["core"], inputs: { code: "app" } },
    { name: "core", dependencies: [], inputs: { code: "core" } },
  ]);
  assert.deepEqual(result.waves, [["core"], ["app"]]);
});
