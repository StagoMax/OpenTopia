import assert from "node:assert/strict";
import test from "node:test";
import type { AgentTemplateVersionView } from "../../types";
import { activationSourceNodeIds } from "./flowActivation.ts";
import {
  addableWorkflowNodeKinds,
  addWorkflowNode,
  createDefaultWorkflowNodes,
  removeWorkflowNode,
  workflowNodesFromGraph,
} from "./workflowNodeSelection.ts";

const template = {
  template: {
    templateId: "reviewer",
    version: 3,
    name: "Reviewer",
  },
} as AgentTemplateVersionView;

test("a newly created Flow starts with an empty canvas", () => {
  assert.deepEqual(createDefaultWorkflowNodes(), []);
});

test("adding an Agent creates an unconfigured node before choosing a template", () => {
  const nodes = addWorkflowNode([], "agent");
  assert.deepEqual(
    nodes.map((node) => node.kind),
    ["agent", "output"],
  );
  const agent = nodes[0];
  assert.equal(agent?.kind, "agent");
  if (agent?.kind !== "agent") return;
  assert.equal(agent.templateKey, "");
  assert.deepEqual(activationSourceNodeIds(nodes[1]!.activation), [agent.id]);
});

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

test("default workflow can start without an approval when the creator opts out", () => {
  const nodes = createDefaultWorkflowNodes(template, {
    includeApproval: false,
  });
  assert.deepEqual(
    nodes.map((node) => node.kind),
    ["agent", "output"],
  );
  assert.deepEqual(activationSourceNodeIds(nodes[1]!.activation), [
    nodes[0]!.id,
  ]);
});

test("the product node palette excludes Agent capabilities and edge semantics", () => {
  assert.deepEqual(addableWorkflowNodeKinds, [
    "agent",
    "tool",
    "approval",
    "validator",
    "join",
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

test("a deterministic Tool step is presented as an Action node", () => {
  const initial = createDefaultWorkflowNodes(template);
  const withAction = addWorkflowNode(initial, "tool", template);
  const action = withAction.at(-2);
  assert.equal(action?.kind, "tool");
  if (action?.kind !== "tool") return;
  assert.equal(action.label, "Action / 操作");
  assert.equal(action.reference, "");
});

test("removing the entry node promotes the next node without creating a self-loop", () => {
  const initial = createDefaultWorkflowNodes(template);
  const removed = removeWorkflowNode(initial, initial[0]!.id);
  assert.equal(removed[0]?.kind, "approval");
  assert.deepEqual(activationSourceNodeIds(removed[0]!.activation), []);
});

test("removing the only configured step restores the empty canvas", () => {
  const nodes = addWorkflowNode([], "agent");
  assert.deepEqual(removeWorkflowNode(nodes, nodes[0]!.id), []);
});

test("existing graph state and edge rules survive reopening the editor", () => {
  const nodes = workflowNodesFromGraph({
    schemaVersion: 1,
    entryNodeId: "check",
    nodes: [
      {
        id: "check",
        kind: "condition",
        label: "Check",
        config: {
          expression: "score == 1",
          stateWrites: [
            { channel: "checks", reducer: "append", valuePath: "$.value" },
          ],
        },
        inputSchema: { type: "object" },
        outputSchema: { type: "object" },
      },
      {
        id: "output",
        kind: "output",
        label: "Output",
        config: {},
        inputSchema: { type: "object" },
        outputSchema: { type: "object" },
      },
    ],
    edges: [
      {
        from: "check",
        to: "output",
        condition: "matched == true",
        allowedFields: ["value"],
        dataClassification: "internal",
        onError: null,
        loopPolicy: null,
      },
    ],
  });

  const condition = nodes[0];
  assert.equal(condition?.kind, "condition");
  if (condition?.kind !== "condition") return;
  assert.equal(condition.expression, "score == 1");
  assert.deepEqual(condition.stateWrites, [
    { channel: "checks", reducer: "append", valuePath: "$.value" },
  ]);
  assert.equal(
    nodes[1]?.incomingEdgeConfigs?.check?.condition,
    "matched == true",
  );
});
