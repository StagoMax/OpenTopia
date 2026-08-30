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

export function RunsPage({ client }: { client: ApiClient }) {
  const { snapshot, store } = useEnterpriseStore(client);
  const selection = useFlowWorkspaceSelection();
  const selected =
    snapshot.runs.find((run) => run.id === selection?.selectedRunId) ??
    snapshot.runs[0] ??
    null;
  const selectedFlow = selected
    ? snapshot.flows.find((flow) => flow.flowId === selected.flowId)
    : null;
  const flowName = selectedFlow?.name ?? selected?.flowId ?? "Workflow Run";
  const [busy, setBusy] = useState<"pause" | "resume" | "cancel" | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (selected && selected.id !== selection?.selectedRunId) {
      selection?.setSelectedRunId(selected.id);
    }
  }, [selected, selection]);

  useFlowWorkspaceTitle(
    selected ? `${flowName} · 运行记录` : "Runs / 运行追踪",
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
  const canCancel = ![
    "succeeded",
    "failed",
    "cancel_requested",
    "cancelled",
  ].includes(selected.status);
  const status = runStatusPresentation(selected.status);
  const endedAt = selected.completedAt ?? selected.updatedAt;

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
          status={status.label}
          statusVariant={status.variant}
          subtitle={`${flowName} · v${selected.flowVersion}`}
          title="运行概览"
        >
          {snapshot.error || error ? (
            <p className="enterprise-page__message is-error" role="alert">
              {snapshot.error ?? error}
            </p>
          ) : null}
          <FlowInspectorSection title="时间">
            <dl className="enterprise-facts flow-inspector-facts">
              <div>
                <dt>开始</dt>
                <dd>{formatDateTime(selected.startedAt)}</dd>
              </div>
              <div>
                <dt>{selected.completedAt ? "完成" : "更新"}</dt>
                <dd>{formatDateTime(endedAt)}</dd>
              </div>
              <div>
                <dt>耗时</dt>
                <dd>{formatDuration(selected.startedAt, endedAt)}</dd>
              </div>
            </dl>
          </FlowInspectorSection>
          <FlowInspectorSection title="用量与预算">
            <dl className="enterprise-facts flow-inspector-facts">
              <div>
                <dt>节点执行</dt>
                <dd>
                  {selected.nodeExecutions}/{selected.budget.maxNodeExecutions}
                </dd>
              </div>
              <div>
                <dt>工具调用</dt>
                <dd>
                  {selected.toolCalls}/{selected.budget.maxToolCalls}
                </dd>
              </div>
              <div>
                <dt>检查点</dt>
                <dd>{selected.checkpointHistory.length}</dd>
              </div>
              <div>
                <dt>最长运行</dt>
                <dd>{selected.budget.maxDurationSeconds} 秒</dd>
              </div>
            </dl>
          </FlowInspectorSection>
          <FlowInspectorSection title="技术标识">
            <dl className="enterprise-facts flow-inspector-facts">
              <div>
                <dt>Run ID</dt>
                <dd>
                  <code>{selected.id}</code>
                </dd>
              </div>
              <div>
                <dt>Thread</dt>
                <dd>
                  <code>{selected.threadId}</code>
                </dd>
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
        </FlowInspectorPanel>
      </FlowInspectorPortal>
      <div className="enterprise-page enterprise-runs enterprise-core-detail">
        <RunDetails flowName={flowName} run={selected} />
      </div>
    </>
  );
}
