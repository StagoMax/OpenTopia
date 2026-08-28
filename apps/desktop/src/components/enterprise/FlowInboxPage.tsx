import { Clock3, Play, RadioTower, RefreshCw } from "lucide-react";
import { useState } from "react";
import type { ApiClient } from "../../api/client";
import { HumanTaskInboxPanel } from "../HumanTaskInboxPanel";
import { Badge, Button, IconButton, Panel } from "../ui";
import { useEnterpriseStore } from "./store";

export function FlowInboxPage({ client }: { client: ApiClient }) {
  const { snapshot, store } = useEnterpriseStore(client);
  const [busyCaseId, setBusyCaseId] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const pending = snapshot.cases.filter(
    (item) => item.status === "accepted" && !item.flowRunId,
  );

  return (
    <div className="enterprise-page enterprise-inbox-page">
      <Panel
        title={`Pending events / 待确认事件 · ${pending.length}`}
        actions={
          <IconButton
            aria-label="刷新待确认事件"
            disabled={Boolean(busyCaseId)}
            onClick={() => void store.load(true)}
            size="compact"
          >
            <RefreshCw aria-hidden="true" size={14} />
          </IconButton>
        }
      >
        <p className="enterprise-page__lede">
          事件已经通过入口认证和幂等检查，但尚未创建 Flow
          Run。确认后将使用事件接收时冻结的 Flow Revision 启动运行。
        </p>
        {snapshot.error || actionError ? (
          <p
            className="enterprise-page__message is-error"
            role="alert"
          >
            {snapshot.error ?? actionError}
          </p>
        ) : null}
        <ol className="enterprise-pending-events">
          {pending.map((flowCase) => {
            const flow = snapshot.flows.find(
              (item) => item.flowId === flowCase.flowId,
            );
            const busy = busyCaseId === flowCase.id;
            return (
              <li key={flowCase.id}>
                <span className="enterprise-pending-events__icon">
                  <RadioTower aria-hidden="true" size={16} />
                </span>
                <span className="enterprise-pending-events__content">
                  <span>
                    <strong>{flow?.name ?? flowCase.flowId}</strong>
                    <Badge variant="warning">waiting review</Badge>
                  </span>
                  <small>
                    <Clock3 aria-hidden="true" size={13} />
                    {new Date(flowCase.createdAt).toLocaleString()} ·{" "}
                    {flowCase.idempotencyKey}
                  </small>
                  <details>
                    <summary>查看事件输入</summary>
                    <pre>{JSON.stringify(flowCase.input, null, 2)}</pre>
                  </details>
                </span>
                <Button
                  disabled={Boolean(busyCaseId)}
                  onClick={() => {
                    setBusyCaseId(flowCase.id);
                    setActionError(null);
                    void client
                      .startPendingFlowCase(flowCase.id)
                      .then(() => store.load(true))
                      .catch((error: unknown) =>
                        setActionError(
                          error instanceof Error ? error.message : String(error),
                        ),
                      )
                      .finally(() => setBusyCaseId(null));
                  }}
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
