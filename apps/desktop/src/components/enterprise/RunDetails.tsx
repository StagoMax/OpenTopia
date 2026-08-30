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
  formatScalarValue,
  payloadFields,
  payloadItemSchema,
  runStatusPresentation,
} from "./runPresentation";

export function RunDetails({
  flowName,
  run,
}: {
  flowName: string;
  run: FlowRun;
}) {
  const status = runStatusPresentation(run.status);
  const endedAt = run.completedAt ?? run.updatedAt;
  const workflow = run.flowRevision?.compiledWorkflow;

  return (
    <article className="run-detail" aria-label={`${flowName} 运行详情`}>
      <section
        className={`enterprise-core-detail__summary ${runToneClass(run.status)}`}
      >
        <span className="enterprise-core-detail__icon" aria-hidden="true">
          <RunStatusIcon status={run.status} />
        </span>
        <div>
          <small>运行记录 · Flow v{run.flowVersion}</small>
          <span className="run-detail__title-row">
            <h2>{flowName}</h2>
            <Badge variant={status.variant}>{status.label}</Badge>
          </span>
          <p>
            {status.description} 本次共执行 {run.nodeExecutions} 个节点，调用了{" "}
            {run.toolCalls} 次工具。
          </p>
        </div>
      </section>

      <section className="run-detail__metrics" aria-label="本次运行摘要">
        <RunMetric
          detail={run.completedAt ? "已完成" : "截至最近更新"}
          icon={<Clock3 aria-hidden="true" size={16} />}
          label="耗时"
          value={formatDuration(run.startedAt, endedAt)}
        />
        <RunMetric
          detail={`预算上限 ${run.budget.maxNodeExecutions}`}
          icon={<ListChecks aria-hidden="true" size={16} />}
          label="节点执行"
          value={String(run.nodeExecutions)}
        />
        <RunMetric
          detail={`预算上限 ${run.budget.maxToolCalls}`}
          icon={<Wrench aria-hidden="true" size={16} />}
          label="工具调用"
          value={String(run.toolCalls)}
        />
        <RunMetric
          detail={`已推进 ${run.superstep} 轮`}
          icon={<Gauge aria-hidden="true" size={16} />}
          label="检查点"
          value={String(run.checkpointHistory.length)}
        />
      </section>

      {run.error ? (
        <p className="run-detail__error" role="alert">
          <CircleX aria-hidden="true" size={16} />
          <span>
            <strong>运行未完成</strong>
            {run.error}
          </span>
        </p>
      ) : null}

      <section className="enterprise-core-detail__payload run-detail__result">
        <header>
          <span>
            <strong>运行结果</strong>
            <small>流程最终返回给调用方的数据</small>
          </span>
        </header>
        <RunPayload
          emptyLabel="本次运行尚未生成结果。"
          schema={workflow?.outputSchema}
          value={run.output}
        />
      </section>

      <section className="enterprise-core-detail__payload run-detail__path">
        <header>
          <span>
            <strong>执行路径</strong>
            <small>按实际发生顺序展示节点、重试与工具调用</small>
          </span>
          <Badge variant="neutral">{run.nodeRuns.length} 条记录</Badge>
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
            <li className="run-detail__empty">尚无节点执行记录。</li>
          ) : null}
        </ol>
      </section>

      <details className="run-diagnostics">
        <summary>
          <span>
            <strong>输入、检查点与技术信息</strong>
            <small>需要排查、恢复或审计时再展开</small>
          </span>
          <span className="run-diagnostics__summary-meta">
            {run.checkpointHistory.length} 个检查点
            <ChevronDown aria-hidden="true" size={16} />
          </span>
        </summary>
        <div className="run-diagnostics__body">
          <section>
            <header>
              <strong>本次输入</strong>
            </header>
            <RunPayload
              emptyLabel="本次运行没有输入数据。"
              schema={workflow?.inputSchema}
              value={run.input}
            />
          </section>
          <section>
            <header>
              <strong>恢复检查点</strong>
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
                        .join("、") || "无节点"}
                    </strong>
                    <small>
                      {checkpoint.pendingWriteCount} 次状态写入 ·{" "}
                      {formatDateTime(checkpoint.completedAt)}
                    </small>
                  </span>
                  <Badge variant={checkpointVariant(checkpoint.status)}>
                    {checkpointStatusLabel(checkpoint.status)}
                  </Badge>
                </li>
              ))}
              {run.checkpointHistory.length === 0 ? (
                <li className="run-detail__empty">尚未生成恢复检查点。</li>
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
  const status = nodeStatusPresentation(node.status);
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
          {node.attempt > 1 ? ` · 第 ${node.attempt} 次尝试` : ""} ·{" "}
          {formatDuration(node.startedAt, node.completedAt)} ·{" "}
          {node.toolCalls > 0 ? `${node.toolCalls} 次工具调用` : "未调用工具"}
        </small>
        {node.error ? (
          <span className="run-timeline__error">{node.error}</span>
        ) : null}
      </span>
    </li>
  );
}

function RunPayload({
  emptyLabel,
  schema,
  value,
}: {
  emptyLabel: string;
  schema?: unknown;
  value: unknown;
}) {
  if (value === null || value === undefined) {
    return <p className="run-detail__empty">{emptyLabel}</p>;
  }

  if (Array.isArray(value)) {
    if (value.length === 0) {
      return <p className="run-detail__empty">{emptyLabel}</p>;
    }
    return (
      <ol className="run-payload-list">
        {value.map((item, index) => (
          <li key={index}>
            <PayloadValue schema={payloadItemSchema(schema)} value={item} />
          </li>
        ))}
      </ol>
    );
  }

  if (typeof value === "object") {
    const fields = payloadFields(value as Record<string, unknown>, schema);
    if (fields.length === 0) {
      return <p className="run-detail__empty">{emptyLabel}</p>;
    }
    return (
      <dl className="run-payload">
        {fields.map((field) => (
          <div key={field.key}>
            <dt>
              <span>{field.label}</span>
              {field.description ? <small>{field.description}</small> : null}
            </dt>
            <dd>
              <PayloadValue schema={field.schema} value={field.value} />
            </dd>
          </div>
        ))}
      </dl>
    );
  }

  return (
    <p className="run-payload__text">
      {formatScalarValue(value) ?? String(value)}
    </p>
  );
}

function PayloadValue({ schema, value }: { schema?: unknown; value: unknown }) {
  const scalar = formatScalarValue(value);
  if (scalar !== null)
    return <span className="run-payload__text">{scalar}</span>;

  const count = Array.isArray(value)
    ? `${value.length} 项`
    : `${Object.keys(value as Record<string, unknown>).length} 个字段`;
  return (
    <details className="run-payload__structured">
      <summary>{count}，查看详情</summary>
      <RunPayload emptyLabel="无数据" schema={schema} value={value} />
    </details>
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

function nodeStatusPresentation(status: FlowNodeRun["status"]): {
  label: string;
  variant: "success" | "danger" | "warning" | "info" | "neutral";
} {
  if (status === "succeeded") return { label: "成功", variant: "success" };
  if (status === "failed") return { label: "失败", variant: "danger" };
  if (status === "cancelled") return { label: "已取消", variant: "danger" };
  if (status === "waiting_approval") {
    return { label: "等待审批", variant: "warning" };
  }
  if (status === "waiting_human") {
    return { label: "等待处理", variant: "warning" };
  }
  if (status === "resuming") return { label: "恢复中", variant: "info" };
  return { label: "运行中", variant: "info" };
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
