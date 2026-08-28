import type { AgentInstance, AgentTemplateSpec } from "../../types";

export function AgentTextAreaField({
  label,
  value,
  onChange,
  mono = false,
}: {
  label: string;
  value: string;
  onChange(value: string): void;
  mono?: boolean;
}) {
  return (
    <label className="agent-template-panel__field">
      <span>{label}</span>
      <textarea
        className={mono ? "is-mono" : undefined}
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
    </label>
  );
}

export function shortAgentId(value: string): string {
  return value.slice(0, 8);
}

export function agentCapabilitySummary(values: string[]): string {
  return values.length ? values.join(", ") : "无";
}

export function agentRiskLabel(risk: AgentTemplateSpec["riskClass"]): string {
  return {
    low: "低风险",
    medium: "中风险",
    high: "高风险",
    critical: "关键风险",
  }[risk];
}

export function agentRiskBadge(
  risk: AgentTemplateSpec["riskClass"],
): "neutral" | "warning" | "danger" {
  if (risk === "critical") return "danger";
  if (risk === "high") return "warning";
  return "neutral";
}

export function agentChangeKindLabel(
  kind: "added" | "removed" | "expanded" | "reduced",
): string {
  return { added: "新增", removed: "移除", expanded: "扩展", reduced: "收窄" }[
    kind
  ];
}

export function agentInstanceStatusLabel(
  status: AgentInstance["status"],
): string {
  return {
    active: "运行中",
    suspended: "已暂停",
    completed: "已完成",
    revoked: "已撤销",
  }[status];
}
