import assert from "node:assert/strict";
import test from "node:test";
import {
  createFinalActivation,
  createManualActivation,
} from "./flowActivation.ts";
import {
  canConnectWorkflowNodes,
  connectWorkflowNodes,
  disconnectWorkflowNodes,
  workflowConnections,
} from "./workflowGraphOperations.ts";
import type { WorkflowNodeSelection } from "./workflowNodeSelection.ts";

function graph(): WorkflowNodeSelection[] {
  return [
    {
      id: "a",
      kind: "approval",
      label: "A",
      instructions: "",
      activation: createManualActivation(),
    },
    {
      id: "b",
      kind: "approval",
      label: "B",
      instructions: "",
      activation: createFinalActivation("a"),
    },
    {
      id: "output",
      kind: "output",
      label: "Output",
      activation: createFinalActivation("b"),
    },
  ];
}

test("connect adds a second upstream subscription without duplicating edges", () => {
  const nodes = graph();
  const connected = connectWorkflowNodes(nodes, "a", "output");
  assert.deepEqual(workflowConnections(connected), [
    { sourceId: "a", targetId: "b" },
    { sourceId: "b", targetId: "output" },
    { sourceId: "a", targetId: "output" },
  ]);
  assert.equal(connectWorkflowNodes(connected, "a", "output").length, 3);
  assert.deepEqual(
    workflowConnections(connectWorkflowNodes(connected, "a", "output")),
    workflowConnections(connected),
  );
});

test("disconnect collapses activation expressions and falls back to manual", () => {
  const nodes = connectWorkflowNodes(graph(), "a", "output");
  const oneRemaining = disconnectWorkflowNodes(nodes, "b", "output");
  assert.deepEqual(
    workflowConnections(oneRemaining).filter(
      (edge) => edge.targetId === "output",
    ),
    [{ sourceId: "a", targetId: "output" }],
  );
  const manual = disconnectWorkflowNodes(oneRemaining, "a", "output");
  assert.deepEqual(
    workflowConnections(manual).filter((edge) => edge.targetId === "output"),
    [],
  );
  assert.equal(manual.at(-1)?.activation.ingressPolicy, "require_review");
});

test("connection validation rejects self, duplicate, output source, and cycles", () => {
  const nodes = graph();
  assert.equal(canConnectWorkflowNodes(nodes, "a", "a"), false);
  assert.equal(canConnectWorkflowNodes(nodes, "a", "b"), false);
  assert.equal(canConnectWorkflowNodes(nodes, "output", "a"), false);
  assert.equal(canConnectWorkflowNodes(nodes, "b", "a"), false);
  assert.equal(canConnectWorkflowNodes(nodes, "a", "output"), true);
});
