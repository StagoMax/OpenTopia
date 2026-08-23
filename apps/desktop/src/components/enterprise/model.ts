import type { AgentTemplateVersionView, FlowSpec } from "../../types";
import type { EnterpriseSnapshot } from "./store";

export type TrustSignal = {
  id: string;
  level: "attention" | "warning" | "healthy";
  title: string;
  detail: string;
};

export function activeRunCount(snapshot: EnterpriseSnapshot): number {
  return snapshot.runs.filter(
    (run) => !["succeeded", "failed", "cancelled"].includes(run.status),
  ).length;
}

export function latestPublishedTemplateCount(
  snapshot: EnterpriseSnapshot,
): number {
  return new Set(
    snapshot.templates
      .filter((view) => view.template.status === "published")
      .map((view) => view.template.templateId),
  ).size;
}

export function trustSignals(snapshot: EnterpriseSnapshot): TrustSignal[] {
  const signals: TrustSignal[] = [];
  const unhealthyConnections = snapshot.connections.filter(
    (connection) =>
      !connection.enabled ||
      connection.status !== "ready" ||
      !["verified", "not_required"].includes(
        connection.authContext.verification,
      ),
  );
  if (unhealthyConnections.length) {
    signals.push({
      id: "connections",
      level: "warning",
      title: `${unhealthyConnections.length} 个 Connection 需要处理`,
      detail: "停用、降级或认证失效的 Connection 会在调用边界 fail closed。",
    });
  } else {
    signals.push({
      id: "connections",
      level: "healthy",
      title: "Connection 运行许可正常",
      detail: "已启用 Connection 均处于 ready，认证状态可执行。",
    });
  }

  const failedRuns = snapshot.runs.filter((run) => run.status === "failed");
  if (failedRuns.length) {
    signals.push({
      id: "failed-runs",
      level: "warning",
      title: `${failedRuns.length} 个 Run 失败`,
      detail: "检查 Node Trace 和 Activity Receipt 后再决定是否恢复。",
    });
  }

  if (snapshot.tasks.length) {
    signals.push({
      id: "human-tasks",
      level: "attention",
      title: `${snapshot.tasks.length} 个 HumanTask 等待人工处理`,
      detail: "审批、输入、重连、核对和输出审查统一由 Inbox 处理。",
    });
  } else {
    signals.push({
      id: "human-tasks",
      level: "healthy",
      title: "Inbox 已清空",
      detail: "当前没有待处理的人工控制点。",
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
      detail: "只有发布且固定 content hash 的版本能进入 DeploymentSnapshot。",
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
  templates: AgentTemplateVersionView[];
  requireApproval: boolean;
}): FlowSpec {
  const objectSchema = { type: "object" };
  const approvalId = "review";
  const outputId = "output";
  const agentIds = input.templates.map((_, index) => `agent-${index + 1}`);
  const nodes: FlowSpec["graph"]["nodes"] = input.templates.map(
    (template, index) => ({
      id: agentIds[index]!,
      label: template.template.name,
      kind: "agent",
      config: {
        reference: template.template.templateId,
        templateVersion: template.template.version,
      },
      inputSchema: objectSchema,
      outputSchema: objectSchema,
    }),
  );
  if (input.requireApproval) {
    nodes.push({
      id: approvalId,
      label: "Human review / 人工审查",
      kind: "approval",
      config: {},
      inputSchema: objectSchema,
      outputSchema: objectSchema,
    });
  }
  nodes.push({
    id: outputId,
    label: "Inbox output / 收件箱输出",
    kind: "output",
    config: {},
    inputSchema: objectSchema,
    outputSchema: objectSchema,
  });
  const path = input.requireApproval
    ? [...agentIds, approvalId, outputId]
    : [...agentIds, outputId];
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
      entryNodeId: agentIds[0] ?? outputId,
      nodes,
      edges: path.slice(0, -1).map((from, index) => ({
        from,
        to: path[index + 1]!,
        condition: null,
        allowedFields: [],
        dataClassification: "internal",
        onError: null,
        loopPolicy: null,
      })),
    },
    requestedCapabilities: {
      allowAllTools: false,
      tools: [],
      allowAllSkills: false,
      skills: [],
      allowAllPlugins: false,
      plugins: [],
      allowAllMcpServers: false,
      mcpServers: [],
      allowAllWorkspaceRoots: false,
      workspaceRoots: [],
    },
    budget: {
      maxNodeExecutions: 24,
      maxToolCalls: 40,
      maxDurationSeconds: 1800,
      maxLoopIterations: 4,
    },
    riskClass: "medium",
    pendingDecisions: [],
  };
}
