import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  CheckCircle2,
  CircleAlert,
  Clock3,
  Inbox,
  Loader2,
  RefreshCw,
  RotateCcw,
  ShieldCheck,
  Square,
  XCircle,
} from "lucide-react";
import type { ApiClient } from "../api/client";
import {
  humanTaskActionPresentation,
  humanTaskStatusLabel,
  humanTaskTypeLabel,
  orderedHumanTaskActions,
  reconcileHumanTaskSelection,
  sortPendingHumanTasks,
} from "../humanTasks";
import type {
  HumanTask,
  HumanTaskAction,
  HumanTaskResolutionResult,
  HumanTaskType,
} from "../types";
import { Badge, Button, IconButton, Panel, Select } from "./ui";
import "./HumanTaskInboxPanel.css";

const defaultPollIntervalMs = 2_500;

const taskKindOptions: ReadonlyArray<{
  value: "" | HumanTaskType;
  label: string;
}> = [
  { value: "", label: "全部类型" },
  { value: "approval", label: "等待审批" },
  { value: "input_request", label: "需要输入" },
  { value: "output_review", label: "结果审阅" },
  { value: "recovery", label: "故障恢复" },
  { value: "reconnect", label: "重新连接" },
  { value: "data_correction", label: "数据修正" },
  { value: "manual", label: "人工处理" },
];

export type HumanTaskInboxPanelProps = {
  client: ApiClient | null;
  className?: string;
  flowRunId?: string | null;
  initialKind?: HumanTaskType | null;
  initialTaskId?: string | null;
  pollIntervalMs?: number;
  threadId?: string | null;
  onResolved?(result: HumanTaskResolutionResult): void;
  onSelectedTaskChange?(taskId: string | null): void;
};

type RefreshMode = "initial" | "manual" | "silent";

