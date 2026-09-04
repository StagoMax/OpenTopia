import { Bot, Cable, Database, ShieldCheck } from "lucide-react";
import { agentKnowledgeBindingSummary } from "../../agentKnowledgeBinding";
import type { AgentTemplateVersionView } from "../../types";
import { Badge, SelectField } from "../ui";
import { templateKey } from "./flowActivation";
import type { WorkflowAgentSelection } from "./workflowNodeSelection";
import { useApplicationLanguage } from "../../ApplicationLanguageProvider";

export function FlowAgentReferencePage({
  node,
  onChange,
  templates,
}: {
  node: WorkflowAgentSelection;
  onChange(templateKey: string): void;
  templates: AgentTemplateVersionView[];
}) {
  const { language, t } = useApplicationLanguage();
  const selected = templates.find(
    (item) => templateKey(item) === node.templateKey,
  );
  return (
    <div className="flow-agent-reference-page">
      <header>
        <span className="flow-trigger-page__icon">
          <Bot aria-hidden="true" size={18} />
        </span>
        <span>
          <strong>{t("flow.agentReference.title")}</strong>
          <small>{t("flow.agentReference.description")}</small>
        </span>
      </header>
      <SelectField
        label={t("flow.agentReference.agent")}
        onChange={onChange}
        options={templates.map((item) => ({
          value: templateKey(item),
          label: `${item.template.name} · ${item.template.templateId}@${item.template.version}`,
        }))}
        value={node.templateKey}
      />
      {selected ? (
        <div className="flow-agent-reference-page__details">
          <section>
            <header>
              <strong>{selected.template.name}</strong>
              <Badge
                variant={
                  selected.template.status === "published"
                    ? "success"
                    : "warning"
                }
              >
                {selected.template.status === "published"
                  ? t("flow.agents.published")
                  : t("flow.agents.draft")}
              </Badge>
            </header>
            <p>
              {selected.template.spec.description ||
                t("flow.agentReference.noDescription")}
            </p>
            <pre>{selected.template.spec.instructions}</pre>
          </section>
          <dl>
            <div>
              <dt>
                <Cable aria-hidden="true" size={14} />{" "}
                {t("flow.agentReference.connections")}
              </dt>
              <dd>
                {selected.template.spec.connectionBindings?.length ?? 0}{" "}
                {t("flow.agentReference.bindings")}
              </dd>
            </div>
            <div>
              <dt>
                <Database aria-hidden="true" size={14} />{" "}
                {t("flow.agentReference.knowledge")}
              </dt>
              <dd>
                {agentKnowledgeBindingSummary(
                  selected.template.spec.knowledgeBinding,
                  language,
                )}
              </dd>
            </div>
            <div>
              <dt>
                <ShieldCheck aria-hidden="true" size={14} />{" "}
                {t("flow.agentReference.permissions")}
              </dt>
              <dd>{selected.template.spec.riskClass}</dd>
            </div>
          </dl>
        </div>
      ) : null}
    </div>
  );
}
