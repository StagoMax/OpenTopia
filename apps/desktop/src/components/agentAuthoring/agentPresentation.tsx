import type { AgentInstance, AgentTemplateSpec } from "../../types";
import {
  defaultApplicationLanguage,
  interfaceMessage,
  type ApplicationLanguage,
} from "../../applicationLanguage.ts";

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

export function agentCapabilitySummary(
  values: string[],
  language: ApplicationLanguage = defaultApplicationLanguage,
): string {
  return values.length
    ? values.join(", ")
    : interfaceMessage(language, "flow.agentEditor.noneValue");
}

export function agentRiskLabel(
  risk: AgentTemplateSpec["riskClass"],
  language: ApplicationLanguage = defaultApplicationLanguage,
): string {
  return {
    low: interfaceMessage(language, "flow.agentEditor.riskLabel.low"),
    medium: interfaceMessage(language, "flow.agentEditor.riskLabel.medium"),
    high: interfaceMessage(language, "flow.agentEditor.riskLabel.high"),
    critical: interfaceMessage(language, "flow.agentEditor.riskLabel.critical"),
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
  language: ApplicationLanguage = defaultApplicationLanguage,
): string {
  return {
    added: interfaceMessage(language, "flow.agentEditor.change.added"),
    removed: interfaceMessage(language, "flow.agentEditor.change.removed"),
    expanded: interfaceMessage(language, "flow.agentEditor.change.expanded"),
    reduced: interfaceMessage(language, "flow.agentEditor.change.reduced"),
  }[kind];
}

export function agentInstanceStatusLabel(
  status: AgentInstance["status"],
  language: ApplicationLanguage = defaultApplicationLanguage,
): string {
  return {
    active: interfaceMessage(language, "flow.agentEditor.instance.active"),
    suspended: interfaceMessage(
      language,
      "flow.agentEditor.instance.suspended",
    ),
    completed: interfaceMessage(
      language,
      "flow.agentEditor.instance.completed",
    ),
    revoked: interfaceMessage(language, "flow.agentEditor.instance.revoked"),
  }[status];
}
