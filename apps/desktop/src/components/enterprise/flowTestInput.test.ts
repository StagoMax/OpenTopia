import assert from "node:assert/strict";
import test from "node:test";
import { parseFlowTestInput } from "./flowTestInput.ts";

test("parses and formats valid Flow test input", () => {
  assert.deepEqual(parseFlowTestInput('{"customerId":"C-1001"}'), {
    ok: true,
    input: { customerId: "C-1001" },
    formatted: '{\n  "customerId": "C-1001"\n}',
  });
});

test("reports malformed JSON before starting a Test Run", () => {
  const result = parseFlowTestInput("{");
  assert.equal(result.ok, false);
  if (result.ok) return;
  assert.match(result.error, /JSON/);
});

test("validates test input against the Flow input schema", () => {
  const result = parseFlowTestInput('{"amount":"invalid"}', {
    type: "object",
    required: ["amount"],
    properties: { amount: { type: "number" } },
  });
  assert.equal(result.ok, false);
  if (result.ok) return;
  assert.match(result.error, /number/);
});
