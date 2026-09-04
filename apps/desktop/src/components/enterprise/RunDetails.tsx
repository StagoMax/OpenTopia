import {
  Activity,
  CheckCircle2,
  ChevronDown,
  CirclePause,
  CircleX,
  Clock3,
  Gauge,
  ListChecks,
  LoaderCircle,
  Wrench,
} from "lucide-react";
import type {
  FlowNodeRun,
  FlowRun,
  FlowRunStatus,
  WorkflowCheckpointStatus,
} from "../../types";
import { Badge } from "../ui";
import { humanizeIdentifier } from "./enterpriseSidebarPresentation";
import {
  checkpointStatusLabel,
  formatDateTime,
  formatDuration,
  runStatusPresentation,
} from "./runPresentation";
import { StructuredPayload } from "./StructuredPayload";
import { useApplicationLanguage } from "../../ApplicationLanguageProvider";
import {
  interfaceMessage,
  type ApplicationLanguage,
} from "../../applicationLanguage";

export function RunDetails({
  flowName,
  run,
}: {
  flowName: string;
  run: FlowRun;
}) {
  const { language, t } = useApplicationLanguage();
  const status = runStatusPresentation(run.status, language);
  const endedAt = run.completedAt ?? run.updatedAt;
  const workflow = run.flowRevision?.compiledWorkflow;

  return (
    <article
      className="run-detail"
      aria-label={`${flowName} ${t("flow.runDetails.aria")}`}
    >
      <section
        className={`enterprise-core-detail__summary ${runToneClass(run.status)}`}
      >
        <span className="enterprise-core-detail__icon" aria-hidden="true">
          <RunStatusIcon status={run.status} />
        </span>
        <div>
          <small>
            {t("flow.runDetails.record")} ·{" "}
            {t("flow.runDetails.workflowVersion")}
            {run.flowVersion}
          </small>
          <span className="run-detail__title-row">
            <h2>{flowName}</h2>
            <Badge variant={status.variant}>{status.label}</Badge>
          </span>
          <p>
            {status.description} {t("flow.runDetails.summaryPrefix")}{" "}
            {run.nodeExecutions} {t("flow.runDetails.nodes")} {run.toolCalls}{" "}
            {t("flow.runDetails.tools")}
          </p>
        </div>
      </section>

      <section
        className="run-detail__metrics"
        aria-label={t("flow.runDetails.summaryAria")}
      >
        <RunMetric
          detail={
            run.completedAt
              ? t("flow.runDetails.finished")
              : t("flow.runDetails.asOfUpdate")
          }
          icon={<Clock3 aria-hidden="true" size={16} />}
          label={t("flow.runs.duration")}
          value={formatDuration(run.startedAt, endedAt, language)}
        />
        <RunMetric
          detail={`${t("flow.runDetails.budgetLimit")} ${run.budget.maxNodeExecutions}`}
          icon={<ListChecks aria-hidden="true" size={16} />}
          label={t("flow.runs.nodeExecutions")}
          value={String(run.nodeExecutions)}
        />
        <RunMetric
          detail={`${t("flow.runDetails.budgetLimit")} ${run.budget.maxToolCalls}`}
          icon={<Wrench aria-hidden="true" size={16} />}
          label={t("flow.runs.toolCalls")}
          value={String(run.toolCalls)}
        />
        <RunMetric
          detail={`${t("flow.runDetails.supersteps")} ${run.superstep} ${t("flow.runDetails.rounds")}`}
          icon={<Gauge aria-hidden="true" size={16} />}
          label={t("flow.runs.checkpoints")}
          value={String(run.checkpointHistory.length)}
        />
      </section>

      {run.error ? (
        <p className="run-detail__error" role="alert">
          <CircleX aria-hidden="true" size={16} />
          <span>
            <strong>{t("flow.runDetails.incomplete")}</strong>
            {run.error}
          </span>
        </p>
      ) : null}

      <section className="enterprise-core-detail__payload run-detail__result">
        <header>
          <span>
            <h3>{t("flow.runDetails.result")}</h3>
            <small>{t("flow.runDetails.resultHint")}</small>
          </span>
        </header>
        <StructuredPayload
          emptyLabel={t("flow.runDetails.noResult")}
          schema={workflow?.outputSchema}
          value={run.output}
        />
      </section>

      <section className="enterprise-core-detail__payload run-detail__path">
        <header>
          <span>
            <h3>{t("flow.runDetails.path")}</h3>
            <small>{t("flow.runDetails.pathHint")}</small>
          </span>
          <Badge variant="neutral">
            {run.nodeRuns.length} {t("flow.runDetails.records")}
          </Badge>
        </header>
        <ol className="run-timeline">
          {run.nodeRuns.map((node, index) => (
            <NodeTimelineItem
              isLast={index === run.nodeRuns.length - 1}
              key={node.id}
              label={nodeLabel(run, node.nodeId)}
              node={node}
            />
          ))}
          {run.nodeRuns.length === 0 ? (
            <li className="run-detail__empty">
              {t("flow.runDetails.noNodeRuns")}
            </li>
          ) : null}
        </ol>
      </section>

      <details className="run-diagnostics">
        <summary>
          <span>
            <strong>{t("flow.runDetails.diagnostics")}</strong>
            <small>{t("flow.runDetails.diagnosticsHint")}</small>
          </span>
          <span className="run-diagnostics__summary-meta">
            {run.checkpointHistory.length} {t("flow.runs.checkpoints")}
            <ChevronDown aria-hidden="true" size={16} />
          </span>
        </summary>
        <div className="run-diagnostics__body">
          <section>
            <header>
              <h3>{t("flow.runDetails.input")}</h3>
            </header>
            <StructuredPayload
              emptyLabel={t("flow.runDetails.noInput")}
              schema={workflow?.inputSchema}
              value={run.input}
            />
          </section>
          <section>
            <header>
              <h3>{t("flow.runDetails.recoveryCheckpoints")}</h3>
            </header>
            <ol className="run-checkpoints">
              {run.checkpointHistory.map((checkpoint) => (
                <li key={checkpoint.id}>
                  <span className="run-checkpoints__step">
                    {checkpoint.superstep}
                  </span>
                  <span>
                    <strong>
                      {checkpoint.nodeIds
                        .map((nodeId) => nodeLabel(run, nodeId))
                        .join(language === "zh-CN" ? "、" : ", ") ||
                        t("flow.runDetails.noNode")}
                    </strong>
                    <small>
                      {checkpoint.pendingWriteCount}{" "}
                      {t("flow.runDetails.stateWrites")} ·{" "}
                      {formatDateTime(checkpoint.completedAt, language)}
                    </small>
                  </span>
                  <Badge variant={checkpointVariant(checkpoint.status)}>
                    {checkpointStatusLabel(checkpoint.status, language)}
                  </Badge>
                </li>
              ))}
              {run.checkpointHistory.length === 0 ? (
                <li className="run-detail__empty">
                  {t("flow.runDetails.noCheckpoints")}
                </li>
              ) : null}
            </ol>
          </section>
        </div>
      </details>
    </article>
  );
}

