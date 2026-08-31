import type {
  AgentTemplateVersionView,
  FlowGraphNode,
  FlowNodeKind,
  FlowSpec,
} from "../../types";
import {
  activationLabel,
  graphActivationLabel,
  readNodeActivation,
} from "./flowActivation.ts";
import {
  workflowConnections,
  type WorkflowConnection,
} from "./workflowGraphOperations.ts";
import {
  workflowNodeLabel,
  type WorkflowNodeSelection,
} from "./workflowNodeSelection.ts";

export type WorkflowCanvasNodeModel = {
  id: string;
  inputText: string;
  kind: FlowNodeKind;
  label: string;
  subtitle: string;
};

export type WorkflowCanvasGraphModel = {
  connections: WorkflowConnection[];
  nodes: WorkflowCanvasNodeModel[];
};

export function editableWorkflowCanvasModel(
  selections: readonly WorkflowNodeSelection[],
  templates: readonly AgentTemplateVersionView[],
): WorkflowCanvasGraphModel {
  return {
    connections: workflowConnections(selections),
    nodes: selections.map((selection) => {
      const template =
        selection.kind === "agent"
          ? templates.find(
              (item) =>
                `${item.template.templateId}@${item.template.version}` ===
                selection.templateKey,
            )
          : null;
      return {
        id: selection.id,
        inputText: activationLabel(selection.activation, selections, templates),
        kind: selection.kind,
        label: workflowNodeLabel(selection, templates),
        subtitle:
          selection.kind === "agent" && template
            ? `${template.template.templateId}@${template.template.version}`
            : `${selection.kind} node`,
      };
    }),
  };
}

export function compiledWorkflowCanvasModel(
  graph: FlowSpec["graph"],
): WorkflowCanvasGraphModel {
  return {
    connections: graph.edges.map((edge, index) => ({
      id: `compiled-edge-${index}`,
      layoutFeedback: Boolean(edge.loopPolicy),
      sourceId: edge.from,
      targetId: edge.to,
    })),
    nodes: graph.nodes.map((node) => ({
      id: node.id,
      inputText: workflowGraphNodeInputLabel(node, graph),
      kind: node.kind,
      label: node.label,
      subtitle: compiledNodeSubtitle(node),
    })),
  };
}

export function workflowGraphNodeInputLabel(
  node: FlowGraphNode,
  graph: FlowSpec["graph"],
): string {
  const activation = readNodeActivation(node);
  if (activation) return graphActivationLabel(activation, graph.nodes);

  const sourceLabels = graph.edges
    .filter((edge) => edge.to === node.id)
    .map((edge) => {
      const source = graph.nodes.find(
        (candidate) => candidate.id === edge.from,
      );
      return `${source?.label ?? edge.from}.Final`;
    });
  if (sourceLabels.length > 0) {
    return sourceLabels.join(node.kind === "join" ? " AND " : " OR ");
  }
  return node.id === graph.entryNodeId ? "Flow.input" : "未配置";
}

function compiledNodeSubtitle(node: FlowGraphNode): string {
  const reference = node.config.reference;
  const version = node.config.templateVersion;
  if (typeof reference === "string") {
    return typeof version === "number" ? `${reference}@${version}` : reference;
  }
  return `${node.kind} node`;
}
