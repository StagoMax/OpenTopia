import { Plus } from "lucide-react";
import { useEffect } from "react";
import type { ApiClient } from "../../api/client";
import type { FlowPrimaryView } from "../../workspaceNavigation";
import { IconButton, SidebarRow } from "../ui";
import { enterpriseSidebarStatus } from "./enterpriseSidebarStatus";
import { shortId, trustSignals } from "./model";
import {
  templateKeyForAgent,
  useFlowAgentSelection,
} from "./flowAgentSelection";
import { useEnterpriseStore } from "./store";

export function EnterpriseSidebarCollection({
  client,
  view,
}: {
  client: ApiClient;
  view: FlowPrimaryView;
}) {
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

  const rows =
    view === "agents"
      ? snapshot.templates.slice(0, 40).map(({ template }) => ({
          id: templateKeyForAgent(template.templateId, template.version),
          title: template.name,
          detail: `${template.templateId}@${template.version}`,
          status: template.status,
        }))
      : view === "inbox"
        ? [
            ...snapshot.tasks.map((task) => ({
              id: `task:${task.id}`,
              title: task.title,
              detail: task.taskType.replaceAll("_", " "),
              status: task.status,
            })),
            ...pendingCases.map((flowCase) => ({
              id: `case:${flowCase.id}`,
              title:
                snapshot.flows.find((flow) => flow.flowId === flowCase.flowId)
                  ?.name ?? flowCase.flowId,
              detail: "pending event",
              status: "pending",
            })),
          ].slice(0, 40)
        : view === "workflow-templates"
          ? snapshot.flows.slice(0, 40).map((flow) => ({
              id: flow.flowId,
              title: flow.name,
              detail: `${flow.flowId}@${flow.activeRevision.compiledWorkflow.flowVersion}`,
              status: flow.status,
            }))
          : view === "runs"
            ? snapshot.runs.slice(0, 40).map((run) => ({
                id: run.id,
                title: run.flowId,
                detail: shortId(run.id),
                status: run.status,
              }))
            : view === "trust"
              ? trustSignals(snapshot).map((signal) => ({
                  id: signal.id,
                  title: signal.title,
                  detail: signal.detail,
                  status: signal.level,
                }))
              : view === "knowledge"
                ? [
                    {
                      id: "knowledge",
                      title: "Knowledge catalog",
                      detail: "Libraries · RAG sources",
                      status: "ready",
                    },
                  ]
                : [
                    {
                      id: "overview",
                      title: "Operations overview",
                      detail: "Agents · Workflows · Runs",
                      status: "live",
                    },
                  ];
  return (
    <section
      className="enterprise-sidebar-collection"
      aria-label={`${view} collection`}
    >
      <header>
        <strong>{sidebarTitle(view)}</strong>
        {view === "agents" || view === "workflow-templates" ? (
          <IconButton
            aria-label={view === "agents" ? "新建 Agent" : "新建 Flow"}
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
            title={view === "agents" ? "新建 Agent" : "新建 Flow"}
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
                    : view === "overview";
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
                status={enterpriseSidebarStatus(view, row.status)}
                title={row.title}
              />
            </li>
          );
        })}
        {rows.length === 0 ? (
          <li className="enterprise-list__empty">
            {view === "agents" ? "尚未创建 Agent" : "暂无条目"}
          </li>
        ) : null}
      </ol>
    </section>
  );
}

function sidebarTitle(view: FlowPrimaryView): string {
  if (view === "workflow-templates") return "Flows";
  return view.charAt(0).toUpperCase() + view.slice(1);
}
