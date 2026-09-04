import { Bot, Cable, FileText, Workflow } from "lucide-react";
import { useEffect } from "react";
import type { ApiClient } from "../../api/client";
import type { AppSettings } from "../../types";
import { AgentTemplatePanel } from "../AgentTemplatePanel";
import { Badge } from "../ui";
import { useEnterpriseStore } from "./store";
import {
  templateKeyForAgent,
  FlowInspectorPortal,
  useFlowAgentSelection,
  useFlowWorkspaceTitle,
} from "./flowAgentSelection";
import { useApplicationLanguage } from "../../ApplicationLanguageProvider";

export function AgentsPage({
  client,
  settings,
  threadId,
  workspaceRoot,
}: {
  client: ApiClient;
  settings: AppSettings | null;
  threadId: string | null;
  workspaceRoot: string | null;
}) {
  const { t } = useApplicationLanguage();
  const { snapshot } = useEnterpriseStore(client);
  const selection = useFlowAgentSelection();
  const requestedTemplateKey = selection?.selectedTemplateKey ?? null;
  const selectedTemplate = selection?.creatingAgent
    ? null
    : requestedTemplateKey
      ? (snapshot.templates.find(
          (view) =>
            templateKeyForAgent(
              view.template.templateId,
              view.template.version,
            ) === requestedTemplateKey,
        ) ?? null)
      : (snapshot.templates[0] ?? null);
  const selectedTemplateKey = selectedTemplate
    ? templateKeyForAgent(
        selectedTemplate.template.templateId,
        selectedTemplate.template.version,
      )
    : null;

  useEffect(() => {
    if (
      selection &&
      !selection.creatingAgent &&
      !requestedTemplateKey &&
      selectedTemplateKey
    ) {
      selection.setSelectedTemplateKey(selectedTemplateKey);
    }
  }, [requestedTemplateKey, selectedTemplateKey, selection]);

  useFlowWorkspaceTitle(
    selection?.creatingAgent
      ? t("flow.agents.new")
      : selectedTemplate?.template.name,
  );
  const selectedAgent = selectedTemplate?.template;
  const usedByFlows = selectedAgent
    ? snapshot.flows.filter((flow) =>
        Object.values(flow.activeRevision.compiledWorkflow.agentSpecs).some(
          (agent) =>
            agent.templateId === selectedAgent.templateId &&
            agent.templateVersion === selectedAgent.version,
        ),
      )
    : [];
  const matchingInstances = selectedAgent
    ? snapshot.agents.filter(
        (agent) =>
          agent.templateId === selectedAgent.templateId &&
          agent.templateVersion === selectedAgent.version,
      )
    : [];
  const activeInstances = matchingInstances.filter(
    (agent) => agent.status === "active",
  ).length;
  const connectionCount =
    selectedAgent?.spec.connectionBindings?.length ??
    selectedAgent?.spec.capabilities.mcpServers.length ??
    0;

  return (
    <>
      <FlowInspectorPortal>
        <AgentTemplatePanel
          client={client}
          settings={settings}
          showTemplateCollection={false}
          threadId={threadId}
          variant="rail"
          workspaceRoot={workspaceRoot}
        />
      </FlowInspectorPortal>
      <div className="enterprise-page enterprise-agents-page">
        {selectedTemplate ? (
          <article
            aria-labelledby="enterprise-agent-title"
            className="enterprise-agent-overview"
          >
            <section className="enterprise-core-detail__summary">
              <span className="enterprise-core-detail__icon" aria-hidden="true">
                <Bot size={20} />
              </span>
              <div>
                <small>
                  {selectedTemplate.template.owner} · {t("flow.agents.version")}{" "}
                  {selectedTemplate.template.version}
                </small>
                <span className="enterprise-agent-overview__title-row">
                  <h2 id="enterprise-agent-title">
                    {selectedTemplate.template.name}
                  </h2>
                  <Badge
                    variant={
                      selectedTemplate.template.status === "published"
                        ? "success"
                        : "warning"
                    }
                  >
                    {selectedTemplate.template.status === "published"
                      ? t("flow.agents.published")
                      : t("flow.agents.draft")}
                  </Badge>
                </span>
                <p>
                  {selectedTemplate.template.spec.description ||
                    t("flow.agents.noDescription")}
                </p>
              </div>
            </section>

            <dl
              className="enterprise-agent-overview__facts"
              aria-label={t("flow.agents.usageSummary")}
            >
              <div>
                <Workflow aria-hidden="true" size={16} />
                <span>
                  <strong>{usedByFlows.length}</strong>
                  <small>{t("flow.agents.usedByFlows")}</small>
                </span>
              </div>
              <div>
                <Bot aria-hidden="true" size={16} />
                <span>
                  <strong>{activeInstances}</strong>
                  <small>{t("flow.agents.activeInstances")}</small>
                </span>
              </div>
              <div>
                <Cable aria-hidden="true" size={16} />
                <span>
                  <strong>{connectionCount}</strong>
                  <small>{t("flow.agents.connections")}</small>
                </span>
              </div>
            </dl>

            {usedByFlows.length > 0 ? (
              <section className="enterprise-core-detail__payload enterprise-agent-overview__usage">
                <header>
                  <h3>{t("flow.agents.usedIn")}</h3>
                </header>
                <ul>
                  {usedByFlows.slice(0, 5).map((flow) => (
                    <li key={flow.flowId}>{flow.name}</li>
                  ))}
                </ul>
              </section>
            ) : null}

            <details
              className="enterprise-agent-instructions"
              key={selectedTemplateKey ?? undefined}
              open
            >
              <summary>
                <FileText aria-hidden="true" size={14} />
                {t("flow.agents.showInstructions")}
              </summary>
              <pre>
                {selectedTemplate.template.spec.instructions ||
                  t("flow.agents.noInstructions")}
              </pre>
            </details>
          </article>
        ) : (
          <div className="enterprise-agent-prompt-empty" role="status">
            <span>
              <FileText aria-hidden="true" size={20} />
            </span>
            <strong>
              {selection?.creatingAgent
                ? t("flow.agents.configure")
                : snapshot.status === "loading"
                  ? t("flow.agents.loading")
                  : requestedTemplateKey
                    ? t("flow.agents.syncing")
                    : t("flow.agents.none")}
            </strong>
            <p>
              {selection?.creatingAgent
                ? t("flow.agents.configureHint")
                : snapshot.status === "error"
                  ? snapshot.error || t("flow.agents.loadFailed")
                  : t("flow.agents.selectHint")}
            </p>
          </div>
        )}
      </div>
    </>
  );
}
