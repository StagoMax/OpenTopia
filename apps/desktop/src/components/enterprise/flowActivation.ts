import type {
  AgentTemplateVersionView,
  FlowGraphNode,
  WorkflowIngressPolicy,
  WorkflowTrigger,
} from "../../types";

export type FlowTriggerSource =
  | { kind: "flow_input" }
  | { kind: "agent_final"; nodeId: string }
  | { kind: "manual" }
  | { kind: "webhook"; triggerId: string; tokenRef: string }
  | {
      kind: "schedule";
      triggerId: string;
      intervalSeconds: number;
      nextFireAt: string;
    }
  | {
      kind: "event_subscription";
      triggerId: string;
      source: string;
      eventType: string;
    };

export type FlowTriggerExpression =
  | { operator: "source"; source: FlowTriggerSource }
  | { operator: "and" | "or"; inputs: FlowTriggerExpression[] }
  | { operator: "not"; input: FlowTriggerExpression };

export type FlowNodeActivation = {
  expression: FlowTriggerExpression;
  ingressPolicy: WorkflowIngressPolicy;
};

export type WorkflowAgentSelection = {
  id: string;
  templateKey: string;
  activation: FlowNodeActivation;
};

export type EditableTriggerInput = {
  id: string;
  source: FlowTriggerSource;
  negated: boolean;
};

export function createManualActivation(): FlowNodeActivation {
  return {
    expression: { operator: "source", source: { kind: "manual" } },
    ingressPolicy: "require_review",
  };
}

export function createFinalActivation(nodeId: string): FlowNodeActivation {
  return {
    expression: {
      operator: "source",
      source: { kind: "agent_final", nodeId },
    },
    ingressPolicy: "immediate",
  };
}

export function editableTriggerInputs(activation: FlowNodeActivation): {
  logic: "and" | "or";
  inputs: EditableTriggerInput[];
} {
  const expression = activation.expression;
  const expressions =
    expression.operator === "and" || expression.operator === "or"
      ? expression.inputs
      : [expression];
  return {
    logic: expression.operator === "and" ? "and" : "or",
    inputs: expressions.map((item) => {
      const negated = item.operator === "not";
      const sourceExpression = negated ? item.input : item;
      return {
        id: crypto.randomUUID(),
        negated,
        source:
          sourceExpression.operator === "source"
            ? sourceExpression.source
            : { kind: "flow_input" },
      };
    }),
  };
}

export function activationFromEditableInputs(
  inputs: EditableTriggerInput[],
  logic: "and" | "or",
  ingressPolicy: WorkflowIngressPolicy,
): FlowNodeActivation {
  const expressions = inputs.map<FlowTriggerExpression>((item) => {
    const expression: FlowTriggerExpression = {
      operator: "source",
      source: item.source,
    };
    return item.negated ? { operator: "not", input: expression } : expression;
  });
  return {
    expression:
      expressions.length === 1
        ? expressions[0]!
        : { operator: logic, inputs: expressions },
    ingressPolicy,
  };
}

export function activationAgentFinalNodeIds(
  activation: FlowNodeActivation,
): string[] {
  const ids = new Set<string>();
  visitExpression(activation.expression, (source) => {
    if (source.kind === "agent_final") ids.add(source.nodeId);
  });
  return [...ids];
}

export function activationHasIngress(activation: FlowNodeActivation): boolean {
  let found = false;
  visitExpression(activation.expression, (source, negated) => {
    if (!negated && source.kind !== "agent_final") found = true;
  });
  return found;
}

export function workflowTriggerFromActivation(
  activation: FlowNodeActivation,
): WorkflowTrigger | null {
  return workflowTriggersFromActivation(activation)[0] ?? null;
}

export function workflowTriggersFromActivation(
  activation: FlowNodeActivation,
): WorkflowTrigger[] {
  const triggers: WorkflowTrigger[] = [];
  visitExpression(activation.expression, (source, negated) => {
    if (negated) return;
    if (source.kind === "manual") triggers.push({ kind: "manual" });
    if (source.kind === "webhook") triggers.push(source);
    if (source.kind === "schedule") triggers.push(source);
    if (source.kind === "event_subscription") triggers.push(source);
  });
  return triggers;
}

export function activationLabel(
  activation: FlowNodeActivation,
  agents: readonly WorkflowAgentSelection[],
  templates: readonly AgentTemplateVersionView[],
): string {
  const labels: string[] = [];
  visitExpression(activation.expression, (source, negated) => {
    let label: string;
    if (source.kind === "agent_final") {
      const agent = agents.find((item) => item.id === source.nodeId);
      const template = agent
        ? templates.find((item) => templateKey(item) === agent.templateKey)
        : null;
      label = `${template?.template.name ?? source.nodeId}.Final`;
    } else if (source.kind === "event_subscription") {
      label = `${source.source}.${source.eventType}`;
    } else if (source.kind === "webhook") {
      label = "API / Webhook";
    } else if (source.kind === "schedule") {
      label = "Schedule";
    } else if (source.kind === "flow_input") {
      label = "Flow.input";
    } else {
      label = "Manual";
    }
    labels.push(negated ? `NOT ${label}` : label);
  });
  const operator = activation.expression.operator === "and" ? " AND " : " OR ";
  return labels.join(operator) || "未配置";
}

export function readNodeActivation(
  node: FlowGraphNode,
): FlowNodeActivation | null {
  const value = node.config.activation;
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const candidate = value as Partial<FlowNodeActivation>;
  return candidate.expression && candidate.ingressPolicy
    ? (candidate as FlowNodeActivation)
    : null;
}

export function templateKey(item: AgentTemplateVersionView): string {
  return `${item.template.templateId}@${item.template.version}`;
}

function visitExpression(
  expression: FlowTriggerExpression,
  visit: (source: FlowTriggerSource, negated: boolean) => void,
  negated = false,
) {
  if (expression.operator === "source") {
    visit(expression.source, negated);
    return;
  }
  if (expression.operator === "not") {
    visitExpression(expression.input, visit, !negated);
    return;
  }
  expression.inputs.forEach((input) => visitExpression(input, visit, negated));
}
