import {
  Activity,
  ArrowRight,
  Cable,
  CheckCircle2,
  CircleAlert,
  Inbox,
  RefreshCw,
} from "lucide-react";
import type { ApiClient } from "../../api/client";
import type { FlowPrimaryView } from "../../workspaceNavigation";
import { connectionProblems } from "../connections/model";
import { getConnectionsStore } from "../connections/store";
import { Badge, IconButton, Panel } from "../ui";
import { flowCaseCoreLabel } from "./enterpriseSidebarPresentation";
import {
  useFlowWorkspaceSelection,
  useFlowWorkspaceTitle,
} from "./flowAgentSelection";
import { activeRunCount } from "./model";
import { runStatusPresentation } from "./runPresentation";
import { useEnterpriseStore } from "./store";
import { useApplicationLanguage } from "../../ApplicationLanguageProvider";

export function OverviewPage({
  client,
  onNavigate,
}: {
  client: ApiClient;
  onNavigate(view: Exclude<FlowPrimaryView, "conversation">): void;
}) {
  const { language, t } = useApplicationLanguage();
  const { snapshot, store } = useEnterpriseStore(client);
  const workspace = useFlowWorkspaceSelection();
  useFlowWorkspaceTitle(t("flow.overview.workspaceTitle"));

  const pendingEvents = snapshot.cases.filter(
    (item) => item.status === "accepted" && !item.flowRunId,
  );
  const unhealthyConnections = snapshot.connections.filter(
    (connection) => connectionProblems(connection, language).length > 0,
  );
  const recentRuns = snapshot.runs.slice(0, 6);
  const failedRuns = recentRuns.filter((run) => run.status === "failed");
  const humanAttentionCount = snapshot.tasks.length + pendingEvents.length;
  const attentionCount =
    humanAttentionCount + unhealthyConnections.length + failedRuns.length;
  const activeCount = activeRunCount(snapshot);
  const headline = snapshot.error
    ? t("flow.overview.unavailable")
    : attentionCount > 0
      ? `${attentionCount} ${t("flow.overview.attention")}`
      : activeCount > 0
        ? `${activeCount} ${t("flow.overview.active")}`
        : t("flow.overview.healthy");
  const summary = snapshot.error
    ? t("flow.overview.errorSummary")
    : attentionCount > 0
      ? t("flow.overview.attentionSummary")
      : t("flow.overview.healthySummary");

  const attentionItems = [
    ...unhealthyConnections.map((connection) => {
      const problem = connectionProblems(connection, language)[0];
      return {
        id: `connection:${connection.id}`,
        title: connection.name,
        detail: problem?.title ?? t("flow.overview.connectionNeedsAttention"),
        label: t("flow.overview.connection"),
        variant: "danger" as const,
        activate: () => {
          getConnectionsStore(client).reveal(connection.id);
          onNavigate("connections");
        },
      };
    }),
    ...snapshot.tasks.map((task) => ({
      id: `task:${task.id}`,
      title: task.title,
      detail: task.dueAt
        ? `${t("flow.overview.due")} ${formatTime(task.dueAt, language)}`
        : task.description,
      label: t("flow.overview.humanAction"),
      variant: "warning" as const,
      activate: () => {
        workspace?.setSelectedInboxItemId(`task:${task.id}`);
        onNavigate("inbox");
      },
    })),
    ...pendingEvents.map((event) => {
      const flow = snapshot.flows.find((item) => item.flowId === event.flowId);
      return {
        id: `case:${event.id}`,
        title: flowCaseCoreLabel(event),
        detail: flow?.name ?? event.flowId,
        label: t("flow.overview.awaitingConfirmation"),
        variant: "warning" as const,
        activate: () => {
          workspace?.setSelectedInboxItemId(`case:${event.id}`);
          onNavigate("inbox");
        },
      };
    }),
    ...failedRuns.map((run) => {
      const flow = snapshot.flows.find((item) => item.flowId === run.flowId);
      return {
        id: `run:${run.id}`,
        title: flow?.name ?? run.flowId,
        detail:
          run.error ||
          `${t("flow.overview.failedAt")} ${formatTime(run.updatedAt, language)}`,
        label: t("flow.overview.runFailed"),
        variant: "danger" as const,
        activate: () => {
          workspace?.setSelectedRunId(run.id);
          onNavigate("runs");
        },
      };
    }),
  ].slice(0, 6);

  return (
    <div className="enterprise-page enterprise-overview enterprise-core-detail">
      <section
        className={`enterprise-core-detail__summary ${snapshot.error ? "is-warning" : attentionCount > 0 ? "is-attention" : "is-healthy"}`}
      >
        <span className="enterprise-core-detail__icon" aria-hidden="true">
          {attentionCount > 0 || snapshot.error ? (
            <CircleAlert size={22} />
          ) : (
            <CheckCircle2 size={22} />
          )}
        </span>
        <div>
          <small>{t("flow.overview.workspaceTitle")}</small>
          <span className="enterprise-overview__title-row">
            <h2>{headline}</h2>
            <IconButton
              aria-label={t("flow.overview.refresh")}
              disabled={snapshot.status === "loading"}
              onClick={() => void store.load(true)}
              size="compact"
            >
              <RefreshCw aria-hidden="true" size={14} />
            </IconButton>
          </span>
          <p>
            {summary}
            {snapshot.refreshedAt
              ? ` ${t("flow.overview.lastUpdated")}：${formatTime(snapshot.refreshedAt, language)}。`
              : ""}
          </p>
        </div>
      </section>

      {snapshot.error ? (
        <p className="enterprise-page__message is-error" role="alert">
          {snapshot.error}
        </p>
      ) : null}

      <nav
        className="enterprise-overview__signals"
        aria-label={t("flow.overview.signals")}
      >
        <SignalButton
          icon={<Inbox aria-hidden="true" size={16} />}
          label={t("flow.overview.humanPending")}
          onClick={() => onNavigate("inbox")}
          value={humanAttentionCount}
        />
        <SignalButton
          icon={<Activity aria-hidden="true" size={16} />}
          label={t("flow.overview.running")}
          onClick={() => onNavigate("runs")}
          value={activeCount}
        />
        <SignalButton
          icon={<Cable aria-hidden="true" size={16} />}
          label={t("flow.overview.connectionIssues")}
          onClick={() => onNavigate("connections")}
          value={unhealthyConnections.length}
        />
      </nav>

      <div className="enterprise-page__columns">
        <Panel title={t("flow.overview.needsAttention")}>
          <ol className="enterprise-action-list">
            {attentionItems.map((item) => (
              <li key={item.id}>
                <button onClick={item.activate} type="button">
                  <span>
                    <strong>{item.title}</strong>
                    <small>{item.detail}</small>
                  </span>
                  <Badge variant={item.variant}>{item.label}</Badge>
                  <ArrowRight aria-hidden="true" size={14} />
                </button>
              </li>
            ))}
            {attentionItems.length === 0 ? (
              <li className="enterprise-list__empty">
                {t("flow.overview.noAttention")}
              </li>
            ) : null}
          </ol>
        </Panel>

        <Panel title={t("flow.overview.recentRuns")}>
          <ol className="enterprise-action-list">
            {recentRuns.map((run) => {
              const flow = snapshot.flows.find(
                (item) => item.flowId === run.flowId,
              );
              const status = runStatusPresentation(run.status, language);
              return (
                <li key={run.id}>
                  <button
                    onClick={() => {
                      workspace?.setSelectedRunId(run.id);
                      onNavigate("runs");
                    }}
                    type="button"
                  >
                    <span>
                      <strong>{flow?.name ?? run.flowId}</strong>
                      <small>{formatTime(run.updatedAt, language)}</small>
                    </span>
                    <Badge variant={status.variant}>{status.label}</Badge>
                    <ArrowRight aria-hidden="true" size={14} />
                  </button>
                </li>
              );
            })}
            {recentRuns.length === 0 ? (
              <li className="enterprise-list__empty">
                {t("flow.overview.noRuns")}
              </li>
            ) : null}
          </ol>
        </Panel>
      </div>
    </div>
  );
}

function SignalButton({
  icon,
  label,
  onClick,
  value,
}: {
  icon: React.ReactNode;
  label: string;
  onClick(): void;
  value: number;
}) {
  return (
    <button onClick={onClick} type="button">
      {icon}
      <span>
        <strong>{value}</strong>
        <small>{label}</small>
      </span>
    </button>
  );
}

function formatTime(
  value: string,
  language: import("../../applicationLanguage").ApplicationLanguage,
): string {
  return new Date(value).toLocaleString(language);
}
