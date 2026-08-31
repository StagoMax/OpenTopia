import assert from "node:assert/strict";
import test from "node:test";
import {
  createFinalActivation,
  createManualActivation,
} from "./flowActivation.ts";
import {
  canConnectWorkflowNodes,
  configureWorkflowConnection,
  connectWorkflowNodes,
  disconnectWorkflowNodes,
  workflowConnections,
} from "./workflowGraphOperations.ts";
import { createDefaultEdgeConfiguration } from "./workflowNodeSelection.ts";
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
  assert.deepEqual(connectionEndpoints(connected), [
    ["a", "b"],
    ["b", "output"],
    ["a", "output"],
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
    workflowConnections(oneRemaining)
      .filter((edge) => edge.targetId === "output")
      .map((edge) => [edge.sourceId, edge.targetId]),
    [["a", "output"]],
  );
  const manual = disconnectWorkflowNodes(oneRemaining, "a", "output");
  assert.deepEqual(
    workflowConnections(manual).filter((edge) => edge.targetId === "output"),
    [],
  );
  assert.equal(manual.at(-1)?.activation.ingressPolicy, "require_review");
});

test("connection configuration follows the edge into the draft graph", () => {
  const configured = configureWorkflowConnection(graph(), "a", "b", {
    ...createDefaultEdgeConfiguration(),
    allowedFields: ["matched", "value"],
    condition: "matched == true",
  });
  const edge = workflowConnections(configured)[0];
  assert.equal(edge?.condition, "matched == true");
  assert.deepEqual(edge?.allowedFields, ["matched", "value"]);
});

test("connecting back to an upstream node creates a bounded feedback edge", () => {
  const nodes = graph();
  assert.equal(canConnectWorkflowNodes(nodes, "b", "a"), true);
  const connected = connectWorkflowNodes(nodes, "b", "a");
  const feedback = workflowConnections(connected).find(
    (edge) => edge.sourceId === "b" && edge.targetId === "a",
  );
  assert.equal(feedback?.loopPolicy?.maxIterations, 4);
  assert.equal(feedback?.loopPolicy?.onExhausted, "require_human");
});

function connectionEndpoints(nodes: WorkflowNodeSelection[]) {
  return workflowConnections(nodes).map((edge) => [
    edge.sourceId,
    edge.targetId,
  ]);
}

test("connection validation rejects self, duplicate, and output sources", () => {
  const nodes = graph();
  assert.equal(canConnectWorkflowNodes(nodes, "a", "a"), false);
  assert.equal(canConnectWorkflowNodes(nodes, "a", "b"), false);
  assert.equal(canConnectWorkflowNodes(nodes, "output", "a"), false);
  assert.equal(canConnectWorkflowNodes(nodes, "b", "a"), true);
  assert.equal(canConnectWorkflowNodes(nodes, "a", "output"), true);
});
