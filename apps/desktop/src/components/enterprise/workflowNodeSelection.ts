import type {
  AgentTemplateVersionView,
  DataClassification,
  FlowNodeKind,
  FlowSpec,
} from "../../types";
import {
  activationSourceNodeIds,
  createFinalActivation,
  createManualActivation,
  readNodeActivation,
  templateKey,
  type FlowNodeActivation,
} from "./flowActivation.ts";

export type WorkflowStateReducer = "replace" | "append" | "merge_object";

export type WorkflowStateWrite = {
  channel: string;
  reducer: WorkflowStateReducer;
  valuePath?: string;
};

export type WorkflowLoopPolicy = {
  maxIterations: number;
  continueCondition: string;
  onExhausted: "require_human" | "return_partial" | "fail";
};

export type WorkflowEdgeConfiguration = {
  allowedFields: string[];
  condition: string;
  dataClassification: DataClassification;
  loopPolicy: WorkflowLoopPolicy | null;
  onError: string | null;
};

type WorkflowNodeBase = {
  id: string;
  activation: FlowNodeActivation;
  /** Edge configuration is owned beside the incoming activation source. */
  incomingEdgeConfigs?: Record<string, WorkflowEdgeConfiguration>;
  stateWrites?: WorkflowStateWrite[];
};

export type WorkflowAgentSelection = WorkflowNodeBase & {
  kind: "agent";
  templateKey: string;
};

export type WorkflowSkillSelection = WorkflowNodeBase & {
  kind: "skill";
  label: string;
  reference: string;
};

export type WorkflowToolSelection = WorkflowNodeBase & {
  kind: "tool";
  label: string;
  parallelSafe: boolean;
  reference: string;
};

export type WorkflowConditionSelection = WorkflowNodeBase & {
  kind: "condition";
  expression: string;
  label: string;
};

export type WorkflowValidatorSelection = WorkflowNodeBase & {
  kind: "validator";
  expression: string;
  label: string;
  requiredFields: string[];
};

export type WorkflowApprovalSelection = WorkflowNodeBase & {
  kind: "approval";
  label: string;
  instructions: string;
};

export type WorkflowJoinSelection = WorkflowNodeBase & {
  kind: "join";
  label: string;
};

export type WorkflowLoopSelection = WorkflowNodeBase & {
  kind: "loop";
  feedbackSchema: Record<string, unknown>;
  label: string;
};

export type WorkflowOutputSelection = WorkflowNodeBase & {
  kind: "output";
  label: string;
};

export type WorkflowNodeSelection =
  | WorkflowAgentSelection
  | WorkflowSkillSelection
  | WorkflowToolSelection
  | WorkflowConditionSelection
  | WorkflowValidatorSelection
  | WorkflowApprovalSelection
  | WorkflowJoinSelection
  | WorkflowLoopSelection
  | WorkflowOutputSelection;

/**
 * Product-facing nodes are intentionally narrower than the runtime graph
 * kinds. Skill belongs to an Agent template, conditions and loops belong to
 * edges, and Output is managed by the editor.
 */
export const addableWorkflowNodeKinds = [
  "agent",
  "tool",
  "approval",
  "validator",
  "join",
] as const satisfies readonly FlowNodeKind[];

export type AddableWorkflowNodeKind = (typeof addableWorkflowNodeKinds)[number];

export function createDefaultEdgeConfiguration(
  loop = false,
): WorkflowEdgeConfiguration {
  return {
    allowedFields: [],
    condition: "",
    dataClassification: "internal",
    loopPolicy: loop
      ? {
          maxIterations: 4,
          continueCondition: "true",
          onExhausted: "require_human",
        }
      : null,
    onError: null,
  };
}

