import assert from "node:assert/strict";
import test from "node:test";

import type * as RequiredLibrariesModule from "./requiredLibraries";
import type { WorkflowDeployment } from "../../../types";

const { requiredDeploymentLibraryProviders } = (await import(
  "./requiredLibraries" + ".ts"
)) as typeof RequiredLibrariesModule;

function deploymentWithKnowledgeNamespaces(
  namespaces: string[] | undefined,
): WorkflowDeployment {
  return {
    id: "deployment-1",
    status: "active",
    snapshot: {
      compiledWorkflow: {
        agentSpecs: {
          evidence: namespaces
            ? ({ knowledgeBinding: { namespaces } } as never)
            : ({} as never),
        },
      } as never,
    },
  } as unknown as WorkflowDeployment;
}

test("requires SAG when a frozen deployment agent uses a knowledge namespace", () => {
  assert.deepEqual(
    requiredDeploymentLibraryProviders(
      deploymentWithKnowledgeNamespaces(["audit.work-injury"]),
    ),
    ["sag"],
  );
});

test("does not start a library provider for connection-only deployments", () => {
  assert.deepEqual(
    requiredDeploymentLibraryProviders(
      deploymentWithKnowledgeNamespaces(undefined),
    ),
    [],
  );
  assert.deepEqual(requiredDeploymentLibraryProviders(undefined), []);
});
