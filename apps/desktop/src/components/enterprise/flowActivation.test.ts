import assert from "node:assert/strict";
import test from "node:test";
import {
  activationSourceNodeIds,
  activationFromEditableInputs,
  createFinalActivation,
  createManualActivation,
  workflowTriggersFromActivation,
  type EditableTriggerInput,
} from "./flowActivation.ts";

test("a downstream Agent subscribes to the existing upstream Final", () => {
  const activation = createFinalActivation("agent-a");
  assert.deepEqual(activationSourceNodeIds(activation), ["agent-a"]);
  assert.equal(workflowTriggersFromActivation(activation).length, 0);
});

test("AND OR NOT expressions retain source semantics", () => {
  const inputs: EditableTriggerInput[] = [
    {
      id: "final",
      source: { kind: "agent_final", nodeId: "agent-a" },
      negated: false,
    },
    {
      id: "blocked",
      source: { kind: "agent_final", nodeId: "blocked" },
      negated: true,
    },
  ];
  const activation = activationFromEditableInputs(inputs, "and", "immediate");
  assert.equal(activation.expression.operator, "and");
  assert.deepEqual(activationSourceNodeIds(activation), ["agent-a", "blocked"]);
});

test("Flow entry policy and external Trigger remain node-owned", () => {
  const manual = createManualActivation();
  assert.equal(manual.ingressPolicy, "require_review");
  assert.deepEqual(workflowTriggersFromActivation(manual), [
    { kind: "manual" },
  ]);
});