function RunMetric({
  detail,
  icon,
  label,
  value,
}: {
  detail: string;
  icon: React.ReactNode;
  label: string;
  value: string;
}) {
  return (
    <div className="run-detail__metric">
      <span className="run-detail__metric-icon">{icon}</span>
      <span>
        <small>{label}</small>
        <strong>{value}</strong>
        <span>{detail}</span>
      </span>
    </div>
  );
}

function NodeTimelineItem({
  isLast,
  label,
  node,
}: {
  isLast: boolean;
  label: string;
  node: FlowNodeRun;
}) {
  const { language, t } = useApplicationLanguage();
  const status = nodeStatusPresentation(node.status, language);
  return (
    <li className={`run-timeline__item is-${node.status}`}>
      <span
        className={`run-timeline__marker ${isLast ? "is-last" : ""}`}
        aria-hidden="true"
      >
        <NodeStatusIcon status={node.status} />
      </span>
      <span className="run-timeline__content">
        <span className="run-timeline__heading">
          <strong>{label}</strong>
          <Badge variant={status.variant}>{status.label}</Badge>
        </span>
        <small>
          <code>{node.nodeId}</code>
          {node.attempt > 1
            ? language === "zh-CN"
              ? ` · ${t("flow.runDetails.attempt")}${node.attempt}${t("flow.runDetails.attemptSuffix")}`
              : ` · ${t("flow.runDetails.attempt")} ${node.attempt}`
            : ""}{" "}
          · {formatDuration(node.startedAt, node.completedAt, language)} ·{" "}
          {node.toolCalls > 0
            ? `${node.toolCalls} ${t("flow.runDetails.toolCallCount")}`
            : t("flow.runDetails.noToolCalls")}
        </small>
        {node.error ? (
          <span className="run-timeline__error">{node.error}</span>
        ) : null}
      </span>
    </li>
  );
}

