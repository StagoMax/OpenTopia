import {
  Activity,
  Bot,
  Inbox,
  LayoutDashboard,
  Library,
  RadioTower,
  ShieldCheck,
  Workflow,
} from "lucide-react";
import type { ApiClient } from "../../api/client";
import type { FlowPrimaryView } from "../../workspaceNavigation";
import { Badge } from "../ui";
import { shortId, trustSignals } from "./model";
import { useEnterpriseStore } from "./store";

export function EnterpriseSidebarCollection({
  client,
  view,
}: {
  client: ApiClient;
  view: FlowPrimaryView;
}) {
  const { snapshot } = useEnterpriseStore(client);
  const rows =
    view === "agents"
      ? snapshot.agents
          .slice(0, 40)
          .map((agent) => ({
            id: agent.id,
            title: `${agent.templateId}@${agent.templateVersion}`,
            detail: shortId(agent.id),
            status: agent.status,
            icon: Bot,
          }))
      : view === "inbox"
        ? snapshot.tasks
            .slice(0, 40)
            .map((task) => ({
              id: task.id,
              title: task.title,
              detail: task.taskType.replaceAll("_", " "),
              status: task.status,
              icon: Inbox,
            }))
        : view === "workflow-templates"
          ? snapshot.workflows
              .slice(0, 40)
              .map((workflow) => ({
                id: workflow.id,
                title: workflow.name,
                detail: `${workflow.flowId}@${workflow.version}`,
                status: "published",
                icon: Workflow,
              }))
          : view === "automation"
            ? snapshot.deployments
                .slice(0, 40)
                .map((deployment) => ({
                  id: deployment.id,
                  title: deployment.name,
                  detail: `${deployment.environment} · release candidate`,
                  status: deployment.status,
                  icon: RadioTower,
                }))
            : view === "runs"
              ? snapshot.runs
                  .slice(0, 40)
                  .map((run) => ({
                    id: run.id,
                    title: run.flowId,
                    detail: shortId(run.id),
                    status: run.status,
                    icon: Activity,
                  }))
              : view === "trust"
                ? trustSignals(snapshot).map((signal) => ({
                    id: signal.id,
                    title: signal.title,
                    detail: signal.detail,
                    status: signal.level,
                    icon: ShieldCheck,
                  }))
                : view === "knowledge"
                  ? [
                      {
                        id: "knowledge",
                        title: "Knowledge catalog",
                        detail: "Libraries · RAG sources",
                        status: "ready",
                        icon: Library,
                      },
                    ]
                  : [
                      {
                        id: "overview",
                        title: "Operations overview",
                        detail: "Agents · Workflows · Runs",
                        status: "live",
                        icon: LayoutDashboard,
                      },
                    ];
  return (
    <section
      className="enterprise-sidebar-collection"
      aria-label={`${view} collection`}
    >
      <header>
        <strong>{sidebarTitle(view)}</strong>
        <small>{rows.length}</small>
      </header>
      <ol>
        {rows.map((row) => {
          const Icon = row.icon;
          return (
            <li key={row.id}>
              <Icon aria-hidden="true" size={14} />
              <span>
                <strong>{row.title}</strong>
                <small>{row.detail}</small>
              </span>
              <Badge variant="neutral">{row.status}</Badge>
            </li>
          );
        })}
        {rows.length === 0 ? (
          <li className="enterprise-list__empty">暂无条目</li>
        ) : null}
      </ol>
    </section>
  );
}

function sidebarTitle(view: FlowPrimaryView): string {
  if (view === "workflow-templates") return "Workflow Templates";
  return view.charAt(0).toUpperCase() + view.slice(1);
}
