import { Activity, Clock3, RefreshCw, ShieldAlert } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { ApiClient } from "../../api/client";
import type { FlowRun } from "../../types";
import { Badge, Button, Panel, Select } from "../ui";
import { shortId } from "./model";
import { useEnterpriseStore } from "./store";

type RunFilter = "all" | "active" | "waiting" | "failed" | "succeeded";

export function RunsPage({ client }: { client: ApiClient }) {
  const { snapshot, store } = useEnterpriseStore(client);
  const [filter, setFilter] = useState<RunFilter>("all");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const runs = useMemo(
    () => snapshot.runs.filter((run) => matchesFilter(run, filter)),
    [filter, snapshot.runs],
  );
  const selected = runs.find((run) => run.id === selectedId) ?? runs[0] ?? null;
  useEffect(() => {
    if (selected && selected.id !== selectedId) setSelectedId(selected.id);
  }, [selected, selectedId]);

  return (
    <div className="enterprise-page enterprise-runs">
      <Panel
        title="Workflow runs / 工作流运行"
        actions={
          <div className="enterprise-actions">
            <Select<RunFilter>
              label="筛选 Workflow Run"
              onChange={setFilter}
              options={[
                { value: "all", label: "全部" },
                { value: "active", label: "运行中" },
                { value: "waiting", label: "等待人工" },
                { value: "failed", label: "失败" },
                { value: "succeeded", label: "成功" },
              ]}
              value={filter}
            />
            <Button aria-label="刷新 Workflow Runs" onClick={() => void store.load(true)} size="compact" variant="quiet">
              <RefreshCw aria-hidden="true" size={14} /> 刷新
            </Button>
          </div>
        }
      >
        <div className="enterprise-master-detail">
          <ol className="enterprise-run-list" aria-label="Workflow Run 列表">
            {runs.map((run) => (
              <li key={run.id}>
                <button
                  aria-pressed={selected?.id === run.id}
                  className={selected?.id === run.id ? "is-active" : undefined}
                  onClick={() => setSelectedId(run.id)}
                  type="button"
                >
                  <Activity aria-hidden="true" size={16} />
                  <span><strong>{run.flowId}@{run.flowVersion}</strong><small>{shortId(run.id)} · {formatTime(run.updatedAt)}</small></span>
                  <RunBadge run={run} />
                </button>
              </li>
            ))}
            {runs.length === 0 ? <li className="enterprise-list__empty">此筛选下没有 Run。</li> : null}
          </ol>
          {selected ? <RunDetails run={selected} /> : <div className="enterprise-empty-detail">选择一个 Run 查看持久化 Trace。</div>}
        </div>
      </Panel>
    </div>
  );
}

function RunDetails({ run }: { run: FlowRun }) {
  return (
    <article className="enterprise-run-detail" aria-label={`Run ${run.id} 详情`}>
      <header>
        <span><Activity aria-hidden="true" size={18} /><strong>{run.flowId}@{run.flowVersion}</strong></span>
        <RunBadge run={run} />
      </header>
      <dl className="enterprise-facts">
        <div><dt>Run ID</dt><dd><code>{run.id}</code></dd></div>
        <div><dt>Thread</dt><dd><code>{run.threadId}</code></dd></div>
        <div><dt>Supersteps</dt><dd>{run.superstep}</dd></div>
        <div><dt>Node executions</dt><dd>{run.nodeExecutions}/{run.budget.maxNodeExecutions}</dd></div>
        <div><dt>Tool calls</dt><dd>{run.toolCalls}/{run.budget.maxToolCalls}</dd></div>
        <div><dt>Updated</dt><dd>{formatTime(run.updatedAt)}</dd></div>
      </dl>
      {run.error ? <p className="enterprise-page__message is-error" role="alert"><ShieldAlert aria-hidden="true" size={15} />{run.error}</p> : null}
      <h3>Checkpoint timeline / 检查点</h3>
      <ol className="enterprise-trace">
        {run.checkpointHistory.toReversed().map((checkpoint) => (
          <li key={checkpoint.id}>
            <Clock3 aria-hidden="true" size={15} />
            <span><strong>Superstep {checkpoint.superstep}</strong><small>{checkpoint.nodeIds.join(", ") || "no nodes"} · {checkpoint.pendingWriteCount} writes</small></span>
            <Badge variant={checkpoint.status === "committed" ? "success" : checkpoint.status === "failed" ? "danger" : "neutral"}>{checkpoint.status}</Badge>
          </li>
        ))}
        {run.checkpointHistory.length === 0 ? <li className="enterprise-list__empty">尚未提交 Checkpoint。</li> : null}
      </ol>
      <h3>Node trace / 节点轨迹</h3>
      <ol className="enterprise-trace">
        {run.nodeRuns.toReversed().map((node) => (
          <li key={node.id}>
            <Activity aria-hidden="true" size={15} />
            <span><strong>{node.nodeId} · attempt {node.attempt}</strong><small>{node.toolCalls} tool calls · {node.completedAt ? formatTime(node.completedAt) : "running"}</small></span>
            <Badge variant={node.status === "succeeded" ? "success" : node.status === "failed" ? "danger" : node.status.includes("waiting") ? "warning" : "info"}>{node.status}</Badge>
          </li>
        ))}
      </ol>
    </article>
  );
}

function matchesFilter(run: FlowRun, filter: RunFilter): boolean {
  if (filter === "all") return true;
  if (filter === "active") return ["queued", "running", "resuming", "pause_requested", "cancel_requested"].includes(run.status);
  if (filter === "waiting") return ["waiting_approval", "waiting_human", "paused"].includes(run.status);
  return run.status === filter;
}

function RunBadge({ run }: { run: FlowRun }) {
  const variant = run.status === "succeeded" ? "success" : run.status === "failed" || run.status === "cancelled" ? "danger" : run.status.includes("waiting") || run.status === "paused" ? "warning" : run.status === "running" || run.status === "resuming" ? "info" : "neutral";
  return <Badge variant={variant}>{run.status.replaceAll("_", " ")}</Badge>;
}

function formatTime(value: string): string { return new Date(value).toLocaleString(); }
