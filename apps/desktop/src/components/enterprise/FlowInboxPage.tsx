import { CheckCircle2, Clock3, Play, RefreshCw, UserCheck } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { ApiClient } from "../../api/client";
import {
  humanTaskActionPresentation,
  humanTaskInputRequest,
  humanTaskTypeLabel,
  orderedHumanTaskActions,
} from "../../humanTasks";
import type {
  FlowCase,
  HumanTask,
  HumanTaskAction,
  UserInputResponse,
} from "../../types";
import { PlanChoiceCard } from "../PlanChoiceCard";
import { Badge, Button, IconButton, TextField } from "../ui";
import { FlowInspectorPanel, FlowInspectorSection } from "./FlowInspectorPanel";
import {
  FlowInspectorPortal,
  useFlowWorkspaceSelection,
  useFlowWorkspaceTitle,
} from "./flowAgentSelection";
import { workflowTriggerLabel } from "./enterpriseSidebarPresentation";
import { useEnterpriseStore } from "./store";
import { StructuredPayload } from "./StructuredPayload";

type InboxItem =
  | { id: string; kind: "task"; task: HumanTask }
  | { id: string; kind: "case"; flowCase: FlowCase };

export function FlowInboxPage({ client }: { client: ApiClient }) {
  const { snapshot, store } = useEnterpriseStore(client);
  const workspace = useFlowWorkspaceSelection();
  const [busy, setBusy] = useState<string | null>(null);
  const [note, setNote] = useState("");
  const [response, setResponse] = useState("");
  const [error, setError] = useState<string | null>(null);

  const items = useMemo<InboxItem[]>(() => {
    const tasks = snapshot.tasks.map((task) => ({
      id: `task:${task.id}`,
      kind: "task" as const,
      task,
    }));
    const cases = snapshot.cases
      .filter((item) => item.status === "accepted" && !item.flowRunId)
      .map((flowCase) => ({
        id: `case:${flowCase.id}`,
        kind: "case" as const,
        flowCase,
      }));
    return [...tasks, ...cases].sort(
      (left, right) => inboxTime(left) - inboxTime(right),
    );
  }, [snapshot.cases, snapshot.tasks]);

  const selected =
    items.find((item) => item.id === workspace?.selectedInboxItemId) ??
    items[0] ??
    null;

  useEffect(() => {
    if (selected && selected.id !== workspace?.selectedInboxItemId) {
      workspace?.setSelectedInboxItemId(selected.id);
    }
  }, [selected, workspace]);

  useEffect(() => {
    setNote("");
    setResponse("");
    setError(null);
  }, [selected?.id]);

  const selectedFlow =
    selected?.kind === "case"
      ? (snapshot.flows.find(
          (flow) => flow.flowId === selected.flowCase.flowId,
        ) ?? null)
      : null;
  const title =
    selected?.kind === "task"
      ? selected.task.title
      : selected?.kind === "case"
        ? (selectedFlow?.name ?? selected.flowCase.flowId)
        : "Inbox / 待处理";
  useFlowWorkspaceTitle(title);

  async function runAction(
    action: HumanTaskAction,
    explicitResponse?: unknown,
  ) {
    if (selected?.kind !== "task" || busy) return;
    setBusy(action);
    setError(null);
    try {
      const parsedResponse =
        explicitResponse ??
        (response.trim() ? (JSON.parse(response) as unknown) : undefined);
      await client.resolveHumanTask(selected.task.id, {
        expectedRevision: selected.task.revision,
        action,
        note: note.trim() || undefined,
        idempotencyKey: crypto.randomUUID(),
        response: parsedResponse,
      });
      await store.load(true);
    } catch (cause) {
      setError(readableError(cause));
    } finally {
      setBusy(null);
    }
  }

  async function startCase() {
    if (selected?.kind !== "case" || busy) return;
    setBusy("start");
    setError(null);
    try {
      await client.startPendingFlowCase(selected.flowCase.id);
      await store.load(true);
    } catch (cause) {
      setError(readableError(cause));
    } finally {
      setBusy(null);
    }
  }

  async function claimTask() {
    if (selected?.kind !== "task" || busy) return;
    setBusy("claim");
    setError(null);
    try {
      await client.claimHumanTask(selected.task.id, selected.task.revision);
      await store.load(true);
    } catch (cause) {
      setError(readableError(cause));
    } finally {
      setBusy(null);
    }
  }

  if (!selected) {
    return (
      <div className="enterprise-page enterprise-page--empty">
        <CheckCircle2 aria-hidden="true" size={28} />
        <strong>Inbox 已清空</strong>
        <span>当前没有待确认事件或人工任务。</span>
        <FlowInspectorPortal>
          <FlowInspectorPanel
            actions={
              <IconButton
                aria-label="刷新 Inbox"
                onClick={() => void store.load(true)}
                size="compact"
              >
                <RefreshCw aria-hidden="true" size={14} />
              </IconButton>
            }
            status="clear"
            statusVariant="success"
            title="Inbox"
          >
            <p className="enterprise-page__lede">没有需要人工处理的项目。</p>
          </FlowInspectorPanel>
        </FlowInspectorPortal>
      </div>
    );
  }

  const actions =
    selected.kind === "task" ? orderedHumanTaskActions(selected.task) : [];
  const inputRequest =
    selected.kind === "task" ? humanTaskInputRequest(selected.task) : null;
  const requiresObservation =
    selected.kind === "task" &&
    ["reconciliation", "reconnect"].includes(selected.task.taskType);
  const payload =
    selected.kind === "task" ? selected.task.payload : selected.flowCase.input;
  const payloadSchema =
    selected.kind === "case"
      ? selected.flowCase.flowRevision.compiledWorkflow.inputSchema
      : undefined;

  return (
    <div className="enterprise-page enterprise-inbox-page enterprise-core-detail">
      <section className="enterprise-core-detail__summary">
        <span className="enterprise-core-detail__icon" aria-hidden="true">
          {selected.kind === "task" ? (
            <UserCheck size={20} />
          ) : (
            <Clock3 size={20} />
          )}
        </span>
        <div>
          <small>
            {selected.kind === "task"
              ? humanTaskTypeLabel(selected.task.taskType)
              : "待确认 Flow 事件"}
          </small>
          <h2>{title}</h2>
          <p>
            {selected.kind === "task"
              ? selected.task.description
              : "事件已通过认证和幂等检查；批准后将使用接收时冻结的 Flow Revision 启动运行。"}
          </p>
        </div>
      </section>
      <section className="enterprise-core-detail__payload">
        <header>
          <h3>需要确认的信息</h3>
          <Badge variant="neutral">
            {selected.kind === "task"
              ? humanTaskSourceLabel(selected.task.sourceKind)
              : workflowTriggerLabel(selected.flowCase.flowRevision.trigger)}
          </Badge>
        </header>
        <StructuredPayload
          emptyLabel="此事项没有附加信息。"
          schema={payloadSchema}
          value={payload}
        />
      </section>

      <FlowInspectorPortal>
        <FlowInspectorPanel
          actions={
            selected.kind === "case" ? (
              <Button
                disabled={Boolean(busy)}
                onClick={() => void startCase()}
                size="compact"
                variant="primary"
              >
                <Play aria-hidden="true" size={14} />
                {busy === "start" ? "启动中…" : "批准并运行"}
              </Button>
            ) : inputRequest ? null : (
              actions.map((action) => {
                const presentation = humanTaskActionPresentation(action);
                return (
                  <Button
                    disabled={
                      Boolean(busy) ||
                      (requiresObservation &&
                        ["resume", "reconnect", "acknowledge"].includes(
                          action,
                        ) &&
                        !note.trim())
                    }
                    key={action}
                    onClick={() => void runAction(action)}
                    size="compact"
                    variant={presentation.variant}
                  >
                    {busy === action
                      ? presentation.pendingLabel
                      : presentation.label}
                  </Button>
                );
              })
            )
          }
          status="待处理"
          statusVariant="warning"
          subtitle={
            selected.kind === "task"
              ? humanTaskTypeLabel(selected.task.taskType)
              : selectedFlow?.name
          }
          title="处理事项"
        >
          {snapshot.error || error ? (
            <p className="enterprise-page__message is-error" role="alert">
              {snapshot.error ?? error}
            </p>
          ) : null}
          <FlowInspectorSection title="处理信息">
            {selected.kind === "task" ? (
              <>
                <dl className="flow-inspector-facts">
                  <div>
                    <dt>负责人</dt>
                    <dd>{selected.task.assignedTo ?? "尚未分配"}</dd>
                  </div>
                  <div>
                    <dt>领取状态</dt>
                    <dd>
                      {selected.task.claimedBy
                        ? `${selected.task.claimedBy} 已领取`
                        : "待领取"}
                    </dd>
                  </div>
                  <div>
                    <dt>{selected.task.dueAt ? "截止时间" : "收到时间"}</dt>
                    <dd>
                      {formatTime(
                        selected.task.dueAt ?? selected.task.createdAt,
                      )}
                    </dd>
                  </div>
                </dl>
                {!selected.task.claimedBy ? (
                  <Button
                    disabled={Boolean(busy)}
                    onClick={() => void claimTask()}
                    size="compact"
                    variant="secondary"
                  >
                    {busy === "claim" ? "领取中…" : "领取任务"}
                  </Button>
                ) : null}
              </>
            ) : (
              <dl className="flow-inspector-facts">
                <div>
                  <dt>Flow</dt>
                  <dd>{selectedFlow?.name ?? selected.flowCase.flowId}</dd>
                </div>
                <div>
                  <dt>触发方式</dt>
                  <dd>
                    {workflowTriggerLabel(
                      selected.flowCase.flowRevision.trigger,
                    )}
                  </dd>
                </div>
                <div>
                  <dt>收到时间</dt>
                  <dd>{formatTime(selected.flowCase.createdAt)}</dd>
                </div>
              </dl>
            )}
          </FlowInspectorSection>
          {selected.kind === "task" ? (
            <FlowInspectorSection title="处理说明">
              {inputRequest ? (
                <PlanChoiceCard
                  error={null}
                  isSubmitting={Boolean(busy)}
                  onCancel={() => void runAction("cancel")}
                  onSkip={() =>
                    void runAction("submit", {
                      answers: [],
                      skipped: true,
                    } satisfies UserInputResponse)
                  }
                  onSubmit={(value) => void runAction("submit", value)}
                  request={inputRequest}
                />
              ) : (
                <TextField
                  hint={
                    requiresObservation
                      ? "请记录外部系统中的真实状态或已完成的人工动作"
                      : undefined
                  }
                  label={
                    requiresObservation
                      ? "Observation / 核对结果"
                      : "Note / 备注"
                  }
                  onChange={(event) => setNote(event.currentTarget.value)}
                  value={note}
                />
              )}
              {!inputRequest && selected.task.actionSchema ? (
                <details className="enterprise-technical-details">
                  <summary>高级响应（JSON）</summary>
                  <label className="flow-workspace-inspector__textarea">
                    <span>结构化响应</span>
                    <textarea
                      onChange={(event) =>
                        setResponse(event.currentTarget.value)
                      }
                      placeholder="{}"
                      rows={6}
                      value={response}
                    />
                  </label>
                </details>
              ) : null}
            </FlowInspectorSection>
          ) : null}
          <FlowInspectorSection title="技术信息">
            <details className="enterprise-technical-details">
              <summary>查看技术标识</summary>
              <dl className="flow-inspector-facts">
                <div>
                  <dt>事项 ID</dt>
                  <dd>
                    <code>{selected.id.replace(/^(task|case):/, "")}</code>
                  </dd>
                </div>
                {selected.kind === "case" ? (
                  <>
                    <div>
                      <dt>Flow Revision ID</dt>
                      <dd>
                        <code>{selected.flowCase.flowRevisionId}</code>
                      </dd>
                    </div>
                    <div>
                      <dt>触发器 ID</dt>
                      <dd>
                        <code>{selected.flowCase.triggerId}</code>
                      </dd>
                    </div>
                  </>
                ) : null}
              </dl>
            </details>
          </FlowInspectorSection>
        </FlowInspectorPanel>
      </FlowInspectorPortal>
    </div>
  );
}

function readableError(error: unknown): string {
  if (error instanceof SyntaxError) return "Response 必须是有效 JSON。";
  return error instanceof Error ? error.message : String(error);
}

function formatTime(value: string): string {
  return new Date(value).toLocaleString();
}

function humanTaskSourceLabel(source: HumanTask["sourceKind"]): string {
  return source === "flow_run" ? "流程运行" : "外部交付";
}

function inboxTime(item: InboxItem): number {
  const value =
    item.kind === "task"
      ? (item.task.dueAt ?? item.task.createdAt)
      : item.flowCase.createdAt;
  const timestamp = new Date(value).getTime();
  return Number.isFinite(timestamp) ? timestamp : Number.MAX_SAFE_INTEGER;
}
