import { Clock3, Play, RadioTower, RefreshCw } from "lucide-react";
import type { ApiClient } from "../../api/client";
import { HumanTaskInboxPanel } from "../HumanTaskInboxPanel";
import { Badge, Button, IconButton, Panel } from "../ui";
import { useWorkflowAutomationStore } from "./automation/store";

export function FlowInboxPage({ client }: { client: ApiClient }) {
  const { snapshot, store } = useWorkflowAutomationStore(client);
  const pending = snapshot.invocations.filter(
    (item) => item.status === "accepted" && !item.flowRunId,
  );

  return (
    <div className="enterprise-page enterprise-inbox-page">
      <Panel
        title={`Pending events / 待确认事件 · ${pending.length}`}
        actions={
          <IconButton
            aria-label="刷新待确认事件"
            disabled={Boolean(snapshot.busyAction)}
            onClick={() => void store.load(true)}
            size="compact"
          >
            <RefreshCw aria-hidden="true" size={14} />
          </IconButton>
        }
      >
        <p className="enterprise-page__lede">
          事件已经通过入口认证和幂等检查，但尚未创建 Flow
          Run。确认后将使用事件接收时固定的 Deployment 启动运行。
        </p>
        {snapshot.error || snapshot.notice ? (
          <p
            className={`enterprise-page__message${snapshot.error ? " is-error" : " is-success"}`}
            role={snapshot.error ? "alert" : "status"}
          >
            {snapshot.error ?? snapshot.notice}
          </p>
        ) : null}
        <ol className="enterprise-pending-events">
          {pending.map((invocation) => {
            const release = snapshot.releases.find(
              (item) => item.id === invocation.releaseId,
            );
            const busy = snapshot.busyAction === `start:${invocation.id}`;
            return (
              <li key={invocation.id}>
                <span className="enterprise-pending-events__icon">
                  <RadioTower aria-hidden="true" size={16} />
                </span>
                <span className="enterprise-pending-events__content">
                  <span>
                    <strong>{release?.releaseKey ?? "Workflow event"}</strong>
                    <Badge variant="warning">waiting review</Badge>
                  </span>
                  <small>
                    <Clock3 aria-hidden="true" size={13} />
                    {new Date(invocation.createdAt).toLocaleString()} ·{" "}
                    {invocation.idempotencyKey}
                  </small>
                  <details>
                    <summary>查看事件输入</summary>
                    <pre>{JSON.stringify(invocation.input, null, 2)}</pre>
                  </details>
                </span>
                <Button
                  disabled={Boolean(snapshot.busyAction)}
                  onClick={() => void store.startPending(invocation)}
                  size="compact"
                  variant="primary"
                >
                  <Play aria-hidden="true" size={14} />
                  {busy ? "启动中…" : "批准并运行"}
                </Button>
              </li>
            );
          })}
          {pending.length === 0 ? (
            <li className="enterprise-list__empty">当前没有待确认事件。</li>
          ) : null}
        </ol>
      </Panel>

      <HumanTaskInboxPanel client={client} />
    </div>
  );
}
