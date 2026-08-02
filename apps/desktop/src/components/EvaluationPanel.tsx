import { useCallback, useEffect, useMemo, useState } from "react";
import { ExternalLink, Loader2, RefreshCw, Wrench } from "lucide-react";
import type { ApiClient } from "../api/client";
import type { EvaluationRun, EvaluationTaskResult } from "../types";
import { Badge, Button, Panel } from "./ui";
import "./EvaluationPanel.css";

type EvaluationPanelProps = {
  client: ApiClient | null;
  workspaceRoot: string | null;
  onOpenPath(path: string): void;
};

type StatusVariant = "neutral" | "info" | "success" | "warning" | "danger";

function statusVariant(status: string): StatusVariant {
  switch (status.toLowerCase()) {
    case "passed":
    case "succeeded":
      return "success";
    case "failed":
    case "infra_error":
      return "danger";
    case "inconclusive":
    case "cancelled":
      return "warning";
    case "running":
      return "info";
    default:
      return "neutral";
  }
}

function statusLabel(status: string): string {
  switch (status.toLowerCase()) {
    case "passed":
      return "通过";
    case "succeeded":
      return "完成";
    case "failed":
      return "失败";
    case "infra_error":
      return "基础设施错误";
    case "inconclusive":
      return "不计分";
    case "running":
      return "运行中";
    default:
      return status || "未知";
  }
}

function formatDate(value?: string | null): string {
  if (!value) return "时间未记录";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat("zh-CN", {
    dateStyle: "short",
    timeStyle: "medium",
  }).format(date);
}

function formatTokens(value?: number | null): string | null {
  if (value === undefined || value === null) return null;
  return new Intl.NumberFormat("zh-CN").format(value);
}

function taskLabel(task: EvaluationTaskResult): string {
  return task.title ? `${task.taskId} · ${task.title}` : task.taskId;
}

