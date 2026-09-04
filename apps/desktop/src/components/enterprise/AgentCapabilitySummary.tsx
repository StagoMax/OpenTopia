import { Sparkles, Wrench } from "lucide-react";
import type { AgentTemplateVersionView } from "../../types";
import { useApplicationLanguage } from "../../ApplicationLanguageProvider";
import { Badge } from "../ui";

export function AgentCapabilitySummary({
  template,
}: {
  template: AgentTemplateVersionView | undefined;
}) {
  const { t } = useApplicationLanguage();
  const capabilities = template?.template.spec?.capabilities;
  if (!capabilities) {
    return (
      <section className="flow-editor-inspector__section">
        <header>
          <strong>{t("flow.agentCapabilities.title")}</strong>
        </header>
        <p className="flow-editor-inspector__note">
          {t("flow.agentCapabilities.selectHint")}
        </p>
      </section>
    );
  }

  return (
    <section className="flow-editor-inspector__section">
      <header>
        <span>
          <strong>{t("flow.agentCapabilities.title")}</strong>
          <small>{t("flow.agentCapabilities.readOnlyHint")}</small>
        </span>
        <Badge variant="neutral">{t("flow.agentCapabilities.template")}</Badge>
      </header>
      <dl className="flow-agent-capabilities">
        <CapabilityRow
          all={capabilities.allowAllSkills}
          allLabel={t("flow.agentCapabilities.all")}
          emptyLabel={t("flow.agentCapabilities.noSkills")}
          icon={Sparkles}
          label={t("flow.agentCapabilities.skills")}
          values={capabilities.skills}
        />
        <CapabilityRow
          all={capabilities.allowAllTools}
          allLabel={t("flow.agentCapabilities.all")}
          emptyLabel={t("flow.agentCapabilities.noTools")}
          icon={Wrench}
          label={t("flow.agentCapabilities.tools")}
          values={capabilities.tools}
        />
      </dl>
      <p className="flow-editor-inspector__note">
        {t("flow.agentCapabilities.updateHint")}
      </p>
    </section>
  );
}

function CapabilityRow({
  all,
  allLabel,
  emptyLabel,
  icon: Icon,
  label,
  values,
}: {
  all: boolean;
  allLabel: string;
  emptyLabel: string;
  icon: typeof Sparkles;
  label: string;
  values: string[];
}) {
  return (
    <div>
      <dt>
        <Icon aria-hidden="true" size={14} />
        {label}
      </dt>
      <dd>
        {all ? allLabel : values.length > 0 ? values.join(", ") : emptyLabel}
      </dd>
    </div>
  );
}
