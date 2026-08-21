import assert from "node:assert/strict";
import test from "node:test";
import {
  activeRunCount,
  guidedWorkflowSpec,
  latestPublishedTemplateCount,
  trustSignals,
} from "./model.ts";
import type { EnterpriseSnapshot } from "./store.ts";

function snapshot(): EnterpriseSnapshot {
  return {
    status: "ready",
    templates: [],
    agents: [],
    workflows: [],
    deployments: [],
    runs: [],
    tasks: [],
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
      enabled: true,
      status: "degraded",
      authContext: { verification: "verified" },
    },
  ] as unknown as EnterpriseSnapshot["connections"];
  assert.equal(trustSignals(value)[0]?.level, "warning");
});

test("guided workflow pins the selected Agent template without exposing JSON", () => {
  const template = {
    template: { templateId: "reviewer", version: 3, name: "Reviewer" },
  } as unknown as EnterpriseSnapshot["templates"][number];
  const spec = guidedWorkflowSpec({
    flowId: "review-flow",
    name: "Review flow",
    owner: "ops",
    outcome: "Review incoming records",
    template,
    requireApproval: true,
  });
  assert.deepEqual(spec.graph.nodes[0]?.config, {
    reference: "reviewer",
    templateVersion: 3,
  });
  assert.deepEqual(
    spec.graph.nodes.map((node) => node.kind),
    ["agent", "approval", "output"],
  );
});