export function HumanTaskInboxPanel({
  client,
  className,
  flowRunId = null,
  initialKind = null,
  initialTaskId = null,
  pollIntervalMs = defaultPollIntervalMs,
  threadId = null,
  onResolved,
  onSelectedTaskChange,
}: HumanTaskInboxPanelProps) {
  const detailId = useId();
  const [tasks, setTasks] = useState<HumanTask[]>([]);
  const [selectedTaskId, setSelectedTaskId] = useState<string | null>(
    initialTaskId,
  );
  const [kind, setKind] = useState<"" | HumanTaskType>(initialKind ?? "");
  const [note, setNote] = useState("");
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [busyAction, setBusyAction] = useState<HumanTaskAction | null>(null);
  const [error, setError] = useState<string | null>(null);
  const hiddenTaskIdsRef = useRef(new Set<string>());

  const selectedTask = useMemo(
    () => tasks.find((task) => task.id === selectedTaskId) ?? null,
    [selectedTaskId, tasks],
  );

  const refresh = useCallback(
    async (mode: RefreshMode, signal?: AbortSignal) => {
      if (!client) {
        setTasks([]);
        setSelectedTaskId(null);
        setLoading(false);
        setRefreshing(false);
        setError("OpenTopia 服务尚未连接，无法加载人工任务。");
        return;
      }

      if (mode === "initial") setLoading(true);
      if (mode === "manual") setRefreshing(true);
      if (mode !== "silent") setError(null);

      try {
        const response = await client.listHumanTasks(
          {
            status: "pending",
            kind: kind || undefined,
            threadId: threadId || undefined,
            flowRunId: flowRunId || undefined,
          },
          signal,
        );
        const nextTasks = sortPendingHumanTasks(response).filter(
          (task) => !hiddenTaskIdsRef.current.has(task.id),
        );
        setTasks(nextTasks);
        setSelectedTaskId((current) =>
          reconcileHumanTaskSelection(nextTasks, current, initialTaskId),
        );
        setError(null);
      } catch (cause) {
        if (isAbortError(cause)) return;
        setError(readableError(cause));
      } finally {
        if (!signal?.aborted) {
          setLoading(false);
          setRefreshing(false);
        }
      }
    },
    [client, flowRunId, initialTaskId, kind, threadId],
  );

  useEffect(() => {
    hiddenTaskIdsRef.current.clear();
    if (!client) {
      void refresh("initial");
      return;
    }

    const controller = new AbortController();
    let timer: number | null = null;
    let stopped = false;

    const poll = async (mode: RefreshMode) => {
      await refresh(mode, controller.signal);
      if (stopped || controller.signal.aborted) return;
      timer = window.setTimeout(
        () => void poll("silent"),
        Math.max(pollIntervalMs, defaultPollIntervalMs),
      );
    };

    void poll("initial");
    return () => {
      stopped = true;
      controller.abort();
      if (timer !== null) window.clearTimeout(timer);
    };
  }, [client, pollIntervalMs, refresh]);

  useEffect(() => {
    if (
      initialTaskId &&
      tasks.some((task) => task.id === initialTaskId) &&
      selectedTaskId !== initialTaskId
    ) {
      setSelectedTaskId(initialTaskId);
    }
  }, [initialTaskId, selectedTaskId, tasks]);

  useEffect(() => {
    setNote("");
  }, [selectedTaskId]);

  useEffect(() => {
    onSelectedTaskChange?.(selectedTaskId);
  }, [onSelectedTaskChange, selectedTaskId]);

  async function resolveTask(action: HumanTaskAction) {
    if (!client || !selectedTask || busyAction) return;
    const taskId = selectedTask.id;
    setBusyAction(action);
    setError(null);
    try {
      const result = await client.resolveHumanTask(taskId, {
        expectedRevision: selectedTask.revision,
        action,
        note: note.trim() || undefined,
      });
      hiddenTaskIdsRef.current.add(taskId);
      const remaining = tasks.filter((task) => task.id !== taskId);
      setTasks(remaining);
      setSelectedTaskId((current) =>
        reconcileHumanTaskSelection(
          remaining,
          current === taskId ? null : current,
          initialTaskId === taskId ? null : initialTaskId,
        ),
      );
      setNote("");
      onResolved?.(result);
      await refresh("silent");
    } catch (cause) {
      hiddenTaskIdsRef.current.delete(taskId);
      setError(readableError(cause));
    } finally {
      setBusyAction(null);
    }
  }

  const classes = ["human-task-inbox", className ?? ""]
    .filter(Boolean)
    .join(" ");

  return (
    <section className={classes} aria-label="HumanTask Inbox 人工任务收件箱">
      <header className="human-task-inbox__header">
        <span className="human-task-inbox__heading-icon" aria-hidden="true">
          <Inbox size={18} />
        </span>
        <span className="human-task-inbox__heading">
          <small>Inbox / 人工任务</small>
          <strong>等待人工处理的 Flow</strong>
        </span>
        <Badge variant={tasks.length > 0 ? "warning" : "neutral"}>
          {tasks.length} 待处理
        </Badge>
        <Select<"" | HumanTaskType>
          className="human-task-inbox__kind-filter"
          disabled={loading || busyAction !== null}
          label="按人工任务类型筛选"
          onChange={(value) => setKind(value)}
          options={taskKindOptions}
          value={kind}
        />
        <IconButton
          aria-label="刷新人工任务"
          disabled={loading || refreshing || busyAction !== null}
          onClick={() => void refresh("manual")}
          size="compact"
          title="刷新人工任务"
        >
          <RefreshCw
            aria-hidden="true"
            className={refreshing ? "is-spinning" : undefined}
            size={14}
          />
        </IconButton>
      </header>

      {error ? (
        <div className="human-task-inbox__error" role="alert">
          <CircleAlert aria-hidden="true" size={16} />
          <span>{error}</span>
        </div>
      ) : null}

      <div className="human-task-inbox__workspace">
        <Panel
          actions={<Badge variant="neutral">{tasks.length}</Badge>}
          className="human-task-inbox__list-panel"
          title="待处理"
        >
          {loading ? (
            <LoadingState />
          ) : tasks.length === 0 ? (
            <EmptyState filtered={kind !== ""} />
          ) : (
            <div className="human-task-inbox__list" aria-label="待处理人工任务">
              {tasks.map((task) => {
                const selected = task.id === selectedTaskId;
                return (
                  <button
                    aria-controls={detailId}
                    aria-current={selected ? "true" : undefined}
                    className={selected ? "is-selected" : undefined}
                    data-task-id={task.id}
                    key={task.id}
                    onClick={() => setSelectedTaskId(task.id)}
                    type="button"
                  >
                    <span className="human-task-inbox__task-icon">
                      <TaskTypeIcon type={task.taskType} />
                    </span>
                    <span className="human-task-inbox__task-copy">
                      <strong>{task.title}</strong>
                      <small>{task.description}</small>
                      <span>
                        <Clock3 aria-hidden="true" size={12} />
                        {formatDateTime(task.createdAt)}
                      </span>
                    </span>
                    <Badge variant="warning">
                      {humanTaskTypeLabel(task.taskType)}
                    </Badge>
                  </button>
                );
              })}
            </div>
          )}
        </Panel>

        <Panel
          actions={
            selectedTask ? (
              <Badge variant="warning">
                {humanTaskStatusLabel(selectedTask.status)}
              </Badge>
            ) : undefined
          }
          className="human-task-inbox__detail-panel"
          id={detailId}
          title="任务详情"
        >
          {selectedTask ? (
            <HumanTaskDetail
              busyAction={busyAction}
              note={note}
              onNoteChange={setNote}
              onResolve={(action) => void resolveTask(action)}
              task={selectedTask}
            />
          ) : (
            <div className="human-task-inbox__detail-empty">
              <Inbox aria-hidden="true" size={20} />
              <span>从左侧选择一个待处理任务查看上下文。</span>
            </div>
          )}
        </Panel>
      </div>
    </section>
  );
}

