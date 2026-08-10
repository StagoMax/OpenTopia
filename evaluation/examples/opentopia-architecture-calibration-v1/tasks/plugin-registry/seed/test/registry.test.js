import assert from "node:assert/strict";
import test from "node:test";
import { resolveRegistry } from "../src/registry.js";

test("falls back past disabled overrides", () => {
  const result = resolveRegistry([
    { scope: "system", plugins: [{ id: "fmt", version: "1", enabled: true, dependencies: [] }] },
    { scope: "workspace", plugins: [{ id: "fmt", version: "2", enabled: false, dependencies: [] }] },
  ]);
  assert.equal(result[0].version, "1");
});
