import assert from "node:assert/strict";
import test from "node:test";
import {
  activeRunCount,
  guidedWorkflowSpec,
  latestPublishedTemplateCount,
  trustSignals,
} from "./model.ts";
import type { EnterpriseSnapshot } from "./store.ts";
import {
  createFinalActivation,
  createManualActivation,
} from "./flowActivation.ts";

function snapshot(): EnterpriseSnapshot {
  return {
    status: "ready",
    templates: [],
    agents: [],
    flows: [],
    runs: [],
    tasks: [],
    cases: [],
    connections: [],
    error: null,
    refreshedAt: "2026-08-21T00:00:00.000Z",
  };
}

test("overview counts only non-terminal runs", () => {
  const value = snapshot();
  value.runs = [
    { status: "running" },
    { status: "waiting_human" },
    { status: "succeeded" },
  ] as unknown as EnterpriseSnapshot["runs"];
  assert.equal(activeRunCount(value), 2);
});

test("published templates are counted by stable template identity", () => {
  const value = snapshot();
  value.templates = [
    { template: { templateId: "a", status: "published" } },
    { template: { templateId: "a", status: "published" } },
    { template: { templateId: "b", status: "draft" } },
  ] as unknown as EnterpriseSnapshot["templates"];
  assert.equal(latestPublishedTemplateCount(value), 1);
});

test("trust signals fail closed for degraded connections", () => {
  const value = snapshot();
  value.connections = [
    {
      id: "crm-production",
      name: "CRM Production",
      environment: "production",
      enabled: true,
      status: "degraded",
      lastError: "runtime handshake timed out",
      authContext: {
        account: { displayName: "Operations" },
        verification: "verified",
      },
    },
  ] as unknown as EnterpriseSnapshot["connections"];
  const signal = trustSignals(value)[0];
  assert.equal(signal?.level, "warning");
  assert.equal(signal?.findings[0]?.label, "CRM Production");
  assert.match(signal?.findings[0]?.context ?? "", /Operations.*production/);
  assert.deepEqual(
    signal?.findings[0]?.problems.map((problem) => problem.code),
    ["degraded"],
  );
  assert.match(
    signal?.findings[0]?.problems[0]?.detail ?? "",
    /runtime handshake timed out/,
  );
  assert.deepEqual(signal?.findings[0]?.target, {
    kind: "connection",
    connectionId: "crm-production",
  });
});

test("guided workflow pins reusable Agents and derives graph edges from Final subscriptions", () => {
  const reviewer = {
    template: { templateId: "reviewer", version: 3, name: "Reviewer" },
  } as unknown as EnterpriseSnapshot["templates"][number];
  const writer = {
    template: { templateId: "writer", version: 2, name: "Writer" },
  } as unknown as EnterpriseSnapshot["templates"][number];
  const spec = guidedWorkflowSpec({
    flowId: "review-flow",
    name: "Review flow",
    owner: "ops",
    outcome: "Review incoming records",
    nodes: [
      {
        id: "agent-reviewer",
        kind: "agent",
        templateKey: "reviewer@3",
        activation: createManualActivation(),
      },
      {
        id: "agent-writer",
        kind: "agent",
        templateKey: "writer@2",
        activation: createFinalActivation("agent-reviewer"),
      },
      {
        id: "review",
        kind: "approval",
        label: "Human review / 人工审查",
        instructions: "Review the output",
        activation: createFinalActivation("agent-writer"),
      },
      {
        id: "output",
        kind: "output",
        label: "Inbox output / 收件箱输出",
        activation: createFinalActivation("review"),
      },
    ],
    templates: [reviewer, writer],
  });
  assert.deepEqual(spec.graph.nodes[0]?.config, {
    reference: "reviewer",
    templateVersion: 3,
    activation: createManualActivation(),
  });
  assert.deepEqual(
    spec.graph.nodes.map((node) => node.kind),
    ["agent", "agent", "approval", "output"],
  );
  assert.deepEqual(
    spec.graph.edges.map((edge) => [edge.from, edge.to]),
    [
      ["agent-reviewer", "agent-writer"],
      ["agent-writer", "review"],
      ["review", "output"],
    ],
  );
});
