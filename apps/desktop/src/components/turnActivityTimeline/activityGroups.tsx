import { useEffect, useMemo, useState } from "react";
import {
  AlertCircle,
  ChevronDown,
  ChevronRight,
  Clock3,
  FileText,
  Wifi,
  X,
} from "lucide-react";
import {
  toolActivityGroupStatus,
  type ToolActivityGroup as ToolGroupKey,
} from "../../toolActivity";
import { toolExecutionDurationMs } from "../../toolExecutionTiming";
import type { ToolResult } from "../../types";
import { ToolActivityCard, toolActivityIcon } from "../ToolActivityCard";
import {
  ActivityNotice,
  ActivityResultIcon,
  ContextCompactionActivity,
  GuardianReviewActivity,
  NarrativeActivity,
  WorkFormActivity,
} from "./activityDetails";
import {
  fileChangedEventSummary,
  fileChangeStatsLabel,
  type ActivityEntry,
  type ActivityFile,
  type FileChangeSummary,
  type ToolExecution,
} from "./model";
import {
  formatActivityTiming,
  formatExecutionGroupTiming,
  formatFileGroupTiming,
  formatToolSandbox,
} from "./timing";
import { useTimelineClock } from "./hooks";
import { buildToolGroupPresentation } from "./toolGroupPresentation";

