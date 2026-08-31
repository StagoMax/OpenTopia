import type { AgentTemplateVersionView, FlowSpec } from "../../types";
import { activationHasIngress } from "./flowActivation.ts";
import { workflowConnections } from "./workflowGraphOperations.ts";
import type { WorkflowNodeSelection } from "./workflowNodeSelection.ts";
import type { EnterpriseSnapshot } from "./store";
import {
  connectionAccountLabel,
  connectionProblems,
  type ConnectionProblem,
} from "../connections/model.ts";

export type TrustSignalFinding = {
  id: string;
  label: string;
  context: string;
  problems: readonly ConnectionProblem[];
  target: {
    kind: "connection";
    connectionId: string;
  };
};

export type TrustSignal = {
  id: string;
  level: "attention" | "warning" | "healthy";
  title: string;
  detail: string;
  findings: readonly TrustSignalFinding[];
};

export const DEFAULT_GUIDED_FLOW_BUDGET: FlowSpec["budget"] = {
  maxNodeExecutions: 24,
  maxToolCalls: 40,
  maxDurationSeconds: 1800,
  maxLoopIterations: 4,
};

export const DEFAULT_GUIDED_FLOW_RISK_CLASS: FlowSpec["riskClass"] = "medium";

export function activeRunCount(snapshot: EnterpriseSnapshot): number {
  return snapshot.runs.filter(
    (run) => !["succeeded", "failed", "cancelled"].includes(run.status),
  ).length;
}

export function latestPublishedTemplateCount(
  snapshot: EnterpriseSnapshot,
): number {
  return latestPublishedTemplateVersions(snapshot.templates).length;
}

export function latestPublishedTemplateVersions(
  templates: readonly AgentTemplateVersionView[],
): AgentTemplateVersionView[] {
  const latest = new Map<string, AgentTemplateVersionView>();
  for (const view of templates) {
    if (view.template.status !== "published") continue;
    const current = latest.get(view.template.templateId);
    if (!current || view.template.version > current.template.version) {
      latest.set(view.template.templateId, view);
    }
  }
  return [...latest.values()];
}

export function trustSignals(snapshot: EnterpriseSnapshot): TrustSignal[] {
  const signals: TrustSignal[] = [];
  const connectionFindings = snapshot.connections.flatMap((connection) => {
    const problems = connectionProblems(connection);
    return problems.length > 0
      ? [
          {
            id: `connection:${connection.id}`,
            label: connection.name,
            context: `${connectionAccountLabel(connection)} · ${connection.environment} · ${shortId(connection.id)}`,
            problems,
            target: {
              kind: "connection" as const,
              connectionId: connection.id,
            },
          },
        ]
      : [];
  });
  if (connectionFindings.length) {
    signals.push({
      id: "connections",
      level: "warning",
      title: `${connectionFindings.length} 个 Connection 需要处理`,
      detail: "停用、降级或认证失效的 Connection 会在调用边界 fail closed。",
      findings: connectionFindings,
    });
  } else {
    signals.push({
      id: "connections",
      level: "healthy",
      title: "Connection 运行许可正常",
      detail: "已启用 Connection 均处于 ready，认证状态可执行。",
      findings: [],
    });
  }

  const failedRuns = snapshot.runs.filter((run) => run.status === "failed");
  if (failedRuns.length) {
    signals.push({
      id: "failed-runs",
      level: "warning",
      title: `${failedRuns.length} 个 Run 失败`,
      detail: "检查 Node Trace 和 Activity Receipt 后再决定是否恢复。",
      findings: [],
    });
  }

  if (snapshot.tasks.length) {
    signals.push({
      id: "human-tasks",
      level: "attention",
      title: `${snapshot.tasks.length} 个 HumanTask 等待人工处理`,
      detail: "审批、输入、重连、核对和输出审查统一由 Inbox 处理。",
      findings: [],
    });
  } else {
    signals.push({
      id: "human-tasks",
      level: "healthy",
      title: "Inbox 已清空",
      detail: "当前没有待处理的人工控制点。",
      findings: [],
    });
  }

  const draftTemplates = snapshot.templates.filter(
    (view) => view.template.status === "draft",
  ).length;
  if (draftTemplates) {
    signals.push({
      id: "draft-templates",
      level: "attention",
      title: `${draftTemplates} 个 Agent 模板版本尚未发布`,
      detail:
        "只有已发布且固定 content hash 的 Agent 版本能进入 Flow Revision。",
      findings: [],
    });
  }
  return signals;
}

