import assert from "node:assert/strict";
import test from "node:test";

import type { WorkflowDeployment } from "./types";
import type * as ModelModule from "./components/workflowDeployments/model";

const {
  deploymentMatchesQuery,
  deploymentStatusLabel,
  shortHash,
  sortWorkflowDeployments,
} = (await import(
  "./components/workflowDeployments/model" + ".ts"
)) as typeof ModelModule;

function deployment(
  id: string,
  status: WorkflowDeployment["status"],
  updatedAt: string,
): WorkflowDeployment {
  return {
    id,
    status,
    updatedAt,
    name: `Deployment ${id}`,
    environment: id === "prod" ? "production" : "staging",
    snapshot: {
      compiledWorkflow: {
        flowId: `flow-${id}`,
        flowVersion: 2,
      },
    },
  } as WorkflowDeployment;
}

test("sorts active deployments before disabled ones, newest first", () => {
  const sorted = sortWorkflowDeployments([
    deployment("old", "active", "2026-01-01T00:00:00Z"),
    deployment("disabled", "disabled", "2026-08-01T00:00:00Z"),
    deployment("new", "active", "2026-07-01T00:00:00Z"),
  ]);
  assert.deepEqual(
    sorted.map((item) => item.id),
    ["new", "old", "disabled"],
  );
});

test("matches deployment identity, environment and exact Flow version", () => {
  const item = deployment("prod", "active", "2026-01-01T00:00:00Z");
  assert.equal(deploymentMatchesQuery(item, "PRODUCTION"), true);
  assert.equal(deploymentMatchesQuery(item, "flow-prod@2"), true);
  assert.equal(deploymentMatchesQuery(item, "missing"), false);
});

test("presents bilingual status and compact hashes", () => {
  assert.equal(deploymentStatusLabel("active"), "Active / 运行中");
  assert.equal(deploymentStatusLabel("disabled"), "Disabled / 已停用");
  assert.equal(shortHash("12345678901234567890"), "123456789012345678…");
});
