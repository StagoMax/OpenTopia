import { Plus } from "lucide-react";
import { useEffect } from "react";
import type { ApiClient } from "../../api/client";
import type { FlowPrimaryView } from "../../workspaceNavigation";
import { IconButton, SidebarRow } from "../ui";
import { enterpriseSidebarStatus } from "./enterpriseSidebarStatus";
import {
  compactSidebarTime,
  enterpriseSidebarTitle,
  flowCaseCoreLabel,
  humanizeIdentifier,
  workflowTriggerLabel,
} from "./enterpriseSidebarPresentation";
import { shortId, trustSignals } from "./model";
import {
  templateKeyForAgent,
  useFlowAgentSelection,
} from "./flowAgentSelection";
import { useEnterpriseStore } from "./store";
import { useApplicationLanguage } from "../../ApplicationLanguageProvider";
import {
  interfaceMessage,
  type ApplicationLanguage,
} from "../../applicationLanguage";

export function EnterpriseSidebarCollection({
  client,
  view,
}: {
  client: ApiClient;
  view: FlowPrimaryView;
}) {
  const { language, t } = useApplicationLanguage();
  const { snapshot, store } = useEnterpriseStore(client);
  const selection = useFlowAgentSelection();
  const agentDataRevision = selection?.agentDataRevision ?? 0;
  const selectedFlowId = snapshot.flows.some(
    (flow) => flow.flowId === selection?.selectedFlowId,
  )
    ? selection?.selectedFlowId
    : snapshot.flows[0]?.flowId;
  const pendingCases = snapshot.cases.filter(
    (item) => item.status === "accepted" && !item.flowRunId,
  );

  useEffect(() => {
    if (agentDataRevision > 0) void store.load(true);
  }, [agentDataRevision, store]);

  if (view === "overview") return null;

  const rows =
    view === "agents"
      ? snapshot.templates.slice(0, 40).map(({ template }) => ({
          id: templateKeyForAgent(template.templateId, template.version),
          title: enterpriseSidebarTitle({
            id: template.templateId,
            label: template.name,
            qualifier: `v${template.version} · ${template.owner}`,
          }),
          detail: `${template.templateId}@${template.version}`,
          status: template.status,
        }))
      : view === "inbox"
        ? [
            ...snapshot.tasks.map((task) => ({
              id: `task:${task.id}`,
              title: enterpriseSidebarTitle({
                id: task.id,
                label: task.title,
                qualifier: humanizeIdentifier(task.taskType),
              }),
              detail: shortId(task.id),
              status: task.status,
            })),
            ...pendingCases.map((flowCase) => ({
              id: `case:${flowCase.id}`,
              title: enterpriseSidebarTitle({
                id: flowCase.flowId,
                label:
                  snapshot.flows.find((flow) => flow.flowId === flowCase.flowId)
                    ?.name ?? flowCase.flowId,
                qualifier: flowCaseCoreLabel(flowCase),
              }),
              detail: shortId(flowCase.id),
              status: "pending",
            })),
          ].slice(0, 40)
        : view === "workflow-templates"
          ? snapshot.flows.slice(0, 40).map((flow) => ({
              id: flow.flowId,
              title: enterpriseSidebarTitle({
                id: flow.flowId,
                label: flow.name,
                qualifier: `v${flow.activeRevision.compiledWorkflow.flowVersion} · ${workflowTriggerLabel(flow.activeRevision.trigger, language)}`,
              }),
              detail: `${flow.flowId}@${flow.activeRevision.compiledWorkflow.flowVersion}`,
              status: flow.status,
            }))
          : view === "runs"
            ? snapshot.runs.slice(0, 40).map((run) => {
                const flow = snapshot.flows.find(
                  (item) => item.flowId === run.flowId,
                );
                return {
                  id: run.id,
                  title: enterpriseSidebarTitle({
                    id: run.flowId,
                    label: flow?.name ?? run.flowId,
                    qualifier: compactSidebarTime(run.updatedAt),
                  }),
                  detail: `${shortId(run.id)} · v${run.flowVersion}`,
                  status: run.status,
                };
              })
            : view === "trust"
              ? trustSignals(snapshot, language).map((signal) => ({
                  id: signal.id,
                  title: signal.title,
                  detail: signal.detail,
                  status: signal.level,
                }))
              : view === "knowledge"
                ? [
                    {
                      id: "knowledge",
                      title: t("flow.sidebar.knowledgeCatalog"),
                      detail: t("flow.sidebar.knowledgeDetail"),
                      status: "ready",
                    },
                  ]
                : [];
  return (
    <section
      className="enterprise-sidebar-collection"
      aria-label={`${sidebarTitle(view, language)} ${t("flow.sidebar.collection")}`}
    >
      <header>
        <strong>{sidebarTitle(view, language)}</strong>
        {view === "agents" || view === "workflow-templates" ? (
          <IconButton
            aria-label={
              view === "agents"
                ? t("flow.sidebar.newAgent")
                : t("flow.sidebar.newFlow")
            }
            className="enterprise-sidebar-collection__create"
            onClick={() => {
              if (view === "agents") {
                selection?.requestCreateAgent();
              } else {
                selection?.setSelectedFlowId(null);
                selection?.requestCreateFlow();
              }
            }}
            size="compact"
            title={
              view === "agents"
                ? t("flow.sidebar.newAgent")
                : t("flow.sidebar.newFlow")
            }
          >
            <Plus aria-hidden="true" size={14} />
          </IconButton>
        ) : (
          <small>{rows.length}</small>
        )}
      </header>
      <ol>
        {rows.map((row) => {
          const isAgentRow = view === "agents";
          const isFlowRow = view === "workflow-templates";
          const isInboxRow = view === "inbox";
          const isRunRow = view === "runs";
          const isTrustRow = view === "trust";
          const isSelected = isAgentRow
            ? !selection?.creatingAgent &&
              row.id === selection?.selectedTemplateKey
            : isFlowRow
              ? !selection?.creatingFlow && row.id === selectedFlowId
              : isInboxRow
                ? row.id === (selection?.selectedInboxItemId ?? rows[0]?.id)
                : isRunRow
                  ? row.id === (selection?.selectedRunId ?? rows[0]?.id)
                  : isTrustRow
                    ? row.id ===
                      (selection?.selectedTrustSignalId ?? rows[0]?.id)
                    : false;
          return (
            <li key={row.id}>
              <SidebarRow
                active={isSelected}
                description={row.detail}
                onSelect={
                  isAgentRow
                    ? () => selection?.requestViewAgent(row.id)
                    : isFlowRow
                      ? () => selection?.setSelectedFlowId(row.id)
                      : isInboxRow
                        ? () => selection?.setSelectedInboxItemId(row.id)
                        : isRunRow
                          ? () => selection?.setSelectedRunId(row.id)
                          : isTrustRow
                            ? () => selection?.setSelectedTrustSignalId(row.id)
                            : undefined
                }
                status={enterpriseSidebarStatus(view, row.status, language)}
                title={row.title}
              />
            </li>
          );
        })}
        {rows.length === 0 ? (
          <li className="enterprise-list__empty">
            {view === "agents"
              ? t("flow.sidebar.noAgents")
              : t("flow.sidebar.noItems")}
          </li>
        ) : null}
      </ol>
    </section>
  );
}

function sidebarTitle(
  view: FlowPrimaryView,
  language: ApplicationLanguage,
): string {
  if (view === "overview")
    return interfaceMessage(language, "flow.nav.overview");
  if (view === "inbox") return interfaceMessage(language, "flow.nav.inbox");
  if (view === "agents") return interfaceMessage(language, "flow.nav.agents");
  if (view === "workflow-templates")
    return interfaceMessage(language, "flow.nav.flows");
  if (view === "runs") return interfaceMessage(language, "flow.nav.runs");
  if (view === "connections")
    return interfaceMessage(language, "flow.nav.connections");
  if (view === "trust") return interfaceMessage(language, "flow.nav.trust");
  if (view === "knowledge")
    return interfaceMessage(language, "flow.nav.knowledge");
  return "Flow";
}