export function shortId(value: string): string {
  return value.length > 12 ? `${value.slice(0, 8)}…` : value;
}

export function guidedWorkflowSpec(input: {
  flowId: string;
  name: string;
  owner: string;
  outcome: string;
  nodes: WorkflowNodeSelection[];
  templates: AgentTemplateVersionView[];
  budget?: FlowSpec["budget"];
  riskClass?: FlowSpec["riskClass"];
}): FlowSpec {
  const objectSchema = { type: "object" };
  const nodes: FlowSpec["graph"]["nodes"] = input.nodes.map((selection) => {
    if (selection.kind === "agent") {
      const template = input.templates.find(
        (item) =>
          `${item.template.templateId}@${item.template.version}` ===
          selection.templateKey,
      );
      if (!template) {
        throw new Error(
          `Agent template ${selection.templateKey} is unavailable`,
        );
      }
      return {
        id: selection.id,
        label: template.template.name,
        kind: selection.kind,
        config: {
          reference: template.template.templateId,
          templateVersion: template.template.version,
          activation: selection.activation,
          ...(selection.stateWrites?.length
            ? { stateWrites: selection.stateWrites }
            : {}),
        },
        inputSchema: objectSchema,
        outputSchema: objectSchema,
      };
    }
    const nodeConfig = selectionConfig(selection);
    return {
      id: selection.id,
      label: selection.label,
      kind: selection.kind,
      config: {
        activation: selection.activation,
        ...nodeConfig,
        ...(selection.stateWrites?.length
          ? { stateWrites: selection.stateWrites }
          : {}),
      },
      inputSchema: objectSchema,
      outputSchema: objectSchema,
    };
  });
  const edges = workflowConnections(input.nodes).map((edge) => ({
    from: edge.sourceId,
    to: edge.targetId,
    condition: edge.condition.trim() || null,
    allowedFields: edge.allowedFields,
    dataClassification: edge.dataClassification,
    onError: edge.onError,
    loopPolicy: edge.loopPolicy,
  }));
  const entryNodeId =
    input.nodes.find((selection) => activationHasIngress(selection.activation))
      ?.id ??
    input.nodes[0]?.id ??
    "output";
  return {
    flowId: input.flowId,
    name: input.name,
    description: input.outcome,
    owner: input.owner,
    categories: ["guided"],
    source: { kind: "natural_language", description: input.outcome },
    inputSchema: objectSchema,
    outputSchema: objectSchema,
    graph: {
      schemaVersion: 1,
      entryNodeId,
      nodes,
      edges,
    },
    requestedCapabilities: {
      allowAllTools: false,
      tools: requestedNodeReferences(input.nodes, "tool"),
      allowAllSkills: false,
      skills: requestedNodeReferences(input.nodes, "skill"),
      allowAllPlugins: false,
      plugins: [],
      allowAllMcpServers: false,
      mcpServers: [],
      allowAllWorkspaceRoots: false,
      workspaceRoots: [],
    },
    budget: { ...(input.budget ?? DEFAULT_GUIDED_FLOW_BUDGET) },
    riskClass: input.riskClass ?? DEFAULT_GUIDED_FLOW_RISK_CLASS,
    pendingDecisions: [],
  };
}

function selectionConfig(
  selection: Exclude<WorkflowNodeSelection, { kind: "agent" }>,
): Record<string, unknown> {
  if (selection.kind === "skill") {
    return { reference: selection.reference.trim() };
  }
  if (selection.kind === "tool") {
    return {
      reference: selection.reference.trim(),
      parallelSafe: selection.parallelSafe,
      sideEffect: "none",
    };
  }
  if (selection.kind === "condition") {
    return { expression: selection.expression.trim() || "true" };
  }
  if (selection.kind === "validator") {
    return {
      ...(selection.expression.trim()
        ? { expression: selection.expression.trim() }
        : {}),
      requiredFields: selection.requiredFields,
    };
  }
  if (selection.kind === "approval") {
    return { instructions: selection.instructions };
  }
  if (selection.kind === "loop") {
    return { feedbackSchema: selection.feedbackSchema };
  }
  return {};
}

function requestedNodeReferences(
  nodes: WorkflowNodeSelection[],
  kind: "skill" | "tool",
) {
  return nodes.flatMap((node) => {
    if (node.kind !== kind || !node.reference.trim()) return [];
    return [node.reference.trim()];
  });
}
