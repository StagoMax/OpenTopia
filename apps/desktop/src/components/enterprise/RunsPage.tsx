import {
  Activity,
  PauseCircle,
  PlayCircle,
  RefreshCw,
  XCircle,
} from "lucide-react";
import { useEffect, useState } from "react";
import type { ApiClient } from "../../api/client";
import type { FlowRun } from "../../types";
import { Button, IconButton } from "../ui";
import {
  FlowInspectorPortal,
  useFlowWorkspaceSelection,
  useFlowWorkspaceTitle,
} from "./flowAgentSelection";
import { FlowInspectorPanel, FlowInspectorSection } from "./FlowInspectorPanel";
import { RunDetails } from "./RunDetails";
import {
  formatDateTime,
  formatDuration,
  runStatusPresentation,
} from "./runPresentation";
import { useEnterpriseStore } from "./store";
import "./runs-page.css";
import { useApplicationLanguage } from "../../ApplicationLanguageProvider";

export function RunsPage({ client }: { client: ApiClient }) {
  const { language, t } = useApplicationLanguage();
  const { snapshot, store } = useEnterpriseStore(client);
  const selection = useFlowWorkspaceSelection();
  const selected =
    snapshot.runs.find((run) => run.id === selection?.selectedRunId) ??
    snapshot.runs[0] ??
    null;
  const selectedFlow = selected
    ? snapshot.flows.find((flow) => flow.flowId === selected.flowId)
    : null;
  const flowName =
    selectedFlow?.name ?? selected?.flowId ?? t("flow.runs.fallbackName");
  const [busy, setBusy] = useState<"pause" | "resume" | "cancel" | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (selected && selected.id !== selection?.selectedRunId) {
      selection?.setSelectedRunId(selected.id);
    }
  }, [selected, selection]);

  useFlowWorkspaceTitle(
    selected
      ? `${flowName} · ${t("flow.nav.runs")}`
      : t("flow.runs.workspaceTitle"),
  );

  async function runAction(
    action: "pause" | "resume" | "cancel",
    operation: () => Promise<FlowRun>,
  ) {
    if (busy) return;
    setBusy(action);
    setError(null);
    try {
      await operation();
      await store.load(true);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  }

  if (!selected) {
    return (
      <div className="enterprise-agent-prompt-empty" role="status">
        <Activity aria-hidden="true" size={20} />
        <strong>{t("flow.runs.none")}</strong>
        <p>{t("flow.runs.noneHint")}</p>
      </div>
    );
  }

  const canPause = ["queued", "running", "resuming"].includes(selected.status);
  const canResume = ["paused", "waiting_approval", "waiting_human"].includes(
    selected.status,
  );
  const canCancel = ![
    "succeeded",
    "failed",
    "cancel_requested",
    "cancelled",
  ].includes(selected.status);
  const status = runStatusPresentation(selected.status, language);
  const endedAt = selected.completedAt ?? selected.updatedAt;

  return (
    <>
      <FlowInspectorPortal>
        <FlowInspectorPanel
          actions={
            <>
              <IconButton
                aria-label={t("flow.runs.refresh")}
                disabled={Boolean(busy)}
                onClick={() => void store.load(true)}
                size="compact"
              >
                <RefreshCw aria-hidden="true" size={14} />
              </IconButton>
              {canPause ? (
                <Button
                  disabled={Boolean(busy)}
                  onClick={() =>
                    void runAction("pause", () =>
                      client.pauseFlowRun(selected.id),
                    )
                  }
                  size="compact"
                  variant="primary"
                >
                  <PauseCircle aria-hidden="true" size={14} />
                  {busy === "pause"
                    ? t("flow.runs.pausing")
                    : t("flow.runs.pause")}
                </Button>
              ) : canResume ? (
                <Button
                  disabled={Boolean(busy)}
                  onClick={() =>
                    void runAction("resume", () =>
                      client.resumeFlowRun(selected.id),
                    )
                  }
                  size="compact"
                  variant="primary"
                >
                  <PlayCircle aria-hidden="true" size={14} />
                  {busy === "resume"
                    ? t("flow.runs.resuming")
                    : t("flow.runs.resume")}
                </Button>
              ) : null}
            </>
          }
          status={status.label}
          statusVariant={status.variant}
          subtitle={`${flowName} · v${selected.flowVersion}`}
          title={t("flow.runs.overview")}
        >
          {snapshot.error || error ? (
            <p className="enterprise-page__message is-error" role="alert">
              {snapshot.error ?? error}
            </p>
          ) : null}
          <FlowInspectorSection title={t("flow.runs.time")}>
            <dl className="enterprise-facts flow-inspector-facts">
              <div>
                <dt>{t("flow.runs.started")}</dt>
                <dd>{formatDateTime(selected.startedAt, language)}</dd>
              </div>
              <div>
                <dt>
                  {selected.completedAt
                    ? t("flow.runs.completed")
                    : t("flow.runs.updated")}
                </dt>
                <dd>{formatDateTime(endedAt, language)}</dd>
              </div>
              <div>
                <dt>{t("flow.runs.duration")}</dt>
                <dd>{formatDuration(selected.startedAt, endedAt, language)}</dd>
              </div>
            </dl>
          </FlowInspectorSection>
          <FlowInspectorSection title={t("flow.runs.usageBudget")}>
            <dl className="enterprise-facts flow-inspector-facts">
              <div>
                <dt>{t("flow.runs.nodeExecutions")}</dt>
                <dd>
                  {selected.nodeExecutions}/{selected.budget.maxNodeExecutions}
                </dd>
              </div>
              <div>
                <dt>{t("flow.runs.toolCalls")}</dt>
                <dd>
                  {selected.toolCalls}/{selected.budget.maxToolCalls}
                </dd>
              </div>
              <div>
                <dt>{t("flow.runs.checkpoints")}</dt>
                <dd>{selected.checkpointHistory.length}</dd>
              </div>
              <div>
                <dt>{t("flow.runs.maxDuration")}</dt>
                <dd>
                  {selected.budget.maxDurationSeconds} {t("flow.runs.seconds")}
                </dd>
              </div>
            </dl>
          </FlowInspectorSection>
          <FlowInspectorSection title={t("flow.runs.identifiers")}>
            <dl className="enterprise-facts flow-inspector-facts">
              <div>
                <dt>{t("flow.runs.runId")}</dt>
                <dd>
                  <code>{selected.id}</code>
                </dd>
              </div>
              <div>
                <dt>{t("flow.runs.thread")}</dt>
                <dd>
                  <code>{selected.threadId}</code>
                </dd>
              </div>
            </dl>
          </FlowInspectorSection>
          {canCancel ? (
            <FlowInspectorSection title={t("flow.runs.controls")}>
              <Button
                disabled={Boolean(busy)}
                onClick={() =>
                  void runAction("cancel", () =>
                    client.cancelFlowRun(selected.id),
                  )
                }
                size="compact"
                variant="danger"
              >
                <XCircle aria-hidden="true" size={14} />
                {busy === "cancel"
                  ? t("flow.runs.cancelling")
                  : t("flow.runs.cancel")}
              </Button>
            </FlowInspectorSection>
          ) : null}
        </FlowInspectorPanel>
      </FlowInspectorPortal>
      <div className="enterprise-page enterprise-runs enterprise-core-detail">
        <RunDetails flowName={flowName} run={selected} />
      </div>
    </>
  );
}
