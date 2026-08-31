import assert from "node:assert/strict";
import test from "node:test";
import type { FlowGraphEdge, FlowGraphNode, FlowSpec } from "../../types";
import { automaticWorkflowPositions } from "./workflowCanvasLayout.ts";
import { compiledWorkflowCanvasModel } from "./workflowCanvasModel.ts";
import { createDefaultEdgeConfiguration } from "./workflowNodeSelection.ts";

const objectSchema = { type: "object" };

function node(
  id: string,
  label: string,
  kind: FlowGraphNode["kind"],
  config: Record<string, unknown> = {},
): FlowGraphNode {
  return {
    id,
    label,
    kind,
    config,
    inputSchema: objectSchema,
    outputSchema: objectSchema,
  };
}

function edge(from: string, to: string): FlowGraphEdge {
  return {
    from,
    to,
    condition: null,
    allowedFields: [],
    dataClassification: "confidential",
    onError: null,
    loopPolicy: null,
  };
}

function auditGraph(): FlowSpec["graph"] {
  return {
    schemaVersion: 1,
    entryNodeId: "domain_audit",
    nodes: [
      node("domain_audit", "Domain audit", "agent", {
        reference: "audit.credit.domain",
        templateVersion: 6,
        activation: {
          expression: {
            operator: "source",
            source: {
              kind: "event_subscription",
              triggerId: "credit-trigger",
              source: "audit.credit-review",
              eventType: "case.submitted",
            },
          },
          ingressPolicy: "require_review",
        },
      }),
      node("sag_evidence", "SAG evidence", "agent", {
        reference: "audit.credit.evidence",
        templateVersion: 6,
        activation: {
          expression: {
            operator: "source",
            source: { kind: "agent_final", nodeId: "domain_audit" },
          },
          ingressPolicy: "immediate",
        },
      }),
      node("evidence_validator", "Evidence check", "validator"),
      node("review_gate", "Human review", "approval"),
      node("review_context", "Review context", "join"),
      node("review_report", "Review report", "agent", {
        reference: "audit.credit.report",
        templateVersion: 6,
      }),
      node("output", "Output", "output"),
    ],
    edges: [
      edge("domain_audit", "sag_evidence"),
      edge("sag_evidence", "evidence_validator"),
      edge("evidence_validator", "review_gate"),
      edge("domain_audit", "review_context"),
      edge("sag_evidence", "review_context"),
      edge("evidence_validator", "review_context"),
      edge("review_gate", "review_context"),
      edge("review_context", "review_report"),
      edge("review_report", "output"),
    ],
  };
}

test("compiled canvas preserves every runtime node and graph edge", () => {
  const graph = auditGraph();
  const model = compiledWorkflowCanvasModel(graph);

  assert.deepEqual(
    model.nodes.map((item) => [item.id, item.kind]),
    graph.nodes.map((item) => [item.id, item.kind]),
  );
  assert.deepEqual(
    model.connections.map((item) => [item.sourceId, item.targetId]),
    graph.edges.map((item) => [item.from, item.to]),
  );
  assert.equal(
    model.nodes.find((item) => item.id === "domain_audit")?.inputText,
    "audit.credit-review.case.submitted",
  );
  assert.equal(
    model.nodes.find((item) => item.id === "review_context")?.inputText,
    "Domain audit.Final AND SAG evidence.Final AND Evidence check.Final AND Human review.Final",
  );
});

test("compiled graph layout ranks the complete dependency chain", () => {
  const model = compiledWorkflowCanvasModel(auditGraph());
  const positions = automaticWorkflowPositions(model.nodes, [
    ...model.connections,
    {
      ...createDefaultEdgeConfiguration(true),
      id: "feedback",
      layoutFeedback: true,
      sourceId: "output",
      targetId: "domain_audit",
    },
  ]);

  assert.ok(positions.sag_evidence!.x > positions.domain_audit!.x);
  assert.ok(positions.evidence_validator!.x > positions.sag_evidence!.x);
  assert.ok(positions.review_gate!.x > positions.evidence_validator!.x);
  assert.ok(positions.review_context!.x > positions.review_gate!.x);
  assert.ok(positions.review_report!.x > positions.review_context!.x);
  assert.ok(positions.output!.x > positions.review_report!.x);
});
