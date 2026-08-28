import {
  Activity,
  Bot,
  Cable,
  Inbox,
  RefreshCw,
  Workflow,
} from "lucide-react";
import type { FlowPrimaryView } from "../../workspaceNavigation";
import { Badge, Button, Panel } from "../ui";
import { activeRunCount, latestPublishedTemplateCount, shortId } from "./model";
import { useEnterpriseStore } from "./store";
import type { ApiClient } from "../../api/client";

export function OverviewPage({
  client,
  onNavigate,
}: {
  client: ApiClient;
  onNavigate(view: Exclude<FlowPrimaryView, "conversation">): void;
}) {
  const { snapshot, store } = useEnterpriseStore(client);
  const pendingEvents = snapshot.cases.filter(
    (item) => item.status === "accepted" && !item.flowRunId,
  );
  const metrics = [
    {
      label: "Published Agents",
      value: latestPublishedTemplateCount(snapshot),
      detail: `${snapshot.agents.length} 个实例`,
      icon: Bot,
      view: "agents" as const,
    },
    {
      label: "Flows",
      value: snapshot.flows.length,
      detail: `${snapshot.flows.filter((item) => item.status === "active").length} 个已激活`,
      icon: Workflow,
      view: "workflow-templates" as const,
    },
    {
      label: "Active Runs",
      value: activeRunCount(snapshot),
      detail: `${snapshot.runs.length} 个最近运行`,
      icon: Activity,
      view: "runs" as const,
    },
    {
      label: "Inbox",
      value: snapshot.tasks.length + pendingEvents.length,
      detail: `${pendingEvents.length} 个事件待确认`,
      icon: Inbox,
      view: "inbox" as const,
    },
    {
      label: "Connections",
      value: snapshot.connections.filter((item) => item.status === "ready")
        .length,
      detail: `${snapshot.connections.length} 个已配置`,
      icon: Cable,
      view: "connections" as const,
    },
  ];

  return (
    <div className="enterprise-page enterprise-overview">
      <Panel
        title="Operations overview / 运行总览"
        actions={
          <Button
            aria-label="刷新企业运行总览"
            disabled={snapshot.status === "loading"}
            onClick={() => void store.load(true)}
            size="compact"
            variant="quiet"
          >
            <RefreshCw aria-hidden="true" size={14} />
            刷新
          </Button>
        }
      >
        <p className="enterprise-page__lede">
          从 Flow、运行到人工控制点的统一视图。所有数字来自服务端持久化对象，不扫描会话消息。
        </p>
        {snapshot.error ? (
          <p className="enterprise-page__message is-error" role="alert">
            {snapshot.error}
          </p>
        ) : null}
        <div className="enterprise-metrics" aria-label="Flow 运行指标">
          {metrics.map((metric) => {
            const Icon = metric.icon;
            return (
              <button
                className="enterprise-metric"
                key={metric.label}
                onClick={() => onNavigate(metric.view)}
                type="button"
              >
                <span className="enterprise-metric__icon">
                  <Icon aria-hidden="true" size={17} />
                </span>
                <strong>{metric.value}</strong>
                <span>{metric.label}</span>
                <small>{metric.detail}</small>
              </button>
            );
          })}
        </div>
      </Panel>

      <div className="enterprise-page__columns">
        <Panel title="Recent runs / 最近运行">
          <ol className="enterprise-list">
            {snapshot.runs.slice(0, 8).map((run) => (
              <li key={run.id}>
                <button onClick={() => onNavigate("runs")} type="button">
                  <Activity aria-hidden="true" size={15} />
                  <span>
                    <strong>{run.flowId}</strong>
                    <small>
                      {shortId(run.id)} · {formatTime(run.updatedAt)}
                    </small>
                  </span>
                  <Badge variant={runVariant(run.status)}>{run.status}</Badge>
                </button>
              </li>
            ))}
            {snapshot.runs.length === 0 ? (
              <li className="enterprise-list__empty">尚无 Workflow Run。</li>
            ) : null}
          </ol>
        </Panel>
        <Panel title="Attention / 待处理">
          <ol className="enterprise-list">
            {snapshot.tasks.slice(0, 8).map((task) => (
              <li key={task.id}>
                <button onClick={() => onNavigate("inbox")} type="button">
                  <Inbox aria-hidden="true" size={15} />
                  <span>
                    <strong>{task.title}</strong>
                    <small>{task.taskType.replaceAll("_", " ")}</small>
                  </span>
                  <Badge variant="warning">pending</Badge>
                </button>
              </li>
            ))}
            {pendingEvents
              .slice(0, Math.max(0, 8 - snapshot.tasks.length))
              .map((event) => (
                <li key={event.id}>
                  <button onClick={() => onNavigate("inbox")} type="button">
                    <Inbox aria-hidden="true" size={15} />
                    <span>
                      <strong>Workflow event / 工作流事件</strong>
                      <small>{event.idempotencyKey}</small>
                    </span>
                    <Badge variant="warning">review</Badge>
                  </button>
                </li>
              ))}
            {snapshot.tasks.length === 0 && pendingEvents.length === 0 ? (
              <li className="enterprise-list__empty">当前没有待处理事项。</li>
            ) : null}
          </ol>
        </Panel>
      </div>
    </div>
  );
}

function runVariant(
  status: string,
): "success" | "danger" | "warning" | "info" | "neutral" {
  if (status === "succeeded") return "success";
  if (status === "failed" || status === "cancelled") return "danger";
  if (status.includes("waiting") || status === "paused") return "warning";
  if (status === "running" || status === "resuming") return "info";
  return "neutral";
}

function formatTime(value: string): string {
  return new Date(value).toLocaleString();
}