export function ActivityEntryView({
  entry,
  isActive,
  traceThreadId,
  traceTurnId,
  formatError,
  onOpenMarkdownLink,
  onLoadToolResultDetail,
}: {
  entry: ActivityEntry;
  isActive: boolean;
  traceThreadId?: string;
  traceTurnId?: string | null;
  formatError(message: string): string;
  onOpenMarkdownLink?(href: string): void;
  onLoadToolResultDetail?(eventId: string): Promise<ToolResult>;
}) {
  if (entry.kind === "tool-group") {
    return (
      <ToolActivityGroup
        group={entry.group}
        executions={entry.executions}
        isActive={isActive}
        onLoadToolResultDetail={onLoadToolResultDetail}
      />
    );
  }
  if (entry.kind === "file-group") {
    return <FileActivityGroup files={entry.files} />;
  }
  if (entry.kind === "reasoning") {
    return null;
  }
  if (entry.kind === "commentary") {
    return (
      <NarrativeActivity
        onOpenMarkdownLink={onOpenMarkdownLink}
        streaming={isActive}
        text={entry.text}
        traceThreadId={traceThreadId}
        traceTurnId={traceTurnId}
      />
    );
  }
  if (entry.kind === "work-form") {
    return <TimedWorkFormActivity entry={entry} isActive={isActive} />;
  }
  if (entry.kind === "context-compaction") {
    return <TimedContextCompactionActivity entry={entry} />;
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
  if (entry.kind === "guardian-review") {
    return <TimedGuardianReviewActivity entry={entry} />;
  }
  if (entry.kind === "browser-handoff") {
    const details = [entry.reason, entry.url, "完成后在对话中告诉我继续。"]
      .filter((value): value is string => Boolean(value))
      .join("\n");
    return (
      <ActivityNotice
        icon={<Clock3 size={13} />}
        tone="waiting"
        title="需要手动完成浏览器操作"
        detail={details}
      />
    );
  }
  if (entry.kind === "browser-handoff-completed") {
    return (
      <ActivityNotice
        icon={<Clock3 size={13} />}
        title="已继续浏览器任务"
        detail="正在根据当前页面状态继续执行。"
      />
    );
  }
  if (entry.kind === "reconnect") {
    const retryCount =
      typeof entry.retryIndex === "number" &&
      typeof entry.retryLimit === "number"
        ? ` ${entry.retryIndex}/${entry.retryLimit}`
        : "";
    const title =
      entry.retryKind === "state_recovery"
        ? `正在恢复模型响应${retryCount}`
        : `正在重新连接${retryCount}`;
    return (
      <ActivityNotice
        className="is-reconnect"
        icon={<Wifi size={13} />}
        title={title}
        detail={entry.reason}
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
  group,
  executions,
  isActive,
  onLoadToolResultDetail,
}: {
  group: ToolGroupKey;
  executions: ToolExecution[];
  isActive: boolean;
  onLoadToolResultDetail?(eventId: string): Promise<ToolResult>;
}) {
  const state = toolActivityGroupStatus(
    executions.map((execution) => execution.result),
  );
  const running = state === "running";
  const now = useTimelineClock(running);
  const timing = formatExecutionGroupTiming(executions, running, now);
  const [presentationRevision, setPresentationRevision] = useState(0);
  const presentation = useMemo(
    () => buildToolGroupPresentation(group, executions, Date.now()),
    [executions, group, presentationRevision],
  );
  const [expanded, setExpanded] = useState(false);

  useEffect(() => {
    if (!presentation.settleUntil) return;
    const remaining = presentation.settleUntil - Date.now();
    if (remaining <= 0) {
      setPresentationRevision((current) => current + 1);
      return;
    }
    const timer = window.setTimeout(
      () => setPresentationRevision((current) => current + 1),
      remaining,
    );
    return () => window.clearTimeout(timer);
  }, [presentation.settleUntil]);

  if (executions.length === 1 && !isActive) {
    return (
      <ToolExecutionItem
        execution={executions[0]}
        now={now}
        onLoadToolResultDetail={onLoadToolResultDetail}
      />
    );
  }

  return (
    <div className="activity-group" data-state={state}>
      <button
        className="activity-group-header"
        type="button"
        aria-expanded={expanded}
        onClick={() => setExpanded((current) => !current)}
      >
        <span className="activity-group-icon" aria-hidden="true">
          {toolActivityIcon(presentation.iconKind, 13)}
        </span>
        <span className="activity-group-title" title={presentation.label}>
          <span aria-live="polite">{presentation.label}</span>
          {timing ? ` · ${timing}` : ""}
        </span>
        <ActivityResultIcon running={running} />
        {expanded ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
      </button>
      {expanded && (
        <div className="activity-group-content">
          {executions.map((execution) => (
            <ToolExecutionItem
              key={execution.call.id}
              execution={execution}
              now={now}
              onLoadToolResultDetail={onLoadToolResultDetail}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function TimedWorkFormActivity({
  entry,
  isActive,
}: {
  entry: Extract<ActivityEntry, { kind: "work-form" }>;
  isActive: boolean;
}) {
  const running =
    isActive && entry.form.items.some((item) => item.status === "in_progress");
  const now = useTimelineClock(running);
  return (
    <WorkFormActivity
      form={entry.form}
      itemTimings={entry.itemTimings}
      startedAt={entry.startedAt}
      finishedAt={entry.finishedAt}
      defaultExpanded={isActive}
      isActive={isActive}
      now={now}
    />
  );
}

function TimedGuardianReviewActivity({
  entry,
}: {
  entry: Extract<ActivityEntry, { kind: "guardian-review" }>;
}) {
  const now = useTimelineClock(!entry.completed);
  return <GuardianReviewActivity entry={entry} now={now} />;
}

function TimedContextCompactionActivity({
  entry,
}: {
  entry: Extract<ActivityEntry, { kind: "context-compaction" }>;
}) {
  const now = useTimelineClock(!entry.finishedAt);
  return <ContextCompactionActivity entry={entry} now={now} />;
}

function ToolExecutionItem({
  execution,
  now,
  onLoadToolResultDetail,
}: {
  execution: ToolExecution;
  now: number;
  onLoadToolResultDetail?(eventId: string): Promise<ToolResult>;
}) {
  const running = !execution.result;
  const timing = formatActivityTiming(
    execution.startedAt,
    execution.finishedAt,
    running,
    now,
    toolExecutionDurationMs(execution.result),
  );

  return (
    <ToolActivityCard
      call={execution.call}
      result={execution.result}
      timing={timing}
      sandbox={formatToolSandbox(execution.result)}
      onLoadResultDetail={onLoadToolResultDetail}
    />
  );
}

function FileActivityGroup({ files }: { files: ActivityFile[] }) {
  const [expanded, setExpanded] = useState(false);
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
        <ActivityResultIcon running={false} />
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
