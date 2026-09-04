import type {
  AgentTemplateVersionView,
  FlowGraphNode,
  FlowNodeKind,
  FlowSpec,
} from "../../types";
import {
  defaultApplicationLanguage,
  interfaceMessage,
  type ApplicationLanguage,
} from "../../applicationLanguage.ts";
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
  language: ApplicationLanguage = defaultApplicationLanguage,
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
        inputText: activationLabel(
          selection.activation,
          selections,
          templates,
          language,
        ),
        kind: selection.kind,
        label: workflowNodeLabel(selection, templates),
        subtitle:
          selection.kind === "agent" && template
            ? `${template.template.templateId}@${template.template.version}`
            : selection.kind === "agent"
              ? interfaceMessage(language, "flow.canvas.unlinkedAgent")
              : selection.kind === "tool"
                ? interfaceMessage(language, "flow.canvas.action")
                : selection.kind === "output"
                  ? interfaceMessage(language, "flow.canvas.managedEndpoint")
                  : `${selection.kind} ${interfaceMessage(language, "flow.canvas.nodeSuffix")}`,
      };
    }),
  };
}

export function compiledWorkflowCanvasModel(
  graph: FlowSpec["graph"],
  language: ApplicationLanguage = defaultApplicationLanguage,
): WorkflowCanvasGraphModel {
  return {
    connections: graph.edges.map((edge, index) => ({
      allowedFields: edge.allowedFields,
      condition: edge.condition ?? "",
      dataClassification: edge.dataClassification,
      id: `compiled-edge-${index}`,
      layoutFeedback: Boolean(edge.loopPolicy),
      loopPolicy: edge.loopPolicy,
      onError: edge.onError,
      sourceId: edge.from,
      targetId: edge.to,
    })),
    nodes: graph.nodes.map((node) => ({
      id: node.id,
      inputText: workflowGraphNodeInputLabel(node, graph, language),
      kind: node.kind,
      label: node.label,
      subtitle: compiledNodeSubtitle(node, language),
    })),
  };
}

export function workflowGraphNodeInputLabel(
  node: FlowGraphNode,
  graph: FlowSpec["graph"],
  language: ApplicationLanguage = defaultApplicationLanguage,
): string {
  const activation = readNodeActivation(node);
  if (activation)
    return graphActivationLabel(activation, graph.nodes, language);

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
  return node.id === graph.entryNodeId
    ? "Flow.input"
    : interfaceMessage(language, "flow.activation.notConfigured");
}

function compiledNodeSubtitle(
  node: FlowGraphNode,
  language: ApplicationLanguage,
): string {
  const reference = node.config.reference;
  const version = node.config.templateVersion;
  if (typeof reference === "string") {
    return typeof version === "number" ? `${reference}@${version}` : reference;
  }
  if (node.kind === "tool")
    return interfaceMessage(language, "flow.canvas.action");
  if (node.kind === "output")
    return interfaceMessage(language, "flow.canvas.managedEndpoint");
  return `${node.kind} ${interfaceMessage(language, "flow.canvas.nodeSuffix")}`;
}
