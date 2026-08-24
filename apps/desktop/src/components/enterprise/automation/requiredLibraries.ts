import type { WorkflowDeployment } from "../../../types";
import type { LibraryProviderId } from "../../../types/platform";

export function requiredDeploymentLibraryProviders(
  deployment: WorkflowDeployment | undefined,
): LibraryProviderId[] {
  const requiresSag = Object.values(
    deployment?.snapshot.compiledWorkflow.agentSpecs ?? {},
  ).some((agent) => Boolean(agent.knowledgeBinding?.namespaces.length));
  return requiresSag ? ["sag"] : [];
}
