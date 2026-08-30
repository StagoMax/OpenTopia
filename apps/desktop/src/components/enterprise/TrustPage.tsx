import {
  CheckCircle2,
  RefreshCw,
  ShieldAlert,
  ShieldCheck,
  TriangleAlert,
} from "lucide-react";
import { useEffect } from "react";
import type { ApiClient } from "../../api/client";
import { IconButton } from "../ui";
import {
  FlowInspectorPanel,
  FlowInspectorSection,
  type FlowInspectorStatusVariant,
} from "./FlowInspectorPanel";
import {
  FlowInspectorPortal,
  useFlowWorkspaceSelection,
  useFlowWorkspaceTitle,
} from "./flowAgentSelection";
import { trustSignals } from "./model";
import { useEnterpriseStore } from "./store";

export function TrustPage({ client }: { client: ApiClient }) {
  const { snapshot, store } = useEnterpriseStore(client);
  const workspace = useFlowWorkspaceSelection();
  const signals = trustSignals(snapshot);
  const signal =
    signals.find((item) => item.id === workspace?.selectedTrustSignalId) ??
    signals[0] ??
    null;

  useEffect(() => {
    if (signal && signal.id !== workspace?.selectedTrustSignalId) {
      workspace?.setSelectedTrustSignalId(signal.id);
    }
  }, [signal, workspace]);

  useFlowWorkspaceTitle(signal?.title ?? "Trust center / 信任中心");
  const SignalIcon =
    signal?.level === "healthy"
      ? CheckCircle2
      : signal?.level === "warning"
        ? ShieldAlert
        : TriangleAlert;

  return (
    <div className="enterprise-page enterprise-trust enterprise-core-detail">
      {signal ? (
        <section
          className={`enterprise-core-detail__summary is-${signal.level}`}
        >
          <span className="enterprise-core-detail__icon" aria-hidden="true">
            <SignalIcon size={22} />
          </span>
          <div>
            <small>Trust signal / 信任信号</small>
            <h2>{signal.title}</h2>
            <p>{signal.detail}</p>
          </div>
        </section>
      ) : null}

      <section className="enterprise-core-detail__payload">
        <header>
          <strong>Execution invariants / 执行不变量</strong>
        </header>
        <ul className="enterprise-invariants">
          <Invariant
            detail="每个 Workflow Agent 节点固定模板版本、content hash 与 Connection 操作。"
            title="Immutable identity / 不可变身份"
          />
          <Invariant
            detail="节点权限只能从 Agent 配置与 Flow Revision 逐层收窄，不能从 Thread MCP 状态扩权。"
            title="Least privilege / 最小权限"
          />
          <Invariant
            detail="审批、补输入、重连、效果核对和输出审查统一形成 HumanTask。"
            title="Durable control points / 持久化控制点"
          />
          <Invariant
            detail="认证过期、能力移除、描述变更或快照缺失都会在外部调用前拒绝。"
            title="Fail closed / 失败关闭"
          />
        </ul>
      </section>

      <FlowInspectorPortal>
        <FlowInspectorPanel
          actions={
            <IconButton
              aria-label="刷新信任状态"
              disabled={snapshot.status === "loading"}
              onClick={() => void store.load(true)}
              size="compact"
            >
              <RefreshCw aria-hidden="true" size={14} />
            </IconButton>
          }
          status={signal?.level ?? snapshot.status}
          statusVariant={trustVariant(signal?.level)}
          subtitle={signal?.id}
          title="Trust center"
        >
          {snapshot.error ? (
            <p className="enterprise-page__message is-error" role="alert">
              {snapshot.error}
            </p>
          ) : null}
          <FlowInspectorSection title="Current signal / 当前信号">
            <p>{signal?.detail ?? "当前没有信任信号。"}</p>
          </FlowInspectorSection>
          <FlowInspectorSection title="Snapshot / 状态快照">
            <dl className="flow-inspector-facts">
              <div>
                <dt>Connections</dt>
                <dd>{snapshot.connections.length}</dd>
              </div>
              <div>
                <dt>Runs</dt>
                <dd>{snapshot.runs.length}</dd>
              </div>
              <div>
                <dt>Human tasks</dt>
                <dd>{snapshot.tasks.length}</dd>
              </div>
              <div>
                <dt>Refreshed</dt>
                <dd>
                  {snapshot.refreshedAt
                    ? new Date(snapshot.refreshedAt).toLocaleString()
                    : "Not loaded"}
                </dd>
              </div>
            </dl>
          </FlowInspectorSection>
        </FlowInspectorPanel>
      </FlowInspectorPortal>
    </div>
  );
}

function Invariant({ detail, title }: { detail: string; title: string }) {
  return (
    <li>
      <ShieldCheck aria-hidden="true" size={16} />
      <span>
        <strong>{title}</strong>
        <small>{detail}</small>
      </span>
    </li>
  );
}

function trustVariant(level: string | undefined): FlowInspectorStatusVariant {
  if (level === "healthy") return "success";
  if (level === "warning") return "danger";
  return "warning";
}
