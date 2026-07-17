import { useEffect, useMemo, useState, type ReactNode } from "react";
import {
  Activity,
  AlertCircle,
  Bot,
  BrainCircuit,
  Check,
  ChevronDown,
  ChevronRight,
  Clock3,
  FileText,
  Globe2,
  Loader2,
  Table2,
  TerminalSquare,
  Wrench,
  X,
} from "lucide-react";
import type {
  AgentEvent,
  SubagentRun,
  TaskPlan,
  ToolCall,
  ToolResult,
} from "../types";
import "./TurnActivityTimeline.css";

type ToolCategory =
  "command" | "file" | "browser" | "spreadsheet" | "agent" | "tool";

type ToolExecution = {
  call: ToolCall;
  startedAt: string;
  result?: ToolResult;
  finishedAt?: string;
};

type PlanStepTiming = {
  startedAt?: string;
  finishedAt?: string;
};

type FileChangeSummary = {
  path: string;
  operation: string;
  additions?: number;
  deletions?: number;
  detail?: string;
};

type ActivityFile = {
  path: string;
  summary: string;
  createdAt: string;
};

type PrimitiveActivity =
  | { kind: "tool"; seq: number; execution: ToolExecution }
  | {
      kind: "plan";
      seq: number;
      plan: TaskPlan;
      startedAt: string;
      finishedAt?: string;
      stepTimings: PlanStepTiming[];
    }
  | {
      kind: "file";
      seq: number;
      path: string;
      summary: string;
      createdAt: string;
    }
  | {
      kind: "reasoning";
      seq: number;
      text: string;
      isDelta: boolean;
      createdAt: string;
    }
  | { kind: "subagent"; seq: number; run: SubagentRun; createdAt: string }
  | {
      kind: "approval";
      seq: number;
      reason: string;
      action: string;
      createdAt: string;
    }
  | { kind: "context"; seq: number; createdAt: string }
  | { kind: "error"; seq: number; message: string; createdAt: string }
  | { kind: "cancelled"; seq: number; reason: string; createdAt: string }
  | { kind: "suspended"; seq: number; reason: string; createdAt: string };

type ActivityEntry =
  | {
      kind: "tool-group";
      id: string;
      category: ToolCategory;
      executions: ToolExecution[];
    }
  | {
      kind: "file-group";
      id: string;
      files: ActivityFile[];
    }
  | Exclude<PrimitiveActivity, { kind: "tool" } | { kind: "file" }>;

type ActivityState = "running" | "complete" | "waiting" | "cancelled" | "error";