export function createDefaultWorkflowNodes(
  template?: AgentTemplateVersionView,
  options: { includeApproval?: boolean } = {},
): WorkflowNodeSelection[] {
  if (!template) return [];
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
  if (options.includeApproval === false) {
    return [agent, createOutputNode(createFinalActivation(agent.id))];
  }
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
  const outputIndex = selections.findIndex((node) => node.kind === "output");
  const insertAt = outputIndex >= 0 ? outputIndex : selections.length;
  const precedingNode = selections[insertAt - 1];
  const activation = precedingNode
    ? createFinalActivation(precedingNode.id)
    : createManualActivation();
  const node = createWorkflowNode(kind, activation, template);
  const next = [...selections];
  next.splice(insertAt, 0, node);
  const output = next.find((candidate) => candidate.kind === "output");
  if (!output) {
    return [...next, createOutputNode(createFinalActivation(node.id))];
  }
  return next.map((candidate) =>
    candidate.id === output.id
      ? {
          ...candidate,
          activation: createFinalActivation(node.id),
          incomingEdgeConfigs: undefined,
        }
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
  if (remaining.every((item) => item.kind === "output")) return [];
  const previous = removalIndex > 0 ? remaining[removalIndex - 1] : undefined;
  return remaining.map((item) => {
    const incomingEdgeConfigs = omitIncomingEdge(item, nodeId);
    if (!activationSourceNodeIds(item.activation).includes(nodeId)) {
      return incomingEdgeConfigs === item.incomingEdgeConfigs
        ? item
        : { ...item, incomingEdgeConfigs };
    }
    return {
      ...item,
      incomingEdgeConfigs,
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
  if (selection.kind !== "agent") return selection.label;
  return (
    templates.find((item) => templateKey(item) === selection.templateKey)
      ?.template.name ?? "未配置 Agent"
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
    const common = {
      id: node.id,
      activation,
      incomingEdgeConfigs: Object.fromEntries(
        graph.edges
          .filter((edge) => edge.to === node.id)
          .map((edge) => [edge.from, edgeConfigurationFromSpec(edge)]),
      ),
      stateWrites: readStateWrites(node.config.stateWrites),
    };
    if (node.kind === "agent") {
      const reference = node.config.reference;
      const version = node.config.templateVersion;
      if (typeof reference !== "string" || typeof version !== "number")
        return [];
      return [
        { ...common, kind: "agent", templateKey: `${reference}@${version}` },
      ];
    }
    if (node.kind === "skill") {
      return [
        {
          ...common,
          kind: "skill",
          label: node.label,
          reference: readString(node.config.reference),
        },
      ];
    }
    if (node.kind === "tool") {
      return [
        {
          ...common,
          kind: "tool",
          label: node.label,
          parallelSafe: node.config.parallelSafe === true,
          reference: readString(node.config.reference),
        },
      ];
    }
    if (node.kind === "condition") {
      return [
        {
          ...common,
          kind: "condition",
          label: node.label,
          expression: readString(node.config.expression, "true"),
        },
      ];
    }
    if (node.kind === "validator") {
      return [
        {
          ...common,
          kind: "validator",
          label: node.label,
          expression: readString(node.config.expression),
          requiredFields: readStrings(node.config.requiredFields),
        },
      ];
    }
    if (node.kind === "approval") {
      return [
        {
          ...common,
          kind: "approval",
          label: node.label,
          instructions: readString(
            node.config.instructions,
            "检查上游节点输出，确认无误后继续。",
          ),
        },
      ];
    }
    if (node.kind === "join")
      return [{ ...common, kind: "join", label: node.label }];
    if (node.kind === "loop") {
      return [
        {
          ...common,
          kind: "loop",
          label: node.label,
          feedbackSchema: readObject(node.config.feedbackSchema),
        },
      ];
    }
    return [{ ...common, kind: "output", label: node.label }];
  });
  return selections.some((node) => node.kind === "output")
    ? selections
    : [...selections, createOutputNode(createManualActivation())];
}

function createWorkflowNode(
  kind: AddableWorkflowNodeKind,
  activation: FlowNodeActivation,
  template?: AgentTemplateVersionView,
): WorkflowNodeSelection {
  const id = `${kind}-${crypto.randomUUID()}`;
  if (kind === "agent")
    return {
      id,
      kind,
      templateKey: template ? templateKey(template) : "",
      activation,
    };
  if (kind === "tool")
    return {
      id,
      kind,
      label: "Action / 操作",
      parallelSafe: false,
      reference: "",
      activation,
    };
  if (kind === "validator")
    return {
      id,
      kind,
      label: "Validator / 校验",
      expression: "",
      requiredFields: [],
      activation,
    };
  if (kind === "approval")
    return {
      id,
      kind,
      label: "Human approval / 人工审批",
      instructions: "检查上游节点输出，确认无误后继续。",
      activation,
    };
  if (kind === "join") return { id, kind, label: "Join / 汇合", activation };
  return assertNever(kind);
}

function assertNever(value: never): never {
  throw new Error(`Unsupported addable workflow node kind: ${String(value)}`);
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

function edgeConfigurationFromSpec(
  edge: FlowSpec["graph"]["edges"][number],
): WorkflowEdgeConfiguration {
  return {
    allowedFields: [...edge.allowedFields],
    condition: edge.condition ?? "",
    dataClassification: edge.dataClassification,
    loopPolicy: edge.loopPolicy ? { ...edge.loopPolicy } : null,
    onError: edge.onError,
  };
}

function omitIncomingEdge(node: WorkflowNodeSelection, sourceId: string) {
  if (!node.incomingEdgeConfigs?.[sourceId]) return node.incomingEdgeConfigs;
  const { [sourceId]: _removed, ...remaining } = node.incomingEdgeConfigs;
  return Object.keys(remaining).length > 0 ? remaining : undefined;
}

function readString(value: unknown, fallback = "") {
  return typeof value === "string" ? value : fallback;
}

function readStrings(value: unknown) {
  return Array.isArray(value)
    ? value.filter((item): item is string => typeof item === "string")
    : [];
}

function readObject(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : { type: "object" };
}

function readStateWrites(value: unknown): WorkflowStateWrite[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap((candidate) => {
    if (!candidate || typeof candidate !== "object") return [];
    const write = candidate as Partial<WorkflowStateWrite>;
    if (
      typeof write.channel !== "string" ||
      !["replace", "append", "merge_object"].includes(write.reducer ?? "")
    )
      return [];
    return [
      {
        channel: write.channel,
        reducer: write.reducer as WorkflowStateReducer,
        ...(typeof write.valuePath === "string"
          ? { valuePath: write.valuePath }
          : {}),
      },
    ];
  });
}
