import {
  Bot,
  Cable,
  Database,
  FileJson2,
  ShieldCheck,
  Wrench,
} from "lucide-react";
import { Badge } from "./ui";
import { useApplicationLanguage } from "../ApplicationLanguageProvider";
import { agentRiskLabel } from "./agentAuthoring/agentPresentation";
import "../styles/agent-studio.css";

export type AgentConfigPreview = {
  name: string;
  instructions: string;
  connectionCount: number;
  knowledge: string;
  riskClass: "low" | "medium" | "high" | "critical";
  tools: string[];
  outputSchema: string;
};

export function AgentConfigInspector({
  generating,
  preview,
}: {
  generating: boolean;
  preview: AgentConfigPreview;
}) {
  const { language, t } = useApplicationLanguage();
  return (
    <aside
      className="agent-config-inspector"
      aria-label={t("flow.agentInspector.aria")}
    >
      <header>
        <span>
          <Bot aria-hidden="true" size={16} />
          <strong>{t("flow.agentInspector.title")}</strong>
        </span>
        <Badge variant={generating ? "warning" : "neutral"}>
          {generating
            ? t("flow.agentInspector.modelGenerating")
            : t("flow.agents.draft")}
        </Badge>
      </header>
      <section>
        <strong>{preview.name || t("flow.agentInspector.untitled")}</strong>
        <p>{preview.instructions || t("flow.agentInspector.emptyHint")}</p>
      </section>
      <dl>
        <InspectorRow
          icon={Cable}
          label={t("flow.agentEditor.connections")}
          value={`${preview.connectionCount} ${t("flow.agentInspector.bindings")}`}
        />
        <InspectorRow
          icon={Database}
          label={t("flow.agentEditor.knowledge")}
          value={preview.knowledge}
        />
        <InspectorRow
          icon={Wrench}
          label={t("flow.agentEditor.tools")}
          value={
            preview.tools.join(", ") || t("flow.agentInspector.notAuthorized")
          }
        />
        <InspectorRow
          icon={ShieldCheck}
          label={t("flow.agentInspector.permissions")}
          value={agentRiskLabel(preview.riskClass, language)}
        />
        <InspectorRow
          icon={FileJson2}
          label={t("flow.agentInspector.finalSchema")}
          value={schemaLabel(preview.outputSchema, t)}
        />
      </dl>
      <footer>{t("flow.agentInspector.boundary")}</footer>
    </aside>
  );
}

function InspectorRow({
  icon: Icon,
  label,
  value,
}: {
  icon: typeof Cable;
  label: string;
  value: string;
}) {
  return (
    <div>
      <dt>
        <Icon aria-hidden="true" size={14} /> {label}
      </dt>
      <dd>{value}</dd>
    </div>
  );
}

function schemaLabel(
  value: string,
  t: ReturnType<typeof useApplicationLanguage>["t"],
): string {
  try {
    const schema = JSON.parse(value) as { type?: string; properties?: object };
    const propertyCount = Object.keys(schema.properties ?? {}).length;
    return `${schema.type ?? "schema"}${propertyCount ? ` · ${propertyCount} ${t("flow.agentInspector.fields")}` : ""}`;
  } catch {
    return t("flow.agentInspector.schemaInvalid");
  }
}