export function EvaluationPanel({
  client,
  workspaceRoot,
  onOpenPath,
}: EvaluationPanelProps) {
  const [runs, setRuns] = useState<EvaluationRun[]>([]);
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!client || !workspaceRoot) {
      setRuns([]);
      setSelectedRunId(null);
      return;
    }
    setIsRefreshing(true);
    setError(null);
    try {
      const nextRuns = await client.importEvaluationRuns(workspaceRoot);
      setRuns(nextRuns);
      setSelectedRunId((current) =>
        current && nextRuns.some((run) => run.runId === current)
          ? current
          : (nextRuns[0]?.runId ?? null),
      );
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "评测结果读取失败。");
    } finally {
      setIsRefreshing(false);
    }
  }, [client, workspaceRoot]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const selectedRun = useMemo(
    () => runs.find((run) => run.runId === selectedRunId) ?? runs[0] ?? null,
    [runs, selectedRunId],
  );

  return (
    <Panel
      className="evaluation-panel"
      title="评测"
      actions={
        <Button
          size="compact"
          variant="quiet"
          disabled={!workspaceRoot || isRefreshing}
          onClick={() => void refresh()}
          title="刷新评测结果"
        >
          {isRefreshing ? (
            <Loader2 className="spin" size={14} aria-hidden="true" />
          ) : (
            <RefreshCw size={14} aria-hidden="true" />
          )}
          <span>{isRefreshing ? "正在读取" : "刷新"}</span>
        </Button>
      }
    >
      {!workspaceRoot ? (
        <p className="evaluation-panel__empty">请先打开一个工作区。</p>
      ) : null}
      {error ? (
        <p className="evaluation-panel__error" role="alert">
          {error}
        </p>
      ) : null}
      {workspaceRoot && !isRefreshing && runs.length === 0 && !error ? (
        <p className="evaluation-panel__empty">
          当前工作区还没有可导入的评测结果。运行评测后点击刷新即可查看。
        </p>
      ) : null}
      {runs.length > 0 ? (
        <div className="evaluation-panel__layout">
          <div className="evaluation-panel__runs" aria-label="评测运行列表">
            {runs.map((run) => {
              const active = run.runId === selectedRun?.runId;
              return (
                <button
                  key={run.runId}
                  type="button"
                  className={`evaluation-run ${active ? "is-active" : ""}`}
                  aria-pressed={active}
                  onClick={() => setSelectedRunId(run.runId)}
                >
                  <span className="evaluation-run__title">{run.title}</span>
                  <span className="evaluation-run__meta">
                    <Badge variant={statusVariant(run.status)}>
                      {statusLabel(run.status)}
                    </Badge>
                    <span>{formatDate(run.completedAt ?? run.startedAt)}</span>
                  </span>
                </button>
              );
            })}
          </div>
          {selectedRun ? (
            <section className="evaluation-panel__detail" aria-label="评测详情">
              <header className="evaluation-detail__header">
                <div>
                  <h3>{selectedRun.title}</h3>
                  <p>
                    {selectedRun.model ? `模型：${selectedRun.model} · ` : ""}
                    {formatDate(selectedRun.completedAt ?? selectedRun.startedAt)}
                  </p>
                </div>
                <Badge variant={statusVariant(selectedRun.status)}>
                  {statusLabel(selectedRun.status)}
                </Badge>
              </header>
              {selectedRun.failureCategory ? (
                <p className="evaluation-detail__attribution">
                  归因：{selectedRun.failureCategory}
                </p>
              ) : null}
              <div className="evaluation-detail__tasks">
                {selectedRun.tasks.length > 0 ? (
                  selectedRun.tasks.map((task, index) => (
                    <article
                      className="evaluation-task"
                      key={`${task.taskId}-${task.runId ?? index}`}
                    >
                      <header>
                        <span>{taskLabel(task)}</span>
                        <Badge variant={statusVariant(task.status)}>
                          {statusLabel(task.status)}
                        </Badge>
                      </header>
                      {task.failureCategory ? (
                        <p className="evaluation-task__attribution">
                          归因：{task.failureCategory}
                        </p>
                      ) : null}
                      {task.error ? (
                        <p className="evaluation-task__error">{task.error}</p>
                      ) : null}
                      <TaskMetrics task={task} />
                    </article>
                  ))
                ) : (
                  <p className="evaluation-panel__empty">该评测未提供任务级明细。</p>
                )}
              </div>
              <Button
                className="evaluation-detail__source"
                size="compact"
                variant="quiet"
                onClick={() => onOpenPath(selectedRun.sourcePath)}
              >
                <ExternalLink size={14} aria-hidden="true" />
                打开评测产物目录
              </Button>
            </section>
          ) : null}
        </div>
      ) : null}
    </Panel>
  );
}

function TaskMetrics({ task }: { task: EvaluationTaskResult }) {
  const toolCalls = Object.entries(task.toolCallsByName);
  const tokenCount = formatTokens(task.totalTokens);
  const facts = [
    tokenCount ? `Tokens ${tokenCount}` : null,
    task.errorEvents !== undefined && task.errorEvents !== null
      ? `错误事件 ${task.errorEvents}`
      : null,
    task.recoveryPassed !== undefined && task.recoveryPassed !== null
      ? `恢复 ${task.recoveryPassed ? "通过" : "未通过"}`
      : null,
    task.processContractPassed !== undefined &&
    task.processContractPassed !== null
      ? `过程契约 ${task.processContractPassed ? "通过" : "未通过"}`
      : null,
  ].filter((value): value is string => Boolean(value));

  return (
    <div className="evaluation-task__metrics">
      {toolCalls.length > 0 ? (
        <div className="evaluation-task__tools">
          <span className="evaluation-task__tools-label">
            <Wrench size={14} aria-hidden="true" />
            工具调用
          </span>
          <span className="evaluation-task__tools-values">
            {toolCalls.map(([name, count]) => (
              <Badge key={name} variant="neutral">
                {name} {count}
              </Badge>
            ))}
          </span>
        </div>
      ) : null}
      {facts.length > 0 ? (
        <p className="evaluation-task__facts">{facts.join(" · ")}</p>
      ) : null}
    </div>
  );
}
