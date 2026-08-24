import { BookOpenCheck, LockKeyhole } from "lucide-react";
import { Switch, TextField } from "./ui";

type AgentTemplateKnowledgeBindingFieldProps = {
  disabled?: boolean;
  enabled: boolean;
  namespaces: string;
  onEnabledChange(enabled: boolean): void;
  onNamespacesChange(namespaces: string): void;
};

export function AgentTemplateKnowledgeBindingField({
  disabled = false,
  enabled,
  namespaces,
  onEnabledChange,
  onNamespacesChange,
}: AgentTemplateKnowledgeBindingFieldProps) {
  return (
    <section
      className="agent-template-panel__knowledge-binding"
      aria-labelledby="agent-template-knowledge-title"
    >
      <header>
        <BookOpenCheck size={16} aria-hidden="true" />
        <span>
          <strong id="agent-template-knowledge-title">SAG 知识绑定</strong>
          <small>随模板版本冻结，Agent 只能检索指定 namespace。</small>
        </span>
        <Switch
          checked={enabled}
          disabled={disabled}
          label="启用 SAG 知识绑定"
          onChange={onEnabledChange}
        />
      </header>
      {enabled ? (
        <>
          <TextField
            label="Namespaces"
            value={namespaces}
            disabled={disabled}
            placeholder="opentopia.audit.work-injury.v1"
            hint="多个 namespace 用逗号或换行分隔；运行时不允许 Agent 自行扩大范围。"
            onChange={(event) => onNamespacesChange(event.target.value)}
          />
          <p>
            <LockKeyhole size={14} aria-hidden="true" />
            发布后，namespace 变更会作为权限变更进入发布审核。
          </p>
        </>
      ) : null}
    </section>
  );
}
