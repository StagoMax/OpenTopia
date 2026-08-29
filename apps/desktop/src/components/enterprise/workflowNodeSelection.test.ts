import assert from "node:assert/strict";
import test from "node:test";
import type { AgentTemplateVersionView } from "../../types";
import { activationSourceNodeIds } from "./flowActivation.ts";
import {
  addWorkflowNode,
  createDefaultWorkflowNodes,
  removeWorkflowNode,
} from "./workflowNodeSelection.ts";

const template = {
  template: {
    templateId: "reviewer",
    version: 3,
    name: "Reviewer",
  },
} as AgentTemplateVersionView;

test("default workflow models Agent, Approval, and Output as first-class nodes", () => {
  const nodes = createDefaultWorkflowNodes(template);
  assert.deepEqual(
    nodes.map((node) => node.kind),
    ["agent", "approval", "output"],
  );
  assert.deepEqual(activationSourceNodeIds(nodes[1]!.activation), [
    nodes[0]!.id,
  ]);
  assert.deepEqual(activationSourceNodeIds(nodes[2]!.activation), [
    nodes[1]!.id,
  ]);
});

test("adding and removing Approval nodes keeps the downstream graph connected", () => {
  const initial = createDefaultWorkflowNodes(template);
  const withApproval = addWorkflowNode(initial, "approval", template);
  const inserted = withApproval.at(-2)!;
  const output = withApproval.at(-1)!;
  assert.equal(inserted.kind, "approval");
  assert.deepEqual(activationSourceNodeIds(output.activation), [inserted.id]);

  const removed = removeWorkflowNode(withApproval, inserted.id);
  const repairedOutput = removed.at(-1)!;
  assert.deepEqual(activationSourceNodeIds(repairedOutput.activation), [
    removed.at(-2)!.id,
  ]);
});

test("removing the entry node promotes the next node without creating a self-loop", () => {
  const initial = createDefaultWorkflowNodes(template);
  const removed = removeWorkflowNode(initial, initial[0]!.id);
  assert.equal(removed[0]?.kind, "approval");
  assert.deepEqual(activationSourceNodeIds(removed[0]!.activation), []);
});
