import assert from "node:assert/strict";
import test from "node:test";
import path from "node:path";
import { buildExtractionPlan } from "../src/extract.js";

test("plans normalized entries", () => {
  const root = path.resolve("out");
  assert.deepEqual(buildExtractionPlan(root, [{ path: "a\\b.txt", type: "file" }]), [
    { path: "a/b.txt", type: "file", destination: path.join(root, "a", "b.txt"), target: null },
  ]);
});

test("rejects traversal", () => {
  assert.throws(() => buildExtractionPlan("out", [{ path: "../secret", type: "file" }]), /path|traversal|escape/i);
});
