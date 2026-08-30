import {
  Activity,
  Clock3,
  PauseCircle,
  PlayCircle,
  RefreshCw,
  ShieldAlert,
  XCircle,
} from "lucide-react";
import { useEffect, useState } from "react";
import type { ApiClient } from "../../api/client";
import type { FlowRun } from "../../types";
import { Badge, Button, IconButton } from "../ui";
import {
  FlowInspectorPortal,
  useFlowWorkspaceSelection,
  useFlowWorkspaceTitle,
} from "./flowAgentSelection";
import { FlowInspectorPanel, FlowInspectorSection } from "./FlowInspectorPanel";
import { useEnterpriseStore } from "./store";

export function RunsPage({ client }: { client: ApiClient }) {
  const { snapshot, store } = useEnterpriseStore(client);
  const selection = useFlowWorkspaceSelection();
  const selected =
    snapshot.runs.find((run) => run.id === selection?.selectedRunId) ??
    snapshot.runs[0] ??
    null;
  const [busy, setBusy] = useState<"pause" | "resume" | "cancel" | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (selected && selected.id !== selection?.selectedRunId) {
      selection?.setSelectedRunId(selected.id);
    }
  }, [selected, selection]);

  useFlowWorkspaceTitle(
    selected ? `${selected.flowId}@${selected.flowVersion}` : "Runs / 运行追踪",
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
        <strong>尚无 Workflow Run</strong>
        <p>Flow 开始运行后会在左侧显示。</p>
      </div>
    );
  }

  const canPause = ["queued", "running", "resuming"].includes(selected.status);
  const canResume = ["paused", "waiting_approval", "waiting_human"].includes(
    selected.status,
  );
  const canCancel = !["succeeded", "failed", "cancelled"].includes(
    selected.status,
  );

  return (
    <>
      <FlowInspectorPortal>
        <FlowInspectorPanel
          actions={
            <>
              <IconButton
                aria-label="刷新 Workflow Run"
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
                  {busy === "pause" ? "暂停中…" : "暂停"}
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
                  {busy === "resume" ? "恢复中…" : "恢复"}
                </Button>
              ) : null}
            </>
          }
          status={selected.status.replaceAll("_", " ")}
          statusVariant={runVariant(selected.status)}
          subtitle={selected.id}
          title="Run 状态"
        >
          <FlowInspectorSection title="运行信息">
            <dl className="enterprise-facts flow-inspector-facts">
              <div>
                <dt>Thread</dt>
                <dd>{selected.threadId}</dd>
              </div>
              <div>
                <dt>Supersteps</dt>
                <dd>{selected.superstep}</dd>
              </div>
              <div>
                <dt>Node executions</dt>
                <dd>
                  {selected.nodeExecutions}/{selected.budget.maxNodeExecutions}
                </dd>
              </div>
              <div>
                <dt>Tool calls</dt>
                <dd>
                  {selected.toolCalls}/{selected.budget.maxToolCalls}
                </dd>
              </div>
              <div>
                <dt>Updated</dt>
                <dd>{formatTime(selected.updatedAt)}</dd>
              </div>
            </dl>
          </FlowInspectorSection>
          {canCancel ? (
            <FlowInspectorSection title="运行控制">
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
                {busy === "cancel" ? "取消中…" : "取消运行"}
              </Button>
            </FlowInspectorSection>
          ) : null}
          {error ? (
            <p className="enterprise-page__message is-error" role="alert">
              {error}
            </p>
          ) : null}
        </FlowInspectorPanel>
      </FlowInspectorPortal>
      <div className="enterprise-page enterprise-runs enterprise-run-core">
        <RunTrace run={selected} />
      </div>
    </>
  );
}

function RunTrace({ run }: { run: FlowRun }) {
  return (
    <article
      className="enterprise-run-detail"
      aria-label={`Run ${run.id} 详情`}
    >
      <header>
        <span>
          <Activity aria-hidden="true" size={18} />
          <strong>
            {run.flowId}@{run.flowVersion}
          </strong>
        </span>
        <Badge variant={runVariant(run.status)}>
          {run.status.replaceAll("_", " ")}
        </Badge>
      </header>
      {run.error ? (
        <p className="enterprise-page__message is-error" role="alert">
          <ShieldAlert aria-hidden="true" size={15} />
          {run.error}
        </p>
      ) : null}
      <h3>Checkpoint timeline / 检查点</h3>
      <ol className="enterprise-trace">
        {run.checkpointHistory.toReversed().map((checkpoint) => (
          <li key={checkpoint.id}>
            <Clock3 aria-hidden="true" size={15} />
            <span>
              <strong>Superstep {checkpoint.superstep}</strong>
              <small>
                {checkpoint.nodeIds.join(", ") || "no nodes"} ·{" "}
                {checkpoint.pendingWriteCount} writes
              </small>
            </span>
            <Badge
              variant={
                checkpoint.status === "committed"
                  ? "success"
                  : checkpoint.status === "failed"
                    ? "danger"
                    : "neutral"
              }
            >
              {checkpoint.status}
            </Badge>
          </li>
        ))}
      </ol>
      <h3>Node trace / 节点轨迹</h3>
      <ol className="enterprise-trace">
        {run.nodeRuns.toReversed().map((node) => (
          <li key={node.id}>
            <Activity aria-hidden="true" size={15} />
            <span>
              <strong>
                {node.nodeId} · attempt {node.attempt}
              </strong>
              <small>
                {node.toolCalls} tool calls ·{" "}
                {node.completedAt ? formatTime(node.completedAt) : "running"}
              </small>
            </span>
            <Badge variant={runVariant(node.status)}>{node.status}</Badge>
          </li>
        ))}
      </ol>
    </article>
  );
}

function runVariant(
  status: string,
): "success" | "danger" | "warning" | "info" | "neutral" {
  if (status === "succeeded" || status === "committed") return "success";
  if (status === "failed" || status === "cancelled") return "danger";
  if (status.includes("waiting") || status === "paused") return "warning";
  if (["queued", "running", "resuming"].includes(status)) return "info";
  return "neutral";
}

function formatTime(value: string): string {
  return new Date(value).toLocaleString();
}
