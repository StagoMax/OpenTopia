import { BookOpenCheck, LockKeyhole } from "lucide-react";
import {
  agentKnowledgeProviderOptions,
  type AgentKnowledgeProviderSelection,
} from "../agentKnowledgeBinding";
import { useApplicationLanguage } from "../ApplicationLanguageProvider";
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
  const { language, t } = useApplicationLanguage();
  return (
    <section
      className="agent-template-panel__knowledge-binding"
      aria-labelledby="agent-template-knowledge-title"
    >
      <header>
        <BookOpenCheck size={16} aria-hidden="true" />
        <span>
          <strong id="agent-template-knowledge-title">
            {t("flow.agentKnowledge.title")}
          </strong>
          <small>{t("flow.agentKnowledge.detail")}</small>
        </span>
      </header>
      <SelectField<AgentKnowledgeProviderSelection>
        disabled={disabled}
        hint={t("flow.agentKnowledge.autoGrantHint")}
        label={t("flow.agentEditor.knowledge")}
        onChange={onProviderChange}
        options={agentKnowledgeProviderOptions(language)}
        value={provider}
      />
      {provider === "sag" ? (
        <TextField
          label={t("flow.agentKnowledge.namespaces")}
          value={namespaces}
          disabled={disabled}
          placeholder="opentopia.audit.work-injury.v1"
          hint={t("flow.agentKnowledge.namespacesHint")}
          onChange={(event) => onNamespacesChange(event.target.value)}
        />
      ) : null}
      {provider ? (
        <p>
          <LockKeyhole size={14} aria-hidden="true" />
          {t("flow.agentKnowledge.reviewPrefix")}
          {provider === "sag"
            ? ` ${t("flow.agentKnowledge.reviewNamespace")} `
            : " "}
          {t("flow.agentKnowledge.reviewSuffix")}
        </p>
      ) : null}
    </section>
  );
}
