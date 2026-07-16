import assert from "node:assert/strict";
import test from "node:test";
import {
  buildReleaseWaves,
  parseWorkspace,
  renderReleasePlan,
} from "../src/planner.js";

const workspace = JSON.stringify({
  packages: [
    { name: "web", dependencies: ["api", "ui"] },
    { name: "ui", dependencies: ["core"] },
    { name: "core", dependencies: [] },
    { name: "api", dependencies: ["core"] },
  ],
});

test("parses and sorts package definitions", () => {
  assert.deepEqual(parseWorkspace(workspace), [
    { name: "api", dependencies: ["core"] },
    { name: "core", dependencies: [] },
    { name: "ui", dependencies: ["core"] },
    { name: "web", dependencies: ["api", "ui"] },
  ]);
});

test("builds deterministic parallel release waves", () => {
  assert.deepEqual(buildReleaseWaves(parseWorkspace(workspace)), [
    ["core"],
    ["api", "ui"],
    ["web"],
  ]);
});

test("rejects missing dependencies and cycles", () => {
  assert.throws(
    () => parseWorkspace('{"packages":[{"name":"app","dependencies":["missing"]}]}'),
    /missing|unknown|dependency/i,
  );
  assert.throws(
    () =>
      buildReleaseWaves([
        { name: "a", dependencies: ["b"] },
        { name: "b", dependencies: ["a"] },
      ]),
    /cycle/i,
  );
});

test("renders a numbered release plan", () => {
  assert.deepEqual(renderReleasePlan(parseWorkspace(workspace)), {
    packageCount: 4,
    waveCount: 3,
    waves: [
      { index: 1, packages: ["core"] },
      { index: 2, packages: ["api", "ui"] },
      { index: 3, packages: ["web"] },
    ],
  });
});