export function TurnActivityTimeline({
  events,
  isActive,
  formatError = (message) => message,
}: {
  events: AgentEvent[];
  isActive: boolean;
  formatError?(message: string): string;
}) {
  const entries = useMemo(() => buildActivityEntries(events), [events]);
  const state = activityState(events, isActive);
  const [expanded, setExpanded] = useState(isActive);
  const mountedAt = useMemo(() => Date.now(), []);
  const hasRunningEntry = entries.some(activityEntryIsRunning);
  const now = useTimelineClock(
    isActive || (state === "running" && hasRunningEntry),
  );
  const turnTiming = formatTurnTiming(events, isActive, now, mountedAt);

  useEffect(() => {
    if (isActive || state === "error" || state === "waiting") {
      setExpanded(true);
    }
  }, [isActive, state]);

  const tools = entries.flatMap((entry) =>
    entry.kind === "tool-group" ? entry.executions : [],
  );
  const commandCount = tools.filter(
    (execution) => toolCategory(execution.call.name) === "command",
  ).length;
  const otherToolCount = tools.length - commandCount;
  const fileCount = entries.reduce(
    (count, entry) =>
      count + (entry.kind === "file-group" ? entry.files.length : 0),
    0,
  );
  const operationCount =
    tools.length +
    fileCount +
    entries.filter((entry) => entry.kind === "subagent").length;

  if (!isActive && entries.length === 0) return null;

  const terminalSummary = [...events]
    .reverse()
    .find((event) => event.payload.type === "turn_finished");
  const summary =
    terminalSummary?.payload.type === "turn_finished"
      ? terminalSummary.payload.summary.trim()
      : "";

  return (
    <section className="turn-activity" data-state={state}>
      <button
        className="turn-activity-header"
        type="button"
        aria-expanded={expanded}
        onClick={() => setExpanded((current) => !current)}
      >
        <span className="turn-activity-status-icon" aria-hidden="true">
          <ActivityStateIcon state={state} />
        </span>
        <span className="turn-activity-heading">
          <strong>{isActive ? "正在处理" : "处理过程"}</strong>
          <small>
            {activitySummary({
              operationCount,
              commandCount,
              otherToolCount,
              isActive,
            })}
            {turnTiming ? ` · ${turnTiming}` : ""}
          </small>
        </span>
        <span className="turn-activity-chevron" aria-hidden="true">
          {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
        </span>
      </button>

      {expanded && (
        <div className="turn-activity-body">
          {entries.length === 0 && isActive && (
            <div className="turn-activity-pending" role="status">
              <Loader2 size={13} className="spin" />
              <span>正在连接模型并准备执行步骤</span>
            </div>
          )}
          {entries.map((entry) => (
            <ActivityEntryView
              key={activityEntryKey(entry)}
              entry={entry}
              isActive={isActive}
              now={now}
              formatError={formatError}
            />
          ))}
          {summary && <p className="turn-activity-footer">{summary}</p>}
        </div>
      )}
    </section>
  );
}

function ActivityEntryView({
  entry,
  isActive,
  now,
  formatError,
}: {
  entry: ActivityEntry;
  isActive: boolean;
  now: number;
  formatError(message: string): string;
}) {
  if (entry.kind === "tool-group") {
    return (
      <ToolActivityGroup
        category={entry.category}
        executions={entry.executions}
        defaultExpanded={isActive}
        now={now}
      />
    );
  }
  if (entry.kind === "file-group") {
    return <FileActivityGroup files={entry.files} defaultExpanded={isActive} />;
  }
  if (entry.kind === "reasoning") {
    return <ReasoningActivity text={entry.text} />;
  }
  if (entry.kind === "plan") {
    return (
      <PlanActivity
        plan={entry.plan}
        stepTimings={entry.stepTimings}
        startedAt={entry.startedAt}
        finishedAt={entry.finishedAt}
        defaultExpanded={isActive}
        isActive={isActive}
        now={now}
      />
    );
  }
  if (entry.kind === "subagent") {
    return <SubagentActivity run={entry.run} now={now} />;
  }
  if (entry.kind === "approval") {
    return (
      <ActivityNotice
        icon={<Clock3 size={13} />}
        tone="waiting"
        title="等待用户批准"
        detail={`${entry.reason}${entry.action ? `\n操作：${entry.action}` : ""}`}
      />
    );
  }
  if (entry.kind === "context") {
    return (
      <ActivityNotice
        icon={<Activity size={13} />}
        title="已压缩对话上下文"
        detail="系统已生成上下文摘要，以便继续执行长程任务。"
      />
    );
  }
  if (entry.kind === "cancelled") {
    return (
      <ActivityNotice
        icon={<X size={13} />}
        tone="error"
        title="任务已取消"
        detail={entry.reason}
      />
    );
  }
  if (entry.kind === "suspended") {
    return (
      <ActivityNotice
        icon={<Clock3 size={13} />}
        tone="waiting"
        title="任务已暂停"
        detail={entry.reason}
      />
    );
  }
  return (
    <ActivityNotice
      icon={<AlertCircle size={13} />}
      tone="error"
      title="执行失败"
      detail={formatError(entry.message)}
    />
  );
}

function ToolActivityGroup({
  category,
  executions,
  defaultExpanded,
  now,
}: {
  category: ToolCategory;
  executions: ToolExecution[];
  defaultExpanded: boolean;
  now: number;
}) {
  const running = executions.some((execution) => !execution.result);
  const failed = executions.some((execution) =>
    toolResultFailed(execution.result),
  );
  const timing = formatExecutionGroupTiming(executions, running, now);
  const [expanded, setExpanded] = useState(defaultExpanded || running);

  useEffect(() => {
    if (running) setExpanded(true);
  }, [running]);

  return (
    <div
      className="activity-group"
      data-state={failed ? "error" : running ? "running" : "complete"}
    >
      <button
        className="activity-group-header"
        type="button"
        aria-expanded={expanded}
        onClick={() => setExpanded((current) => !current)}
      >
        <span className="activity-group-icon" aria-hidden="true">
          {toolCategoryIcon(category)}
        </span>
        <span>
          {toolGroupTitle(category, executions.length)}
          {timing ? ` · ${timing}` : ""}
        </span>
        <ActivityResultIcon running={running} failed={failed} />
        {expanded ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
      </button>
      {expanded && (
        <div className="activity-group-content">
          {executions.map((execution) => (
            <ToolExecutionItem
              key={execution.call.id}
              execution={execution}
              now={now}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function ToolExecutionItem({
  execution,
  now,
}: {
  execution: ToolExecution;
  now: number;
}) {
  const running = !execution.result;
  const failed = toolResultFailed(execution.result);
  const [expanded, setExpanded] = useState(false);
  const timing = formatActivityTiming(
    execution.startedAt,
    execution.finishedAt,
    running,
    now,
  );
  const fileChanges = toolFileChangeSummaries(execution);
  const primaryFileChange =
    fileChanges.length === 1 ? fileChanges[0] : undefined;
  const input = formatToolInput(execution.call, fileChanges.length > 0);
  const output = formatToolOutput(execution.result);
  const title = primaryFileChange
    ? primaryFileChange.path
    : fileChanges.length > 1
      ? `修改了 ${fileChanges.length} 个文件`
      : toolExecutionTitle(execution.call);

  return (
    <div
      className="tool-execution"
      data-state={failed ? "error" : running ? "running" : "complete"}
    >
      <button
        className="tool-execution-header"
        type="button"
        aria-expanded={expanded}
        onClick={() => setExpanded((current) => !current)}
      >
        <span className="tool-execution-state" aria-hidden="true">
          <ActivityResultIcon running={running} failed={failed} />
        </span>
        <span className="tool-execution-title">
          <strong title={primaryFileChange?.path}>{title}</strong>
          <small className="tool-execution-meta">
            <span>
              {primaryFileChange?.operation ||
                toolDisplayName(execution.call.name)}
            </span>
            {primaryFileChange && (
              <FileChangeStatsView change={primaryFileChange} />
            )}
            {timing && <span>· {timing}</span>}
          </small>
        </span>
        {expanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
      </button>
      {fileChanges.length > 1 && (
        <FileChangeSummaryList changes={fileChanges} />
      )}
      {expanded && (
        <div className="tool-execution-details">
          <div>
            <span>{execution.call.name === "shell" ? "命令" : "参数"}</span>
            <pre>{input}</pre>
          </div>
          <div>
            <span>结果</span>
            <pre>{output}</pre>
          </div>
        </div>
      )}
    </div>
  );
}

function FileActivityGroup({
  files,
  defaultExpanded,
}: {
  files: ActivityFile[];
  defaultExpanded: boolean;
}) {
  const [expanded, setExpanded] = useState(defaultExpanded);
  const timing = formatFileGroupTiming(files);
  return (
    <div className="activity-group" data-state="complete">
      <button
        className="activity-group-header"
        type="button"
        aria-expanded={expanded}
        onClick={() => setExpanded((current) => !current)}
      >
        <span className="activity-group-icon" aria-hidden="true">
          <FileText size={13} />
        </span>
        <span>
          修改了 {files.length} 个文件{timing ? ` · ${timing}` : ""}
        </span>
        <ActivityResultIcon running={false} failed={false} />
        {expanded ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
      </button>
      {expanded && (
        <div className="activity-file-list">
          {files.map((file, index) => (
            <FileActivityItem key={`${file.path}-${index}`} file={file} />
          ))}
        </div>
      )}
    </div>
  );
}

function FileActivityItem({ file }: { file: ActivityFile }) {
  const [expanded, setExpanded] = useState(false);
  const detail = file.summary.trim();
  const change = fileChangedEventSummary(file);

  return (
    <div className="activity-file-item">
      <button
        type="button"
        className="activity-file-row"
        aria-expanded={detail ? expanded : undefined}
        onClick={() => detail && setExpanded((current) => !current)}
      >
        <FileText size={12} aria-hidden="true" />
        <span title={file.path}>{file.path}</span>
        <span className="activity-file-meta">
          <span>{change.operation}</span>
          <FileChangeStatsView change={change} />
        </span>
        {detail ? (
          expanded ? (
            <ChevronDown size={12} aria-hidden="true" />
          ) : (
            <ChevronRight size={12} aria-hidden="true" />
          )
        ) : (
          <span />
        )}
      </button>
      {expanded && detail && <p className="activity-file-detail">{detail}</p>}
    </div>
  );
}

function FileChangeSummaryList({ changes }: { changes: FileChangeSummary[] }) {
  return (
    <div className="file-change-summary-list" role="list" aria-label="文件变更">
      {changes.map((change, index) => (
        <div key={`${change.path}-${index}`} role="listitem">
          <FileText size={12} aria-hidden="true" />
          <span title={change.path}>{change.path}</span>
          <span className="activity-file-meta">
            <span>{change.operation}</span>
            <FileChangeStatsView change={change} />
          </span>
        </div>
      ))}
    </div>
  );
}

function FileChangeStatsView({ change }: { change: FileChangeSummary }) {
  if (change.additions === undefined && change.deletions === undefined) {
    return null;
  }
  return (
    <span
      className="file-change-stats"
      aria-label={fileChangeStatsLabel(change)}
    >
      {change.additions !== undefined && (
        <span className="file-change-additions">+{change.additions}</span>
      )}
      {change.deletions !== undefined && (
        <span className="file-change-deletions">-{change.deletions}</span>
      )}
    </span>
  );
}

function ReasoningActivity({ text }: { text: string }) {
  const [expanded, setExpanded] = useState(false);
  return (
    <div className="activity-group reasoning-activity" data-state="complete">
      <button
        className="activity-group-header"
        type="button"
        aria-expanded={expanded}
        onClick={() => setExpanded((current) => !current)}
      >
        <span className="activity-group-icon" aria-hidden="true">
          <BrainCircuit size={13} />
        </span>
        <span>思考过程</span>
        <span />
        {expanded ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
      </button>
      {expanded && (
        <pre className="reasoning-activity-detail">
          {truncateText(redactText(text), 12_000)}
        </pre>
      )}
    </div>
  );
}

function PlanActivity({
  plan,
  stepTimings,
  startedAt,
  finishedAt,
  defaultExpanded,
  isActive,
  now,
}: {
  plan: TaskPlan;
  stepTimings: PlanStepTiming[];
  startedAt: string;
  finishedAt?: string;
  defaultExpanded: boolean;
  isActive: boolean;
  now: number;
}) {
  const completed = plan.steps.filter(
    (step) => step.status === "completed",
  ).length;
  const running = plan.steps.some((step) => step.status === "in_progress");
  const timing = formatActivityTiming(
    startedAt,
    finishedAt,
    running && isActive,
    now,
  );
  const [expanded, setExpanded] = useState(defaultExpanded || running);
  return (
    <div
      className="activity-group"
      data-state={running ? "running" : "complete"}
    >
      <button
        className="activity-group-header"
        type="button"
        aria-expanded={expanded}
        onClick={() => setExpanded((current) => !current)}
      >
        <span className="activity-group-icon" aria-hidden="true">
          <Activity size={13} />
        </span>
        <span>执行计划</span>
        <small className="activity-group-count">
          {completed}/{plan.steps.length}
          {timing ? ` · ${timing}` : ""}
        </small>
        {expanded ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
      </button>
      {expanded && (
        <div className="activity-plan">
          {plan.explanation && <p>{plan.explanation}</p>}
          <ol>
            {plan.steps.map((step, index) => {
              const stepTiming = formatPlanStepTiming(
                stepTimings[index],
                step.status,
                isActive,
                now,
              );
              return (
                <li key={`${index}-${step.step}`} data-status={step.status}>
                  <span aria-hidden="true">
                    {step.status === "completed" ? (
                      <Check size={12} />
                    ) : step.status === "in_progress" ? (
                      <Loader2 size={12} className="spin" />
                    ) : (
                      <span className="activity-plan-dot" />
                    )}
                  </span>
                  <span>
                    {step.step}
                    {stepTiming ? ` · ${stepTiming}` : ""}
                  </span>
                </li>
              );
            })}
          </ol>
        </div>
      )}
    </div>
  );
}

function SubagentActivity({ run, now }: { run: SubagentRun; now: number }) {
  const [expanded, setExpanded] = useState(run.status === "running");
  const running = run.status === "running" || run.status === "queued";
  const failed = run.status === "failed" || run.status === "timed_out";
  const timing = formatActivityTiming(
    run.startedAt || run.createdAt,
    run.completedAt || undefined,
    running,
    now,
  );
  return (
    <div
      className="activity-group"
      data-state={failed ? "error" : running ? "running" : "complete"}
    >
      <button
        className="activity-group-header"
        type="button"
        aria-expanded={expanded}
        onClick={() => setExpanded((current) => !current)}
      >
        <span className="activity-group-icon" aria-hidden="true">
          <Bot size={13} />
        </span>
        <span>子智能体：{run.name}</span>
        <small className="activity-group-count">
          {subagentStatusLabel(run.status)}
          {timing ? ` · ${timing}` : ""}
        </small>
        {expanded ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
      </button>
      {expanded && (
        <div className="subagent-activity-details">
          <span>任务</span>
          <p>{redactText(run.input)}</p>
          {(run.result || run.error) && (
            <span>{run.error ? "错误" : "结果"}</span>
          )}
          {run.result && (
            <pre>{truncateText(redactText(run.result), 12_000)}</pre>
          )}
          {run.error && (
            <pre>{truncateText(redactText(run.error), 12_000)}</pre>
          )}
        </div>
      )}
    </div>
  );
}

function ActivityNotice({
  icon,
  title,
  detail,
  tone = "neutral",
}: {
  icon: ReactNode;
  title: string;
  detail: string;
  tone?: "neutral" | "waiting" | "error";
}) {
  return (
    <div className="activity-notice" data-tone={tone}>
      <span aria-hidden="true">{icon}</span>
      <div>
        <strong>{title}</strong>
        <p>{detail}</p>
      </div>
    </div>
  );
}

function ActivityStateIcon({ state }: { state: ActivityState }) {
  if (state === "running") return <Loader2 size={14} className="spin" />;
  if (state === "error" || state === "cancelled") return <X size={14} />;
  if (state === "waiting") return <Clock3 size={14} />;
  return <Check size={14} />;
}

function ActivityResultIcon({
  running,
  failed,
}: {
  running: boolean;
  failed: boolean;
}) {
  if (running)
    return <Loader2 size={12} className="spin activity-result-icon" />;
  if (failed) return <X size={12} className="activity-result-icon" />;
  return <Check size={12} className="activity-result-icon" />;
}

function buildActivityEntries(events: AgentEvent[]): ActivityEntry[] {
  const sorted = [...events].sort((left, right) => left.seq - right.seq);
  const resultEvents = new Map<
    string,
    { result: ToolResult; createdAt: string; seq: number }
  >();
  const startedCallIds = new Set<string>();
  const subagents = new Map<
    string,
    Extract<PrimitiveActivity, { kind: "subagent" }>
  >();
  const primitives: PrimitiveActivity[] = [];
  const planEvents = sorted.filter(
    (event) => event.payload.type === "plan_updated",
  );
  const firstPlanEvent = planEvents[0];
  const latestPlanEvent = planEvents[planEvents.length - 1];
  const latestPlan =
    latestPlanEvent?.payload.type === "plan_updated"
      ? latestPlanEvent.payload.plan
      : undefined;
  const planStepTimings = latestPlan
    ? buildPlanStepTimings(planEvents, latestPlan)
    : [];

  for (const event of sorted) {
    if (event.payload.type === "tool_call_finished") {
      resultEvents.set(event.payload.result.callId, {
        result: event.payload.result,
        createdAt: event.createdAt,
        seq: event.seq,
      });
    }
  }

  for (const event of sorted) {
    const payload = event.payload;
    const reasoning = readReasoningPayload(payload);
    if (reasoning) {
      primitives.push({
        kind: "reasoning",
        seq: event.seq,
        text: reasoning.text,
        isDelta: reasoning.isDelta,
        createdAt: event.createdAt,
      });
    } else if (payload.type === "tool_call_started") {
      startedCallIds.add(payload.call.id);
      const finished = resultEvents.get(payload.call.id);
      primitives.push({
        kind: "tool",
        seq: event.seq,
        execution: {
          call: payload.call,
          startedAt: event.createdAt,
          result: finished?.result,
          finishedAt: finished?.createdAt,
        },
      });
    } else if (
      payload.type === "plan_updated" &&
      event.id === firstPlanEvent?.id &&
      latestPlanEvent?.payload.type === "plan_updated"
    ) {
      primitives.push({
        kind: "plan",
        seq: event.seq,
        plan: latestPlanEvent.payload.plan,
        startedAt: event.createdAt,
        finishedAt: latestPlanEvent.payload.plan.steps.every(
          (step) => step.status === "completed",
        )
          ? latestPlanEvent.createdAt
          : undefined,
        stepTimings: planStepTimings,
      });
    } else if (payload.type === "file_changed") {
      primitives.push({
        kind: "file",
        seq: event.seq,
        path: payload.path,
        summary: payload.summary,
        createdAt: event.createdAt,
      });
    } else if (payload.type === "subagent_updated") {
      const current = subagents.get(payload.run.id);
      if (current) {
        current.run = payload.run;
      } else {
        const entry: Extract<PrimitiveActivity, { kind: "subagent" }> = {
          kind: "subagent",
          seq: event.seq,
          run: payload.run,
          createdAt: event.createdAt,
        };
        subagents.set(payload.run.id, entry);
        primitives.push(entry);
      }
    } else if (payload.type === "approval_requested") {
      primitives.push({
        kind: "approval",
        seq: event.seq,
        reason: payload.reason,
        action: payload.action,
        createdAt: event.createdAt,
      });
    } else if (payload.type === "context_compacted") {
      primitives.push({
        kind: "context",
        seq: event.seq,
        createdAt: event.createdAt,
      });
    } else if (payload.type === "error") {
      primitives.push({
        kind: "error",
        seq: event.seq,
        message: payload.message,
        createdAt: event.createdAt,
      });
    } else if (payload.type === "turn_cancelled") {
      primitives.push({
        kind: "cancelled",
        seq: event.seq,
        reason: payload.reason,
        createdAt: event.createdAt,
      });
    } else if (payload.type === "turn_suspended") {
      primitives.push({
        kind: "suspended",
        seq: event.seq,
        reason: payload.reason,
        createdAt: event.createdAt,
      });
    }
  }

  for (const [callId, finished] of resultEvents) {
    if (startedCallIds.has(callId)) continue;
    const metadata = asRecord(finished.result.metadata);
    primitives.push({
      kind: "tool",
      seq: finished.seq,
      execution: {
        call: {
          id: callId,
          name:
            typeof metadata?.toolName === "string" ? metadata.toolName : "tool",
          input: {},
        },
        startedAt: finished.createdAt,
        result: finished.result,
        finishedAt: finished.createdAt,
      },
    });
  }

  primitives.sort((left, right) => left.seq - right.seq);
  const entries: ActivityEntry[] = [];
  for (const primitive of primitives) {
    if (primitive.kind === "tool") {
      const category = toolCategory(primitive.execution.call.name);
      const previous = entries[entries.length - 1];
      if (previous?.kind === "tool-group" && previous.category === category) {
        previous.executions.push(primitive.execution);
      } else {
        entries.push({
          kind: "tool-group",
          id: `tool-${primitive.seq}`,
          category,
          executions: [primitive.execution],
        });
      }
    } else if (primitive.kind === "file") {
      const previous = entries[entries.length - 1];
      if (previous?.kind === "file-group") {
        previous.files.push({
          path: primitive.path,
          summary: primitive.summary,
          createdAt: primitive.createdAt,
        });
      } else {
        entries.push({
          kind: "file-group",
          id: `file-${primitive.seq}`,
          files: [
            {
              path: primitive.path,
              summary: primitive.summary,
              createdAt: primitive.createdAt,
            },
          ],
        });
      }
    } else if (primitive.kind === "reasoning") {
      const previous = entries[entries.length - 1];
      if (previous?.kind === "reasoning") {
        previous.text = appendReasoningText(
          previous.text,
          primitive.text,
          primitive.isDelta,
        );
      } else {
        entries.push(primitive);
      }
    } else {
      entries.push(primitive);
    }
  }
  return entries;
}

function buildPlanStepTimings(
  planEvents: AgentEvent[],
  latestPlan: TaskPlan,
): PlanStepTiming[] {
  return latestPlan.steps.map((latestStep, stepIndex) => {
    let firstSeenAt: string | undefined;
    let startedAt: string | undefined;
    let finishedAt: string | undefined;

    for (const event of planEvents) {
      if (event.payload.type !== "plan_updated") continue;
      const snapshotStep =
        event.payload.plan.steps.find(
          (step) => step.step === latestStep.step,
        ) ?? event.payload.plan.steps[stepIndex];
      if (!snapshotStep) continue;

      firstSeenAt ??= event.createdAt;
      if (snapshotStep.status === "in_progress") {
        startedAt ??= event.createdAt;
      } else if (snapshotStep.status === "completed") {
        startedAt ??= firstSeenAt;
        finishedAt ??= event.createdAt;
      }
    }

    if (latestStep.status === "in_progress") {
      startedAt ??= firstSeenAt;
    }
    return { startedAt, finishedAt };
  });
}

function useTimelineClock(shouldTick: boolean) {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    setNow(Date.now());
    if (!shouldTick) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1_000);
    return () => window.clearInterval(timer);
  }, [shouldTick]);

  return now;
}

function activityEntryIsRunning(entry: ActivityEntry) {
  if (entry.kind === "tool-group") {
    return entry.executions.some((execution) => !execution.result);
  }
  if (entry.kind === "plan") {
    return entry.plan.steps.some((step) => step.status === "in_progress");
  }
  if (entry.kind === "subagent") {
    return entry.run.status === "queued" || entry.run.status === "running";
  }
  return false;
}

function activityState(events: AgentEvent[], isActive: boolean): ActivityState {
  if (isActive) return "running";
  const terminal = [...events]
    .sort((left, right) => right.seq - left.seq)
    .find((event) =>
      ["turn_finished", "turn_cancelled", "turn_suspended", "error"].includes(
        event.payload.type,
      ),
    )?.payload;
  if (terminal?.type === "error") return "error";
  if (terminal?.type === "turn_cancelled") return "cancelled";
  if (terminal?.type === "turn_suspended") return "waiting";
  return "complete";
}

function activitySummary({
  operationCount,
  commandCount,
  otherToolCount,
  isActive,
}: {
  operationCount: number;
  commandCount: number;
  otherToolCount: number;
  isActive: boolean;
}) {
  if (operationCount === 0) return isActive ? "等待执行步骤" : "未调用工具";
  const parts = [`${operationCount} 步`];
  if (commandCount > 0) parts.push(`${commandCount} 条命令`);
  if (otherToolCount > 0) parts.push(`${otherToolCount} 个工具`);
  return parts.join(" · ");
}

function activityEntryKey(entry: ActivityEntry) {
  if (entry.kind === "tool-group" || entry.kind === "file-group")
    return entry.id;
  if (entry.kind === "subagent") return `subagent-${entry.run.id}`;
  return `${entry.kind}-${entry.seq}`;
}

function toolCategory(name: string): ToolCategory {
  if (name === "shell") return "command";
  if (
    [
      "list_files",
      "read_file",
      "write_file",
      "search",
      "git_diff",
      "apply_patch",
    ].includes(name)
  ) {
    return "file";
  }
  if (name === "browser") return "browser";
  if (name === "spreadsheet") return "spreadsheet";
  if (
    [
      "spawn_agent",
      "send_input",
      "cancel_agent",
      "wait_agent",
      "wait_agents",
    ].includes(name)
  ) {
    return "agent";
  }
  return "tool";
}

function toolCategoryIcon(category: ToolCategory) {
  if (category === "command") return <TerminalSquare size={13} />;
  if (category === "file") return <FileText size={13} />;
  if (category === "browser") return <Globe2 size={13} />;
  if (category === "spreadsheet") return <Table2 size={13} />;
  if (category === "agent") return <Bot size={13} />;
  return <Wrench size={13} />;
}

function toolGroupTitle(category: ToolCategory, count: number) {
  if (category === "command") return `执行了 ${count} 条命令`;
  if (category === "file") return `进行了 ${count} 个文件操作`;
  if (category === "browser") return `进行了 ${count} 个浏览器操作`;
  if (category === "spreadsheet") return `进行了 ${count} 个表格操作`;
  if (category === "agent") return `进行了 ${count} 个子智能体操作`;
  return `调用了 ${count} 个工具`;
}

function toolExecutionTitle(call: ToolCall) {
  const input = asRecord(call.input);
  if (call.name === "shell") {
    return truncateLine(stringField(input, "command") || "运行命令", 140);
  }
  if (call.name === "list_files")
    return `查看文件 ${stringField(input, "path") || "."}`;
  if (call.name === "read_file")
    return `读取 ${stringField(input, "path") || "文件"}`;
  if (call.name === "write_file")
    return `写入 ${stringField(input, "path") || "文件"}`;
  if (call.name === "search") {
    return `搜索 ${stringField(input, "query") || stringField(input, "pattern") || "内容"}`;
  }
  if (call.name === "git_diff") return "检查 Git 变更";
  if (call.name === "apply_patch")
    return `修改文件${patchTarget(input) ? ` ${patchTarget(input)}` : ""}`;
  if (call.name === "browser") {
    const action = stringField(input, "action") || "操作";
    const target = stringField(input, "url") || stringField(input, "selector");
    return `浏览器 ${action}${target ? ` · ${truncateLine(target, 90)}` : ""}`;
  }
  if (call.name === "spreadsheet") {
    const action = stringField(input, "action") || "操作";
    const target =
      stringField(input, "path") || stringField(input, "outputPath");
    return `表格 ${action}${target ? ` · ${target}` : ""}`;
  }
  if (call.name === "spawn_agent")
    return `创建子智能体 ${stringField(input, "name") || ""}`.trim();
  if (call.name === "send_input") return "向子智能体发送消息";
  if (call.name === "wait_agent" || call.name === "wait_agents")
    return "等待子智能体完成";
  if (call.name === "cancel_agent") return "取消子智能体";
  if (call.name === "update_plan") return "更新执行计划";
  if (call.name === "complete_task") return "完成任务";
  if (call.name === "list_skills") return "查看可用 Skill";
  if (call.name === "read_skill")
    return `读取 Skill ${stringField(input, "name") || ""}`.trim();
  if (call.name.startsWith("mcp__")) return `MCP · ${call.name.slice(5)}`;
  return call.name;
}

function toolDisplayName(name: string) {
  const names: Record<string, string> = {
    shell: "Shell",
    list_files: "文件列表",
    read_file: "读取文件",
    write_file: "写入文件",
    search: "代码搜索",
    git_diff: "Git Diff",
    apply_patch: "Apply Patch",
    browser: "浏览器",
    spreadsheet: "Excel",
    spawn_agent: "子智能体",
    send_input: "子智能体",
    wait_agent: "子智能体",
    wait_agents: "子智能体",
    cancel_agent: "子智能体",
    update_plan: "任务计划",
    complete_task: "任务闭环",
  };
  return names[name] || (name.startsWith("mcp__") ? "MCP" : name);
}

function toolFileChangeSummaries(
  execution: ToolExecution,
): FileChangeSummary[] {
  const input = asRecord(execution.call.input);
  const metadataChanges = fileChangesFromMetadata(execution.result?.metadata);

  if (execution.call.name === "write_file") {
    const inputPath = stringField(input, "path");
    const metadataChange = findMatchingFileChange(metadataChanges, inputPath);
    const path = inputPath || metadataChange?.path;
    if (!path) return metadataChanges;
    return [
      {
        path,
        operation: metadataChange?.operation || "写入",
        additions: metadataChange?.additions,
        deletions: metadataChange?.deletions,
      },
    ];
  }

  const diffText =
    execution.call.name === "apply_patch"
      ? stringField(input, "patch")
      : execution.call.name === "git_diff"
        ? execution.result?.output || ""
        : "";
  const diffChanges = parseUnifiedDiffChanges(diffText);
  if (diffChanges.length > 0) {
    return diffChanges.map((change) => {
      const metadataChange = findMatchingFileChange(
        metadataChanges,
        change.path,
      );
      return metadataChange
        ? mergeFileChange(change, metadataChange, change.path)
        : change;
    });
  }
  if (metadataChanges.length > 0) return metadataChanges;

  if (execution.call.name === "apply_patch") {
    const path = patchTarget(input);
    return path ? [{ path, operation: "修改" }] : [];
  }
  return [];
}

function fileChangedEventSummary(file: ActivityFile): FileChangeSummary {
  const stats = parseSummaryLineStats(file.summary);
  return {
    path: file.path,
    operation: fileOperationLabel(file.summary, "修改"),
    ...stats,
    detail: file.summary.trim() || undefined,
  };
}

function fileChangesFromMetadata(value: unknown): FileChangeSummary[] {
  const metadata = asRecord(value);
  if (!metadata) return [];
  const candidates: Record<string, unknown>[] = [];
  for (const key of ["fileChanges", "changes", "files"]) {
    const list = metadata[key];
    if (!Array.isArray(list)) continue;
    for (const item of list) {
      const record = asRecord(item);
      if (record) candidates.push(record);
    }
  }
  if (
    ["changedPath", "path", "filePath"].some(
      (key) => typeof metadata[key] === "string",
    )
  ) {
    candidates.push(metadata);
  }

  return candidates.flatMap((record) => {
    const path = firstStringField(record, [
      "changedPath",
      "path",
      "filePath",
      "file",
    ]);
    if (!path) return [];
    const additions = firstLineCount(record, [
      "additions",
      "addedLines",
      "linesAdded",
      "insertions",
    ]);
    const deletions = firstLineCount(record, [
      "deletions",
      "deletedLines",
      "removedLines",
      "linesDeleted",
    ]);
    const operation = fileOperationLabel(
      firstStringField(record, ["operation", "action", "status"]),
      "修改",
    );
    return [{ path, operation, additions, deletions }];
  });
}

function parseUnifiedDiffChanges(value: string): FileChangeSummary[] {
  if (!value.trim()) return [];
  type MutableChange = FileChangeSummary & { statsKnown: boolean };
  const changes: MutableChange[] = [];
  let current: MutableChange | undefined;
  let oldPath = "";
  let inHunk = false;

  const selectChange = (
    path: string,
    operation = "修改",
    statsKnown = false,
  ) => {
    const normalizedPath = cleanDiffPath(path);
    if (!normalizedPath || normalizedPath === "/dev/null") return undefined;
    const key = normalizeFileChangeKey(normalizedPath);
    current = changes.find(
      (change) => normalizeFileChangeKey(change.path) === key,
    );
    if (!current) {
      current = {
        path: normalizedPath,
        operation,
        additions: 0,
        deletions: 0,
        statsKnown,
      };
      changes.push(current);
    } else {
      current.operation = operation || current.operation;
      current.statsKnown ||= statsKnown;
    }
    return current;
  };

  for (const line of value.split(/\r?\n/)) {
    const customHeader = line.match(
      /^\*\*\* (Add|Update|Delete) File:\s*(.+)$/,
    );
    if (customHeader) {
      const operation =
        customHeader[1] === "Add"
          ? "新建"
          : customHeader[1] === "Delete"
            ? "删除"
            : "修改";
      selectChange(customHeader[2], operation, true);
      inHunk = true;
      continue;
    }

    const diffHeader = line.match(/^diff --git\s+\S+\s+(\S+)$/);
    if (diffHeader) {
      selectChange(diffHeader[1]);
      oldPath = "";
      inHunk = false;
      continue;
    }
    if (line.startsWith("--- ")) {
      oldPath = cleanDiffPath(line.slice(4));
      inHunk = false;
      continue;
    }
    if (line.startsWith("+++ ")) {
      const newPath = cleanDiffPath(line.slice(4));
      const deleting = newPath === "/dev/null";
      selectChange(
        deleting ? oldPath : newPath,
        deleting ? "删除" : oldPath === "/dev/null" ? "新建" : "修改",
      );
      continue;
    }
    if (line.startsWith("@@")) {
      if (current) current.statsKnown = true;
      inHunk = true;
      continue;
    }
    if (!current || !inHunk) continue;
    if (line.startsWith("+") && !line.startsWith("+++")) {
      current.additions = (current.additions || 0) + 1;
      current.statsKnown = true;
    } else if (line.startsWith("-") && !line.startsWith("---")) {
      current.deletions = (current.deletions || 0) + 1;
      current.statsKnown = true;
    }
  }

  return changes.map(({ statsKnown, ...change }) =>
    statsKnown ? change : { path: change.path, operation: change.operation },
  );
}

function parseSummaryLineStats(
  summary: string,
): Pick<FileChangeSummary, "additions" | "deletions"> {
  const compact = summary.replace(/,/g, "");
  const paired = compact.match(/(?:^|\s)\+(\d+)\s+(?:-|−)(\d+)(?:\s|$)/);
  if (paired) {
    return { additions: Number(paired[1]), deletions: Number(paired[2]) };
  }
  const additions = compact.match(/(\d+)\s+(?:insertions?|additions?)\b/i);
  const deletions = compact.match(/(\d+)\s+(?:deletions?|removals?)\b/i);
  return {
    additions: additions ? Number(additions[1]) : undefined,
    deletions: deletions ? Number(deletions[1]) : undefined,
  };
}

function findMatchingFileChange(changes: FileChangeSummary[], path: string) {
  if (!path) return changes[0];
  const key = normalizeFileChangeKey(path);
  return changes.find((change) => {
    const candidate = normalizeFileChangeKey(change.path);
    return (
      candidate === key ||
      candidate.endsWith(`/${key}`) ||
      key.endsWith(`/${candidate}`)
    );
  });
}

function mergeFileChange(
  base: FileChangeSummary,
  overlay: FileChangeSummary,
  path = base.path,
): FileChangeSummary {
  return {
    path,
    operation: overlay.operation || base.operation,
    additions: overlay.additions ?? base.additions,
    deletions: overlay.deletions ?? base.deletions,
    detail: overlay.detail || base.detail,
  };
}

function fileOperationLabel(value: string, fallback: string) {
  const normalized = value.toLowerCase();
  if (/\b(add|added|create|created|new)\b|新建|创建/.test(normalized))
    return "新建";
  if (/\b(delete|deleted|remove|removed)\b|删除/.test(normalized))
    return "删除";
  if (/\b(write|wrote|written)\b|写入/.test(normalized)) return "写入";
  if (/\b(revert|reverted|restore|restored)\b|回滚|恢复/.test(normalized))
    return "回滚";
  return fallback;
}

function cleanDiffPath(value: string) {
  const withoutTimestamp = value.trim().split("\t", 1)[0].replace(/^"|"$/g, "");
  if (withoutTimestamp === "/dev/null") return withoutTimestamp;
  return withoutTimestamp.replace(/^(?:a|b)[\\/]/, "").replace(/\\/g, "/");
}

function normalizeFileChangeKey(value: string) {
  return cleanDiffPath(value).replace(/^\.\//, "").toLowerCase();
}

function firstStringField(record: Record<string, unknown>, keys: string[]) {
  for (const key of keys) {
    if (typeof record[key] === "string" && record[key].trim()) {
      return record[key].trim();
    }
  }
  return "";
}

function firstLineCount(record: Record<string, unknown>, keys: string[]) {
  for (const key of keys) {
    const value = record[key];
    const count =
      typeof value === "number"
        ? value
        : typeof value === "string" && /^\d+$/.test(value)
          ? Number(value)
          : undefined;
    if (count !== undefined && Number.isFinite(count) && count >= 0) {
      return Math.floor(count);
    }
  }
  return undefined;
}

function fileChangeStatsLabel(change: FileChangeSummary) {
  const parts: string[] = [];
  if (change.additions !== undefined) parts.push(`新增 ${change.additions} 行`);
  if (change.deletions !== undefined) parts.push(`删除 ${change.deletions} 行`);
  return parts.join("，");
}

function readReasoningPayload(value: unknown) {
  const payload = asRecord(value);
  const type = stringField(payload, "type");
  if (
    ![
      "reasoning_delta",
      "reasoning_summary",
      "reasoning_summary_delta",
      "model_reasoning_delta",
      "thinking_delta",
    ].includes(type)
  ) {
    return null;
  }
  const nestedSummary = asRecord(payload?.summary);
  const rawText =
    stringField(payload, "text") ||
    (typeof payload?.summary === "string" ? payload.summary : "") ||
    stringField(nestedSummary, "text");
  if (!rawText.trim()) return null;
  const isDelta = type.endsWith("_delta");
  return { text: isDelta ? rawText : rawText.trim(), isDelta };
}

function appendReasoningText(previous: string, text: string, isDelta: boolean) {
  if (!previous) return text;
  if (previous === text || previous.endsWith(text)) return previous;
  return isDelta ? `${previous}${text}` : `${previous}\n${text}`;
}

function formatToolInput(call: ToolCall, hasFileSummary = false) {
  const input = asRecord(call.input);
  if (call.name === "shell") {
    return truncateText(
      redactText(stringField(input, "command") || "未提供命令"),
      20_000,
    );
  }
  let value: unknown = call.input;
  if (
    call.name === "write_file" &&
    input &&
    typeof input.content === "string"
  ) {
    value = {
      ...input,
      content: `[已隐藏文件正文，共 ${input.content.length} 个字符]`,
    };
  } else if (
    call.name === "apply_patch" &&
    input &&
    typeof input.patch === "string" &&
    !hasFileSummary
  ) {
    value = {
      ...input,
      patch: `[已隐藏补丁正文，共 ${input.patch.length} 个字符；未获得可靠的文件行数统计]`,
    };
  }
  return truncateText(
    JSON.stringify(sanitizeValue(value), null, 2) || "{}",
    20_000,
  );
}

function formatToolOutput(result?: ToolResult) {
  if (!result) return "等待工具返回...";
  const output = redactText(result.output || "").trim();
  return output
    ? truncateText(output, 20_000)
    : "工具执行完成，未返回文本输出。";
}

function toolResultFailed(result?: ToolResult) {
  if (!result) return false;
  const metadata = asRecord(result.metadata);
  return metadata?.success === false || metadata?.isError === true;
}

function formatTurnTiming(
  events: AgentEvent[],
  isActive: boolean,
  now: number,
  mountedAt: number,
) {
  const validEvents = [...events]
    .sort((left, right) => left.seq - right.seq)
    .map((event) => ({ event, time: parseTimestamp(event.createdAt) }))
    .filter(
      (item): item is { event: AgentEvent; time: number } => item.time !== null,
    );
  const turnStarted = validEvents.find(
    ({ event }) => event.payload.type === "turn_started",
  );
  const startedAt =
    turnStarted?.time ?? validEvents[0]?.time ?? (isActive ? mountedAt : null);
  if (startedAt === null || startedAt === undefined) return "";

  const terminal = [...validEvents]
    .reverse()
    .find(({ event }) =>
      ["turn_finished", "turn_cancelled", "turn_suspended", "error"].includes(
        event.payload.type,
      ),
    );
  const finishedAt = isActive
    ? now
    : (terminal?.time ?? validEvents[validEvents.length - 1]?.time ?? null);
  if (finishedAt === null || finishedAt < startedAt) return "";
  return `${isActive ? "已运行" : "总耗时"} ${formatElapsed(
    finishedAt - startedAt,
  )}`;
}

function formatActivityTiming(
  startedAt?: string | null,
  finishedAt?: string | null,
  running = false,
  now = Date.now(),
) {
  const start = parseTimestamp(startedAt);
  if (start === null) return "";
  const finish = running ? now : parseTimestamp(finishedAt);
  if (finish === null || finish < start) return "";
  return `${running ? "已运行" : "耗时"} ${formatElapsed(finish - start)}`;
}

function formatExecutionGroupTiming(
  executions: ToolExecution[],
  running: boolean,
  now: number,
) {
  const starts = executions
    .map((execution) => parseTimestamp(execution.startedAt))
    .filter((value): value is number => value !== null);
  if (starts.length === 0) return "";
  const start = Math.min(...starts);
  const finishes = executions
    .map((execution) => parseTimestamp(execution.finishedAt))
    .filter((value): value is number => value !== null);
  const finish = running
    ? now
    : finishes.length > 0
      ? Math.max(...finishes)
      : null;
  return formatParsedTiming(start, finish, running);
}

function formatFileGroupTiming(files: Array<{ createdAt: string }>) {
  const times = files
    .map((file) => parseTimestamp(file.createdAt))
    .filter((value): value is number => value !== null);
  if (times.length === 0) return "";
  return formatParsedTiming(Math.min(...times), Math.max(...times), false);
}

function formatPlanStepTiming(
  timing: PlanStepTiming | undefined,
  status: TaskPlan["steps"][number]["status"],
  isActive: boolean,
  now: number,
) {
  if (status === "pending") return "尚未开始";
  const formatted = formatActivityTiming(
    timing?.startedAt,
    timing?.finishedAt,
    status === "in_progress" && isActive,
    now,
  );
  return formatted || "时间不可用";
}

function formatParsedTiming(
  startedAt: number,
  finishedAt: number | null,
  running: boolean,
) {
  if (finishedAt === null || finishedAt < startedAt) return "";
  return `${running ? "已运行" : "耗时"} ${formatElapsed(
    finishedAt - startedAt,
  )}`;
}

function parseTimestamp(value?: string | null) {
  if (!value) return null;
  const timestamp = Date.parse(value);
  return Number.isFinite(timestamp) ? timestamp : null;
}

function formatElapsed(duration: number) {
  const safeDuration = Math.max(0, Math.round(duration));
  if (safeDuration < 1_000) return `${safeDuration} ms`;
  if (safeDuration < 10_000) return `${(safeDuration / 1_000).toFixed(1)} 秒`;
  if (safeDuration < 60_000) return `${Math.round(safeDuration / 1_000)} 秒`;

  const totalSeconds = Math.round(safeDuration / 1_000);
  const seconds = totalSeconds % 60;
  const totalMinutes = Math.floor(totalSeconds / 60);
  const minutes = totalMinutes % 60;
  const hours = Math.floor(totalMinutes / 60);
  if (hours > 0) return `${hours} 小时 ${minutes} 分 ${seconds} 秒`;
  return `${minutes} 分 ${seconds} 秒`;
}

function subagentStatusLabel(status: SubagentRun["status"]) {
  const labels: Record<SubagentRun["status"], string> = {
    queued: "排队中",
    running: "执行中",
    completed: "已完成",
    failed: "失败",
    cancelled: "已取消",
    timed_out: "超时",
  };
  return labels[status];
}

function patchTarget(input: Record<string, unknown> | null) {
  const patch = stringField(input, "patch");
  if (!patch) return "";
  return (
    patch.match(/^\*\*\* (?:Update|Add|Delete) File:\s*(.+)$/m)?.[1]?.trim() ||
    ""
  );
}

function stringField(record: Record<string, unknown> | null, key: string) {
  const value = record?.[key];
  return typeof value === "string" ? value : "";
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function sanitizeValue(value: unknown, key = "", depth = 0): unknown {
  if (/api[_-]?key|token|secret|password|authorization|credential/i.test(key)) {
    return "[已隐藏]";
  }
  if (depth > 8) return "[内容层级过深]";
  if (typeof value === "string") return redactText(value);
  if (Array.isArray(value)) {
    return value
      .slice(0, 100)
      .map((item) => sanitizeValue(item, key, depth + 1));
  }
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>).map(
        ([entryKey, item]) => [
          entryKey,
          sanitizeValue(item, entryKey, depth + 1),
        ],
      ),
    );
  }
  return value;
}

function redactText(value: string) {
  return value
    .replace(/(Bearer\s+)[^\s"'`]+/gi, "$1[已隐藏]")
    .replace(
      /((?:api[_-]?key|token|secret|password|authorization|credential)\s*[:=]\s*)("[^"]*"|'[^']*'|[^\s,;]+)/gi,
      "$1[已隐藏]",
    )
    .replace(/\bsk-[A-Za-z0-9_-]{8,}\b/g, "[已隐藏]");
}

function truncateLine(value: string, limit: number) {
  const line = value.replace(/\s+/g, " ").trim();
  return line.length <= limit ? line : `${line.slice(0, limit - 1)}…`;
}

function truncateText(value: string, limit: number) {
  return value.length <= limit
    ? value
    : `${value.slice(0, limit)}\n\n… 输出已截断，共 ${value.length} 个字符`;
}
