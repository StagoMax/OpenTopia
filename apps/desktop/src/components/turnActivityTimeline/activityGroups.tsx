import { useEffect, useRef, useState } from "react";
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
  type ToolActivityIconKind,
} from "../../toolActivity";
import { toolExecutionDurationMs } from "../../toolExecutionTiming";
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

export function ActivityEntryView({
  entry,
  isActive,
  traceThreadId,
  traceTurnId,
  formatError,
  onOpenMarkdownLink,
}: {
  entry: ActivityEntry;
  isActive: boolean;
  traceThreadId?: string;
  traceTurnId?: string | null;
  formatError(message: string): string;
  onOpenMarkdownLink?(href: string): void;
}) {
  if (entry.kind === "tool-group") {
    return (
      <ToolActivityGroup
        group={entry.group}
        executions={entry.executions}
        defaultExpanded={isActive}
      />
    );
  }
  if (entry.kind === "file-group") {
    return <FileActivityGroup files={entry.files} defaultExpanded={isActive} />;
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
    return (
      <ActivityNotice
        className="is-reconnect"
        icon={<Wifi size={13} />}
        title={`正在重新连接 ${entry.retryIndex}/${entry.retryLimit}`}
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
  defaultExpanded,
}: {
  group: ToolGroupKey;
  executions: ToolExecution[];
  defaultExpanded: boolean;
}) {
  const state = toolActivityGroupStatus(
    executions.map((execution) => execution.result),
  );
  const running = state === "running";
  const now = useTimelineClock(running);
  const commandBatch = group === "shell" && executions.length > 1;
  const runningCommand = [...executions]
    .reverse()
    .find((execution) => !execution.result);
  const timing = formatExecutionGroupTiming(executions, running, now);
  const [expanded, setExpanded] = useState(
    !commandBatch && (defaultExpanded || running),
  );
  const wasRunning = useRef(running);

  useEffect(() => {
    if (commandBatch) {
      if (!running && wasRunning.current) setExpanded(false);
      wasRunning.current = running;
      return;
    }
    if (running) setExpanded(true);
  }, [commandBatch, running]);

  if (executions.length === 1) {
    return <ToolExecutionItem execution={executions[0]} now={now} />;
  }

  if (commandBatch && running && runningCommand) {
    return (
      <ToolExecutionItem execution={runningCommand} now={now} currentCommand />
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
          {toolActivityIcon(toolGroupIconKind(group, executions), 13)}
        </span>
        <span>
          {toolGroupTitle(group, executions)}
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
  currentCommand = false,
}: {
  execution: ToolExecution;
  now: number;
  currentCommand?: boolean;
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
      streaming={currentCommand}
    />
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

function toolGroupTitle(group: ToolGroupKey, executions: ToolExecution[]) {
  const count = executions.length;
  if (group === "explore") return `探索了 ${count} 处`;
  if (group === "shell") return `运行了 ${count} 个命令`;
  if (group === "edit") return `修改了 ${count} 次文件`;
  if (group === "browser") return `进行了 ${count} 个浏览器操作`;
  if (group === "computer") return `进行了 ${count} 个计算机操作`;
  if (group === "spreadsheet") return `进行了 ${count} 个表格操作`;
  if (group === "agent") return `进行了 ${count} 个子智能体操作`;
  if (group === "plan") return `更新了 ${count} 次执行计划`;
  if (group === "skill") return `进行了 ${count} 次 Skill 操作`;
  if (group === "attachment") {
    if (executions.every(({ call }) => call.name === "view_attachment")) {
      return `查看了 ${count} 张图片`;
    }
    if (executions.every(({ call }) => call.name === "read_attachment")) {
      return `读取了 ${count} 个附件`;
    }
    return `处理了 ${count} 个附件`;
  }
  if (group === "mcp") return `调用了 ${count} 个 MCP 工具`;
  return `调用了 ${count} 个工具`;
}

function toolGroupIconKind(
  group: ToolGroupKey,
  executions: ToolExecution[],
): ToolActivityIconKind {
  if (group === "explore") return "search";
  if (group === "shell") return "shell";
  if (group === "edit") return "edit";
  if (
    group === "attachment" &&
    executions.every(({ call }) => call.name === "view_attachment")
  ) {
    return "image";
  }
  return group;
}