function HumanTaskDetail({
  busyAction,
  note,
  onNoteChange,
  onResolve,
  task,
}: {
  busyAction: HumanTaskAction | null;
  note: string;
  onNoteChange(value: string): void;
  onResolve(action: HumanTaskAction): void;
  task: HumanTask;
}) {
  const actions = orderedHumanTaskActions(task);
  return (
    <article
      className="human-task-detail"
      data-task-id={task.id}
      aria-live="polite"
    >
      <header>
        <span className="human-task-inbox__task-icon is-detail">
          <TaskTypeIcon type={task.taskType} />
        </span>
        <span>
          <small>{humanTaskTypeLabel(task.taskType)}</small>
          <h3>{task.title}</h3>
          <p>{task.description}</p>
        </span>
      </header>

      <dl className="human-task-detail__facts">
        <div>
          <dt>Flow Run</dt>
          <dd>
            <code>{task.sourceId}</code>
          </dd>
        </div>
        {task.sourceNodeId ? (
          <div>
            <dt>节点</dt>
            <dd>
              <code>{task.sourceNodeId}</code>
            </dd>
          </div>
        ) : null}
        <div>
          <dt>任务 ID</dt>
          <dd>
            <code>{task.id}</code>
          </dd>
        </div>
        <div>
          <dt>更新时间</dt>
          <dd>{formatDateTime(task.updatedAt)}</dd>
        </div>
      </dl>

      {task.taskType === "recovery" ? (
        <div className="human-task-detail__warning">
          <CircleAlert aria-hidden="true" size={16} />
          <span>
            重试前请确认外部系统是否已经产生副作用；继续后会创建新的节点尝试。
          </span>
        </div>
      ) : null}

      {task.payload !== null && task.payload !== undefined ? (
        <details className="human-task-detail__payload">
          <summary>查看任务上下文</summary>
          <pre>{formatPayload(task.payload)}</pre>
        </details>
      ) : null}

      <label className="human-task-detail__note">
        <span>处理说明（可选）</span>
        <textarea
          disabled={busyAction !== null}
          onChange={(event) => onNoteChange(event.target.value)}
          placeholder="记录审批依据、检查结果或拒绝原因"
          value={note}
        />
      </label>

      {actions.length > 0 ? (
        <footer className="human-task-detail__actions">
          {actions.map((action) => {
            const presentation = humanTaskActionPresentation(action);
            return (
              <Button
                disabled={busyAction !== null}
                key={action}
                onClick={() => onResolve(action)}
                variant={presentation.variant}
              >
                {busyAction === action ? (
                  <Loader2
                    aria-hidden="true"
                    className="is-spinning"
                    size={14}
                  />
                ) : (
                  <TaskActionIcon action={action} />
                )}
                {busyAction === action
                  ? presentation.pendingLabel
                  : presentation.label}
              </Button>
            );
          })}
        </footer>
      ) : (
        <p className="human-task-detail__no-actions">
          该任务目前没有可用操作，请联系 Flow 管理员。
        </p>
      )}
    </article>
  );
}

function LoadingState() {
  return (
    <div className="human-task-inbox__loading" role="status">
      <Loader2 aria-hidden="true" className="is-spinning" size={18} />
      <span>正在加载人工任务…</span>
    </div>
  );
}

function EmptyState({ filtered }: { filtered: boolean }) {
  return (
    <div className="human-task-inbox__empty">
      <CheckCircle2 aria-hidden="true" size={20} />
      <strong>
        {filtered ? "没有符合筛选条件的任务" : "所有任务均已处理"}
      </strong>
      <span>
        {filtered
          ? "选择其他任务类型查看待处理项。"
          : "新的中断或审批会自动出现在这里。"}
      </span>
    </div>
  );
}

function TaskTypeIcon({ type }: { type: HumanTaskType }) {
  if (type === "approval" || type === "output_review") {
    return <ShieldCheck aria-hidden="true" size={16} />;
  }
  if (
    type === "recovery" ||
    type === "reconnect" ||
    type === "data_correction"
  ) {
    return <RotateCcw aria-hidden="true" size={16} />;
  }
  return <CircleAlert aria-hidden="true" size={16} />;
}

function TaskActionIcon({ action }: { action: HumanTaskAction }) {
  if (action === "approve") {
    return <CheckCircle2 aria-hidden="true" size={14} />;
  }
  if (action === "reject") {
    return <XCircle aria-hidden="true" size={14} />;
  }
  if (action === "retry") {
    return <RotateCcw aria-hidden="true" size={14} />;
  }
  return <Square aria-hidden="true" size={14} />;
}

function formatDateTime(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

function formatPayload(payload: unknown): string {
  if (typeof payload === "string") return payload;
  try {
    return JSON.stringify(payload, null, 2) ?? String(payload);
  } catch {
    return String(payload);
  }
}

function isAbortError(cause: unknown): boolean {
  return cause instanceof DOMException && cause.name === "AbortError";
}

function readableError(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}
