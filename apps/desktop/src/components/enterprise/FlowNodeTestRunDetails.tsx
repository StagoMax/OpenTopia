import { Clock3, FlaskConical } from "lucide-react";
import type { FlowNodeRun, FlowRun } from "../../types";
import { Badge } from "../ui";

export function FlowNodeTestRunDetails({
  nodeId,
  run,
}: {
  nodeId: string;
  run: FlowRun | null;
}) {
  if (!run) return null;
  const nodeRun = latestNodeRun(run.nodeRuns, nodeId);
  const ready = run.readyNodes.includes(nodeId);

  return (
    <section className="flow-editor-inspector__section flow-node-test-result">
      <header>
        <span>
          <strong>最近一次 Test Run</strong>
          <small>
            {nodeRun ? `第 ${nodeRun.attempt} 次执行` : "本次运行轨迹"}
          </small>
        </span>
        <Badge variant={statusVariant(nodeRun?.status, ready)}>
          {nodeRun
            ? nodeRunStatusLabel(nodeRun.status)
            : ready
              ? "就绪"
              : "未经过"}
        </Badge>
      </header>

      {nodeRun ? (
        <>
          <div className="flow-node-test-result__timing">
            <Clock3 aria-hidden="true" size={14} />
            <span>{durationLabel(nodeRun)}</span>
            <span>{nodeRun.toolCalls} 次 Tool 调用</span>
          </div>
          <JsonResult label="输入" value={nodeRun.input} />
          {nodeRun.output !== null ? (
            <JsonResult label="输出" value={nodeRun.output} />
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
            ? "节点已经进入待执行队列。"
            : "最近一次测试没有经过这个节点，可能是分支条件未满足。"}
        </p>
      )}
    </section>
  );
}

function JsonResult({ label, value }: { label: string; value: unknown }) {
  return (
    <details className="flow-node-test-result__json">
      <summary>{label}</summary>
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
  if (!nodeRun.completedAt) return "执行中";
  const duration =
    new Date(nodeRun.completedAt).getTime() -
    new Date(nodeRun.startedAt).getTime();
  if (!Number.isFinite(duration) || duration < 0) return "已完成";
  return duration < 1_000
    ? `${duration} ms`
    : `${(duration / 1_000).toFixed(1)} s`;
}

function nodeRunStatusLabel(status: FlowNodeRun["status"]) {
  if (status === "succeeded") return "成功";
  if (status === "failed") return "失败";
  if (status === "cancelled") return "已取消";
  if (status === "waiting_approval" || status === "waiting_human")
    return "等待人工";
  if (status === "resuming") return "恢复中";
  return "运行中";
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
