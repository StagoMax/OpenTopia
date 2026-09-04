import { Clock3, FlaskConical } from "lucide-react";
import type { FlowNodeRun, FlowRun } from "../../types";
import { Badge, DisclosureSummary } from "../ui";
import { useApplicationLanguage } from "../../ApplicationLanguageProvider";
import { interfaceMessage } from "../../applicationLanguage";

export function FlowNodeTestRunDetails({
  nodeId,
  run,
}: {
  nodeId: string;
  run: FlowRun | null;
}) {
  const { language, t } = useApplicationLanguage();
  if (!run) return null;
  const nodeRun = latestNodeRun(run.nodeRuns, nodeId);
  const ready = run.readyNodes.includes(nodeId);

  return (
    <section className="flow-editor-inspector__section flow-node-test-result">
      <header>
        <span>
          <strong>{t("flow.nodeTest.latest")}</strong>
          <small>
            {nodeRun
              ? language === "zh-CN"
                ? `第 ${nodeRun.attempt} ${t("flow.nodeTest.attempt")}`
                : `${t("flow.nodeTest.attempt")} ${nodeRun.attempt}`
              : t("flow.nodeTest.trace")}
          </small>
        </span>
        <Badge variant={statusVariant(nodeRun?.status, ready)}>
          {nodeRun
            ? nodeRunStatusLabel(nodeRun.status, language)
            : ready
              ? t("flow.nodeTest.ready")
              : t("flow.nodeTest.notVisited")}
        </Badge>
      </header>

      {nodeRun ? (
        <>
          <div className="flow-node-test-result__timing">
            <Clock3 aria-hidden="true" size={14} />
            <span>{durationLabel(nodeRun)}</span>
            <span>
              {nodeRun.toolCalls} {t("flow.nodeTest.toolCalls")}
            </span>
          </div>
          <JsonResult label={t("flow.nodeTest.input")} value={nodeRun.input} />
          {nodeRun.output !== null ? (
            <JsonResult label={t("flow.nodeTest.output")} value={nodeRun.output} />
          ) : null}
          {nodeRun.error ? (
            <p className="flow-node-test-result__error" role="alert">
              {nodeRun.error}
            </p>
          ) : null}
        </>
      ) : (
        <p className="flow-editor-inspector__note">
          <FlaskConical aria-hidden="true" size={13} />
          {ready
            ? t("flow.nodeTest.queued")
            : t("flow.nodeTest.skipped")}
        </p>
      )}
    </section>
  );
}

function JsonResult({ label, value }: { label: string; value: unknown }) {
  return (
    <details className="flow-node-test-result__json">
      <DisclosureSummary>{label}</DisclosureSummary>
      <pre>{formatJson(value)}</pre>
    </details>
  );
}

function latestNodeRun(nodeRuns: FlowNodeRun[], nodeId: string) {
  return nodeRuns
    .filter((candidate) => candidate.nodeId === nodeId)
    .sort((left, right) => right.attempt - left.attempt)[0];
}

function formatJson(value: unknown) {
  const formatted = JSON.stringify(value, null, 2);
  return formatted ?? String(value);
}

function durationLabel(nodeRun: FlowNodeRun) {
  if (!nodeRun.completedAt) return "…";
  const duration =
    new Date(nodeRun.completedAt).getTime() -
    new Date(nodeRun.startedAt).getTime();
  if (!Number.isFinite(duration) || duration < 0) return "—";
  return duration < 1_000
    ? `${duration} ms`
    : `${(duration / 1_000).toFixed(1)} s`;
}

function nodeRunStatusLabel(
  status: FlowNodeRun["status"],
  language: import("../../applicationLanguage").ApplicationLanguage,
) {
  if (status === "succeeded")
    return interfaceMessage(language, "flow.nodeStatus.succeeded");
  if (status === "failed")
    return interfaceMessage(language, "flow.nodeStatus.failed");
  if (status === "cancelled")
    return interfaceMessage(language, "flow.nodeStatus.cancelled");
  if (status === "waiting_approval" || status === "waiting_human")
    return interfaceMessage(language, "flow.nodeTest.waitingHuman");
  if (status === "resuming")
    return interfaceMessage(language, "flow.nodeStatus.resuming");
  return interfaceMessage(language, "flow.nodeStatus.running");
}

function statusVariant(
  status: FlowNodeRun["status"] | undefined,
  ready: boolean,
): "success" | "danger" | "warning" | "info" | "neutral" {
  if (status === "succeeded") return "success";
  if (status === "failed" || status === "cancelled") return "danger";
  if (status === "waiting_approval" || status === "waiting_human")
    return "warning";
  if (status || ready) return "info";
  return "neutral";
}
