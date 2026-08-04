const assert = require("node:assert/strict");
const test = require("node:test");
const { validateProtocol } = require("./runtime-bundle.cjs");

const current = {
  schema: "ai.opentopia.sandbox.protocol",
  protocolVersion: 1,
  helperVersion: "0.1.0",
  features: ["run.backend", "run.runtime_roots"],
};

test("accepts a helper matching the runtime bundle protocol", () => {
  assert.equal(validateProtocol(current, { ...current }), current);
});

test("rejects a stale helper protocol before backend startup", () => {
  assert.throws(
    () => validateProtocol({ ...current, protocolVersion: 2 }, current),
    /does not match runtime bundle protocol/,
  );
});

test("rejects a helper missing a required bundle feature", () => {
  assert.throws(
    () =>
      validateProtocol(
        { ...current, features: ["run.backend"] },
        current,
      ),
    /missing runtime bundle features: run.runtime_roots/,
  );
});
