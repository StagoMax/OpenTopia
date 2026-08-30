import type { AgentTemplateVersionView, FlowSpec } from "../../types";
import {
  activationSourceNodeIds,
  createFinalActivation,
  createManualActivation,
  readNodeActivation,
  templateKey,
  type FlowNodeActivation,
} from "./flowActivation.ts";

type WorkflowNodeBase = {
  id: string;
  activation: FlowNodeActivation;
};

export type WorkflowAgentSelection = WorkflowNodeBase & {
  kind: "agent";
  templateKey: string;
};

export type WorkflowApprovalSelection = WorkflowNodeBase & {
  kind: "approval";
  label: string;
  instructions: string;
};

export type WorkflowOutputSelection = WorkflowNodeBase & {
  kind: "output";
  label: string;
};

export type WorkflowNodeSelection =
  WorkflowAgentSelection | WorkflowApprovalSelection | WorkflowOutputSelection;

export type AddableWorkflowNodeKind = "agent" | "approval";

export function createDefaultWorkflowNodes(
  template?: AgentTemplateVersionView,
): WorkflowNodeSelection[] {
  if (!template) {
    return [createOutputNode(createManualActivation())];
  }

  const agent: WorkflowAgentSelection = {
    id: `agent-${crypto.randomUUID()}`,
    kind: "agent",
    templateKey: templateKey(template),
    activation: createManualActivation(),
  };
  const approval: WorkflowApprovalSelection = {
    id: `approval-${crypto.randomUUID()}`,
    kind: "approval",
    label: "Human approval / 人工审批",
    instructions: "检查 Agent 输出，确认无误后继续。",
    activation: createFinalActivation(agent.id),
  };
  return [
    agent,
    approval,
    createOutputNode(createFinalActivation(approval.id)),
  ];
}

export function addWorkflowNode(
  selections: readonly WorkflowNodeSelection[],
  kind: AddableWorkflowNodeKind,
  template?: AgentTemplateVersionView,
): WorkflowNodeSelection[] {
  if (kind === "agent" && !template) return [...selections];

  const outputIndex = selections.findIndex((node) => node.kind === "output");
  const insertAt = outputIndex >= 0 ? outputIndex : selections.length;
  const precedingNode = selections[insertAt - 1];
  const activation = precedingNode
    ? createFinalActivation(precedingNode.id)
    : createManualActivation();
  const node: WorkflowNodeSelection =
    kind === "agent"
      ? {
          id: `agent-${crypto.randomUUID()}`,
          kind: "agent",
          templateKey: templateKey(template!),
          activation,
        }
      : {
          id: `approval-${crypto.randomUUID()}`,
          kind: "approval",
          label: "Human approval / 人工审批",
          instructions: "检查上游节点输出，确认无误后继续。",
          activation,
        };

  const next = [...selections];
  next.splice(insertAt, 0, node);
  const output = next.find((candidate) => candidate.kind === "output");
  return next.map((candidate) =>
    candidate.id === output?.id
      ? { ...candidate, activation: createFinalActivation(node.id) }
      : candidate,
  );
}

export function removeWorkflowNode(
  selections: readonly WorkflowNodeSelection[],
  nodeId: string,
): WorkflowNodeSelection[] {
  const removalIndex = selections.findIndex((item) => item.id === nodeId);
  if (removalIndex < 0 || selections[removalIndex]?.kind === "output") {
    return [...selections];
  }
  const remaining = selections.filter((item) => item.id !== nodeId);
  const previous = removalIndex > 0 ? remaining[removalIndex - 1] : undefined;
  return remaining.map((item) => {
    if (!activationSourceNodeIds(item.activation).includes(nodeId)) return item;
    return {
      ...item,
      activation: previous
        ? createFinalActivation(previous.id)
        : createManualActivation(),
    };
  });
}

export function workflowNodeLabel(
  selection: WorkflowNodeSelection,
  templates: readonly AgentTemplateVersionView[],
): string {
  if (selection.kind === "approval" || selection.kind === "output") {
    return selection.label;
  }
  return (
    templates.find((item) => templateKey(item) === selection.templateKey)
      ?.template.name ?? "选择 Agent"
  );
}

export function workflowNodesFromSpec(spec: FlowSpec): WorkflowNodeSelection[] {
  return workflowNodesFromGraph(spec.graph);
}

export function workflowNodesFromGraph(
  graph: FlowSpec["graph"],
): WorkflowNodeSelection[] {
  const selections = graph.nodes.flatMap<WorkflowNodeSelection>((node) => {
    const incoming = graph.edges.find((edge) => edge.to === node.id);
    const activation =
      readNodeActivation(node) ??
      (incoming
        ? createFinalActivation(incoming.from)
        : createManualActivation());
    if (node.kind === "agent") {
      const reference = node.config.reference;
      const version = node.config.templateVersion;
      if (typeof reference !== "string" || typeof version !== "number") {
        return [];
      }
      return [
        {
          id: node.id,
          kind: "agent",
          templateKey: `${reference}@${version}`,
          activation,
        },
      ];
    }
    if (node.kind === "approval") {
      return [
        {
          id: node.id,
          kind: "approval",
          label: node.label,
          instructions:
            typeof node.config.instructions === "string"
              ? node.config.instructions
              : "检查上游节点输出，确认无误后继续。",
          activation,
        },
      ];
    }
    if (node.kind === "output") {
      return [
        {
          id: node.id,
          kind: "output",
          label: node.label,
          activation,
        },
      ];
    }
    return [];
  });
  return selections.some((node) => node.kind === "output")
    ? selections
    : [...selections, createOutputNode(createManualActivation())];
}

function createOutputNode(
  activation: FlowNodeActivation,
): WorkflowOutputSelection {
  return {
    id: "output",
    kind: "output",
    label: "Inbox output / 收件箱输出",
    activation,
  };
}