function RunStatusIcon({ status }: { status: FlowRunStatus }) {
  if (status === "succeeded") return <CheckCircle2 size={20} />;
  if (status === "failed" || status === "cancelled") {
    return <CircleX size={20} />;
  }
  if (
    status === "paused" ||
    status === "waiting_approval" ||
    status === "waiting_human"
  ) {
    return <CirclePause size={20} />;
  }
  if (status === "running" || status === "resuming") {
    return <LoaderCircle size={20} />;
  }
  return <Activity size={20} />;
}

function NodeStatusIcon({ status }: { status: FlowNodeRun["status"] }) {
  if (status === "succeeded") return <CheckCircle2 size={16} />;
  if (status === "failed" || status === "cancelled") {
    return <CircleX size={16} />;
  }
  if (status === "waiting_approval" || status === "waiting_human") {
    return <CirclePause size={16} />;
  }
  if (status === "running" || status === "resuming") {
    return <LoaderCircle size={16} />;
  }
  return <Activity size={16} />;
}

function nodeStatusPresentation(
  status: FlowNodeRun["status"],
  language: ApplicationLanguage,
): {
  label: string;
  variant: "success" | "danger" | "warning" | "info" | "neutral";
} {
  if (status === "succeeded")
    return {
      label: interfaceMessage(language, "flow.nodeStatus.succeeded"),
      variant: "success",
    };
  if (status === "failed")
    return {
      label: interfaceMessage(language, "flow.nodeStatus.failed"),
      variant: "danger",
    };
  if (status === "cancelled")
    return {
      label: interfaceMessage(language, "flow.nodeStatus.cancelled"),
      variant: "danger",
    };
  if (status === "waiting_approval") {
    return {
      label: interfaceMessage(language, "flow.nodeStatus.waitingApproval"),
      variant: "warning",
    };
  }
  if (status === "waiting_human") {
    return {
      label: interfaceMessage(language, "flow.nodeStatus.waitingHuman"),
      variant: "warning",
    };
  }
  if (status === "resuming")
    return {
      label: interfaceMessage(language, "flow.nodeStatus.resuming"),
      variant: "info",
    };
  return {
    label: interfaceMessage(language, "flow.nodeStatus.running"),
    variant: "info",
  };
}

function nodeLabel(run: FlowRun, nodeId: string): string {
  const label = run.graph.nodes.find((node) => node.id === nodeId)?.label;
  return humanizeIdentifier(label || nodeId);
}

function checkpointVariant(
  status: WorkflowCheckpointStatus,
): "success" | "danger" | "neutral" {
  if (status === "committed") return "success";
  if (status === "failed" || status === "cancelled") return "danger";
  return "neutral";
}

function runToneClass(status: FlowRunStatus): string {
  if (status === "succeeded") return "is-healthy";
  if (status === "failed" || status === "cancelled") return "is-warning";
  if (
    status === "paused" ||
    status === "waiting_approval" ||
    status === "waiting_human"
  ) {
    return "is-attention";
  }
  return "";
}
