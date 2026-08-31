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

export function OverviewPage({
  client,
  onNavigate,
}: {
  client: ApiClient;
  onNavigate(view: Exclude<FlowPrimaryView, "conversation">): void;
}) {
  const { snapshot, store } = useEnterpriseStore(client);
  const workspace = useFlowWorkspaceSelection();
  useFlowWorkspaceTitle("Operations overview / 运行总览");

  const pendingEvents = snapshot.cases.filter(
    (item) => item.status === "accepted" && !item.flowRunId,
  );
  const unhealthyConnections = snapshot.connections.filter(
    (connection) => connectionProblems(connection).length > 0,
  );
  const recentRuns = snapshot.runs.slice(0, 6);
  const failedRuns = recentRuns.filter((run) => run.status === "failed");
  const humanAttentionCount = snapshot.tasks.length + pendingEvents.length;
  const attentionCount =
    humanAttentionCount + unhealthyConnections.length + failedRuns.length;
  const activeCount = activeRunCount(snapshot);
  const headline = snapshot.error
    ? "暂时无法读取运行状态"
    : attentionCount > 0
      ? `${attentionCount} 项需要处理`
      : activeCount > 0
        ? `${activeCount} 个流程正在运行`
        : "当前运行正常";
  const summary = snapshot.error
    ? "保留现有状态，刷新后可重新检查。"
    : attentionCount > 0
      ? "优先处理阻塞流程或外部连接的问题。"
      : "没有阻塞事项或连接异常。";

  const attentionItems = [
    ...unhealthyConnections.map((connection) => {
      const problem = connectionProblems(connection)[0];
      return {
        id: `connection:${connection.id}`,
        title: connection.name,
        detail: problem?.title ?? "连接需要处理",
        label: "连接",
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
      detail: task.dueAt ? `截止 ${formatTime(task.dueAt)}` : task.description,
      label: "人工处理",
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
        title: flow?.name ?? event.flowId,
        detail: flowCaseCoreLabel(event),
        label: "等待确认",
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
        detail: run.error || `失败于 ${formatTime(run.updatedAt)}`,
        label: "运行失败",
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
          <small>运行总览</small>
          <span className="enterprise-overview__title-row">
            <h2>{headline}</h2>
            <IconButton
              aria-label="刷新运行总览"
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
              ? ` 最近更新：${formatTime(snapshot.refreshedAt)}。`
              : ""}
          </p>
        </div>
      </section>

      {snapshot.error ? (
        <p className="enterprise-page__message is-error" role="alert">
          {snapshot.error}
        </p>
      ) : null}

      <nav className="enterprise-overview__signals" aria-label="关键运行状态">
        <SignalButton
          icon={<Inbox aria-hidden="true" size={16} />}
          label="待人工处理"
          onClick={() => onNavigate("inbox")}
          value={humanAttentionCount}
        />
        <SignalButton
          icon={<Activity aria-hidden="true" size={16} />}
          label="运行中"
          onClick={() => onNavigate("runs")}
          value={activeCount}
        />
        <SignalButton
          icon={<Cable aria-hidden="true" size={16} />}
          label="连接异常"
          onClick={() => onNavigate("connections")}
          value={unhealthyConnections.length}
        />
      </nav>

      <div className="enterprise-page__columns">
        <Panel title="需要处理">
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
              <li className="enterprise-list__empty">当前没有待处理事项。</li>
            ) : null}
          </ol>
        </Panel>

        <Panel title="最近运行">
          <ol className="enterprise-action-list">
            {recentRuns.map((run) => {
              const flow = snapshot.flows.find(
                (item) => item.flowId === run.flowId,
              );
              const status = runStatusPresentation(run.status);
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
                      <small>{formatTime(run.updatedAt)}</small>
                    </span>
                    <Badge variant={status.variant}>{status.label}</Badge>
                    <ArrowRight aria-hidden="true" size={14} />
                  </button>
                </li>
              );
            })}
            {recentRuns.length === 0 ? (
              <li className="enterprise-list__empty">尚无运行记录。</li>
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

function formatTime(value: string): string {
  return new Date(value).toLocaleString();
}
