import {
  ArrowRight,
  CheckCircle2,
  CircleAlert,
  RefreshCw,
  ShieldAlert,
  TriangleAlert,
} from "lucide-react";
import { useEffect } from "react";
import type { ApiClient } from "../../api/client";
import type { FlowPrimaryView } from "../../workspaceNavigation";
import { IconButton } from "../ui";
import { connectionProblemAreaLabel } from "../connections/model";
import { getConnectionsStore } from "../connections/store";
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

export function TrustPage({
  client,
  onNavigate,
}: {
  client: ApiClient;
  onNavigate(view: Exclude<FlowPrimaryView, "conversation">): void;
}) {
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

  function openConnection(connectionId: string) {
    getConnectionsStore(client).reveal(connectionId);
    onNavigate("connections");
  }

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

      {signal?.findings.length ? (
        <section className="enterprise-core-detail__payload enterprise-trust-findings">
          <header>
            <span>
              <strong>Affected Connections / 受影响的连接</strong>
              <small>点击条目可直达对应 Connection 的问题详情。</small>
            </span>
          </header>
          <ol>
            {signal.findings.map((finding) => (
              <li key={finding.id}>
                <button
                  aria-label={`定位并处理 Connection：${finding.label}`}
                  onClick={() => openConnection(finding.target.connectionId)}
                  type="button"
                >
                  <CircleAlert aria-hidden="true" size={18} />
                  <span className="enterprise-trust-finding__content">
                    <span className="enterprise-trust-finding__identity">
                      <strong>{finding.label}</strong>
                      <small>{finding.context}</small>
                    </span>
                    <span className="enterprise-trust-finding__problems">
                      {finding.problems.map((problem) => (
                        <span key={problem.code}>
                          <small>
                            {connectionProblemAreaLabel(problem.area)}
                          </small>
                          <span>
                            <strong>{problem.title}</strong>
                            <small>{problem.detail}</small>
                          </span>
                        </span>
                      ))}
                    </span>
                  </span>
                  <span className="enterprise-trust-finding__action">
                    定位问题
                    <ArrowRight aria-hidden="true" size={14} />
                  </span>
                </button>
              </li>
            ))}
          </ol>
        </section>
      ) : null}

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
          status={trustStatusLabel(signal?.level, snapshot.status)}
          statusVariant={trustVariant(signal?.level)}
          title="信任状态"
        >
          {snapshot.error ? (
            <p className="enterprise-page__message is-error" role="alert">
              {snapshot.error}
            </p>
          ) : null}
          <FlowInspectorSection title="Current signal / 当前信号">
            <p>{signal?.detail ?? "当前没有信任信号。"}</p>
          </FlowInspectorSection>
          <FlowInspectorSection title="更新时间">
            <dl className="flow-inspector-facts">
              <div>
                <dt>最近刷新</dt>
                <dd>
                  {snapshot.refreshedAt
                    ? new Date(snapshot.refreshedAt).toLocaleString()
                    : "尚未加载"}
                </dd>
              </div>
            </dl>
          </FlowInspectorSection>
        </FlowInspectorPanel>
      </FlowInspectorPortal>
    </div>
  );
}

function trustVariant(level: string | undefined): FlowInspectorStatusVariant {
  if (level === "healthy") return "success";
  if (level === "warning") return "danger";
  return "warning";
}

function trustStatusLabel(level: string | undefined, fallback: string): string {
  if (level === "healthy") return "正常";
  if (level === "warning") return "需要处理";
  if (level === "attention") return "需要关注";
  if (fallback === "loading") return "刷新中";
  return "待检查";
}
