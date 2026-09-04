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
import { useApplicationLanguage } from "../../ApplicationLanguageProvider";
import {
  interfaceMessage,
  type ApplicationLanguage,
} from "../../applicationLanguage";

export function TrustPage({
  client,
  onNavigate,
}: {
  client: ApiClient;
  onNavigate(view: Exclude<FlowPrimaryView, "conversation">): void;
}) {
  const { language, t } = useApplicationLanguage();
  const { snapshot, store } = useEnterpriseStore(client);
  const workspace = useFlowWorkspaceSelection();
  const signals = trustSignals(snapshot, language);
  const signal =
    signals.find((item) => item.id === workspace?.selectedTrustSignalId) ??
    signals[0] ??
    null;

  useEffect(() => {
    if (signal && signal.id !== workspace?.selectedTrustSignalId) {
      workspace?.setSelectedTrustSignalId(signal.id);
    }
  }, [signal, workspace]);

  useFlowWorkspaceTitle(signal?.title ?? t("flow.trust.workspaceTitle"));
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
            <small>{t("flow.trust.signal")}</small>
            <h2>{signal.title}</h2>
            <p>{signal.detail}</p>
          </div>
        </section>
      ) : null}

      {signal?.findings.length ? (
        <section className="enterprise-core-detail__payload enterprise-trust-findings">
          <header>
            <span>
              <h3>{t("flow.trust.affectedConnections")}</h3>
              <small>{t("flow.trust.connectionsHint")}</small>
            </span>
          </header>
          <ol>
            {signal.findings.map((finding) => (
              <li key={finding.id}>
                <button
                  aria-label={`${t("flow.trust.locateAria")}：${finding.label}`}
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
                            {connectionProblemAreaLabel(problem.area, language)}
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
                    {t("flow.trust.locate")}
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
              aria-label={t("flow.trust.refresh")}
              disabled={snapshot.status === "loading"}
              onClick={() => void store.load(true)}
              size="compact"
            >
              <RefreshCw aria-hidden="true" size={14} />
            </IconButton>
          }
          status={trustStatusLabel(signal?.level, snapshot.status, language)}
          statusVariant={trustVariant(signal?.level)}
          title={t("flow.trust.status")}
        >
          {snapshot.error ? (
            <p className="enterprise-page__message is-error" role="alert">
              {snapshot.error}
            </p>
          ) : null}
          <FlowInspectorSection title={t("flow.trust.currentSignal")}>
            <p>{signal?.detail ?? t("flow.trust.noSignal")}</p>
          </FlowInspectorSection>
          <FlowInspectorSection title={t("flow.trust.updateTime")}>
            <dl className="flow-inspector-facts">
              <div>
                <dt>{t("flow.trust.lastRefresh")}</dt>
                <dd>
                  {snapshot.refreshedAt
                    ? new Date(snapshot.refreshedAt).toLocaleString(language)
                    : t("flow.trust.notLoaded")}
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

function trustStatusLabel(
  level: string | undefined,
  fallback: string,
  language: ApplicationLanguage,
): string {
  if (level === "healthy")
    return interfaceMessage(language, "flow.trust.healthy");
  if (level === "warning")
    return interfaceMessage(language, "flow.trust.needsAction");
  if (level === "attention")
    return interfaceMessage(language, "flow.trust.attention");
  if (fallback === "loading")
    return interfaceMessage(language, "flow.trust.refreshing");
  return interfaceMessage(language, "flow.trust.pendingCheck");
}
