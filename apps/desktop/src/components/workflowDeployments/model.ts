import type { FlowDefinition, WorkflowDeployment } from "../../types";

export function sortWorkflowDeployments(
  deployments: readonly WorkflowDeployment[],
): WorkflowDeployment[] {
  return [...deployments].sort((left, right) => {
    const statusOrder =
      Number(left.status === "disabled") - Number(right.status === "disabled");
    return statusOrder || right.updatedAt.localeCompare(left.updatedAt);
  });
}

export function sortFlowDefinitions(
  definitions: readonly FlowDefinition[],
): FlowDefinition[] {
  return [...definitions].sort(
    (left, right) =>
      left.name.localeCompare(right.name) || right.version - left.version,
  );
}

export function deploymentMatchesQuery(
  deployment: WorkflowDeployment,
  query: string,
): boolean {
  const normalized = query.trim().toLocaleLowerCase();
  if (!normalized) return true;
  const workflow = deployment.snapshot.compiledWorkflow;
  return [
    deployment.name,
    deployment.environment,
    workflow.flowId,
    `${workflow.flowId}@${workflow.flowVersion}`,
  ].some((value) => value.toLocaleLowerCase().includes(normalized));
}

export function deploymentStatusLabel(
  status: WorkflowDeployment["status"],
): string {
  return status === "active" ? "Active / 运行中" : "Disabled / 已停用";
}

export function shortHash(value: string): string {
  return value.length > 18 ? `${value.slice(0, 18)}…` : value;
}
