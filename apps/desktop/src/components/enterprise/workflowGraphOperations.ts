import {
  createFinalActivation,
  createManualActivation,
  type FlowNodeActivation,
  type FlowTriggerExpression,
} from "./flowActivation.ts";
import type { WorkflowNodeSelection } from "./workflowNodeSelection.ts";
import {
  createDefaultEdgeConfiguration,
  type WorkflowEdgeConfiguration,
} from "./workflowNodeSelection.ts";

export type WorkflowConnection = {
  id?: string;
  layoutFeedback?: boolean;
  sourceId: string;
  targetId: string;
} & WorkflowEdgeConfiguration;

export function workflowConnections(
  nodes: readonly WorkflowNodeSelection[],
): WorkflowConnection[] {
  return nodes.flatMap((target) =>
    sourceNodeIds(target.activation.expression).map((sourceId) => {
      const configuration =
        target.incomingEdgeConfigs?.[sourceId] ??
        createDefaultEdgeConfiguration();
      return {
        ...configuration,
        layoutFeedback: Boolean(configuration.loopPolicy),
        sourceId,
        targetId: target.id,
      };
    }),
  );
}

export function canConnectWorkflowNodes(
  nodes: readonly WorkflowNodeSelection[],
  sourceId: string | null | undefined,
  targetId: string | null | undefined,
): boolean {
  if (!sourceId || !targetId || sourceId === targetId) return false;
  const source = nodes.find((node) => node.id === sourceId);
  const target = nodes.find((node) => node.id === targetId);
  if (!source || !target || source.kind === "output") return false;
  if (
    workflowConnections(nodes).some(
      (edge) => edge.sourceId === sourceId && edge.targetId === targetId,
    )
  ) {
    return false;
  }

  return true;
}

export function connectWorkflowNodes(
  nodes: readonly WorkflowNodeSelection[],
  sourceId: string,
  targetId: string,
): WorkflowNodeSelection[] {
  if (!canConnectWorkflowNodes(nodes, sourceId, targetId)) return [...nodes];
  const loop = hasPath(nodes, targetId, sourceId);
  return nodes.map((node) =>
    node.id === targetId
      ? {
          ...node,
          activation: addFinalSource(node.activation, sourceId),
          incomingEdgeConfigs: {
            ...node.incomingEdgeConfigs,
            [sourceId]:
              node.incomingEdgeConfigs?.[sourceId] ??
              createDefaultEdgeConfiguration(loop),
          },
        }
      : node,
  );
}

export function configureWorkflowConnection(
  nodes: readonly WorkflowNodeSelection[],
  sourceId: string,
  targetId: string,
  configuration: WorkflowEdgeConfiguration,
): WorkflowNodeSelection[] {
  if (
    !workflowConnections(nodes).some(
      (edge) => edge.sourceId === sourceId && edge.targetId === targetId,
    )
  ) {
    return [...nodes];
  }
  return nodes.map((node) =>
    node.id === targetId
      ? {
          ...node,
          incomingEdgeConfigs: {
            ...node.incomingEdgeConfigs,
            [sourceId]: configuration,
          },
        }
      : node,
  );
}

export function disconnectWorkflowNodes(
  nodes: readonly WorkflowNodeSelection[],
  sourceId: string,
  targetId: string,
): WorkflowNodeSelection[] {
  return nodes.map((node) => {
    if (node.id !== targetId) return node;
    const expression = removeFinalSource(node.activation.expression, sourceId);
    return {
      ...node,
      incomingEdgeConfigs: withoutKey(node.incomingEdgeConfigs, sourceId),
      activation: expression
        ? { ...node.activation, expression }
        : createManualActivation(),
    };
  });
}

function withoutKey<T>(
  record: Record<string, T> | undefined,
  key: string,
): Record<string, T> | undefined {
  if (!record?.[key]) return record;
  const { [key]: _removed, ...remaining } = record;
  return Object.keys(remaining).length > 0 ? remaining : undefined;
}

function addFinalSource(
  activation: FlowNodeActivation,
  sourceId: string,
): FlowNodeActivation {
  if (sourceNodeIds(activation.expression).length === 0) {
    return createFinalActivation(sourceId);
  }

  const source: FlowTriggerExpression = {
    operator: "source",
    source: { kind: "agent_final", nodeId: sourceId },
  };
  const expression = activation.expression;
  if (expression.operator === "and" || expression.operator === "or") {
    return {
      ...activation,
      expression: { ...expression, inputs: [...expression.inputs, source] },
    };
  }
  return {
    ...activation,
    expression: { operator: "or", inputs: [expression, source] },
  };
}

function removeFinalSource(
  expression: FlowTriggerExpression,
  sourceId: string,
): FlowTriggerExpression | null {
  if (expression.operator === "source") {
    return expression.source.kind === "agent_final" &&
      expression.source.nodeId === sourceId
      ? null
      : expression;
  }
  if (expression.operator === "not") {
    const input = removeFinalSource(expression.input, sourceId);
    return input ? { ...expression, input } : null;
  }
  const inputs = expression.inputs.flatMap((input) => {
    const next = removeFinalSource(input, sourceId);
    return next ? [next] : [];
  });
  if (inputs.length === 0) return null;
  if (inputs.length === 1) return inputs[0]!;
  return { ...expression, inputs };
}

function sourceNodeIds(expression: FlowTriggerExpression): string[] {
  if (expression.operator === "source") {
    return expression.source.kind === "agent_final"
      ? [expression.source.nodeId]
      : [];
  }
  if (expression.operator === "not") return sourceNodeIds(expression.input);
  return expression.inputs.flatMap(sourceNodeIds);
}

function hasPath(
  nodes: readonly WorkflowNodeSelection[],
  fromId: string,
  toId: string,
): boolean {
  const adjacency = new Map<string, string[]>();
  for (const edge of workflowConnections(nodes)) {
    adjacency.set(edge.sourceId, [
      ...(adjacency.get(edge.sourceId) ?? []),
      edge.targetId,
    ]);
  }
  const pending = [fromId];
  const visited = new Set<string>();
  while (pending.length > 0) {
    const current = pending.pop()!;
    if (current === toId) return true;
    if (visited.has(current)) continue;
    visited.add(current);
    pending.push(...(adjacency.get(current) ?? []));
  }
  return false;
}
