import { BookOpenCheck, LockKeyhole } from "lucide-react";
import {
  AGENT_KNOWLEDGE_PROVIDER_OPTIONS,
  type AgentKnowledgeProviderSelection,
} from "../agentKnowledgeBinding";
import { SelectField, TextField } from "./ui";

type AgentTemplateKnowledgeBindingFieldProps = {
  disabled?: boolean;
  provider: AgentKnowledgeProviderSelection;
  namespaces: string;
  onProviderChange(provider: AgentKnowledgeProviderSelection): void;
  onNamespacesChange(namespaces: string): void;
};

export function AgentTemplateKnowledgeBindingField({
  disabled = false,
  provider,
  namespaces,
  onProviderChange,
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
          <strong id="agent-template-knowledge-title">Agent 知识库</strong>
          <small>知识库随 Agent 版本冻结，Flow 只继承这个选择。</small>
        </span>
      </header>
      <SelectField<AgentKnowledgeProviderSelection>
        disabled={disabled}
        hint="选择后会自动授予 library_search 权限；不需要在 Flow Revision 再配置。"
        label="知识库"
        onChange={onProviderChange}
        options={AGENT_KNOWLEDGE_PROVIDER_OPTIONS}
        value={provider}
      />
      {provider === "sag" ? (
        <TextField
          label="Namespaces"
          value={namespaces}
          disabled={disabled}
          placeholder="opentopia.audit.work-injury.v1"
          hint="多个 namespace 用逗号或换行分隔；运行时不允许 Agent 自行扩大范围。"
          onChange={(event) => onNamespacesChange(event.target.value)}
        />
      ) : null}
      {provider ? (
        <p>
          <LockKeyhole size={14} aria-hidden="true" />
          发布后，知识库后端
          {provider === "sag" ? " 与 namespace" : ""}
          的变更会作为 Agent 权限变更进入审核。
        </p>
      ) : null}
    </section>
  );
}
