import type { ReactNode } from "react";
import {
  Bot,
  ChevronDown,
  ChevronRight,
  CircleDot,
  File,
  FileCode2,
  FileImage,
  FileText,
  Folder,
  GitBranch,
  Github,
  GitPullRequest,
  Laptop,
  Package,
  Plus,
  SquareTerminal,
  UploadCloud,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import type {
  AgentEvent,
  ArtifactDescriptor,
  TerminalEvent,
  TerminalSession,
  WorkspaceDiff,
} from "../types";
import "../styles/right-context-rail.css";

export type RightContextRailProps = {
  workspaceRoot: string | null;
  workspaceDiff: WorkspaceDiff | null;
  terminalEvents: TerminalEvent[];
  terminalSession: TerminalSession | null;
  agentEvents: AgentEvent[];
  artifacts: ArtifactDescriptor[];
  onOpenDiff(): void;
  onOpenTerminal(): void;
  onOpenFiles(): void;
  onOpenExtensions(): void;
};

type RailRowProps = {
  icon: LucideIcon;
  label: string;
  value?: ReactNode;
  title?: string;
  className?: string;
  disabled?: boolean;
  onClick?: () => void;
};

type ActiveProcess = {
  id: string;
  label: string;
  title: string;
  startedAt: string;
  kind: "session" | "command";
};

type SubagentItem = {
  id: string;
  label: string;
  status: "运行中" | "已返回" | "失败";
  createdAt: string;
};

type SourceItem = {
  id: string;
  label: string;
  title: string;
  createdAt: string;
  icon: LucideIcon;
  dedupeKey: string;
};

const SOURCE_LIMIT = 4;

export function RightContextRail({
  workspaceRoot,
  workspaceDiff,
  terminalEvents,
  terminalSession,
  agentEvents,
  artifacts,
  onOpenDiff,
  onOpenTerminal,
  onOpenFiles,
  onOpenExtensions,
}: RightContextRailProps) {
  const diffStats = countDiffLines(workspaceDiff?.diff ?? "");
  const branch = workspaceDiff?.branch?.trim() || "非 Git 仓库";
  const activeProcesses = collectActiveProcesses(
    terminalSession,
    terminalEvents,
  );
  const subagents = collectSubagents(agentEvents);
  const sources = collectSources(agentEvents, artifacts).slice(0, SOURCE_LIMIT);

  return (
    <div className="right-context-rail" aria-label="右侧上下文摘要">
      <RailSection
        title="环境信息"
        action={
          <button
            className="right-context-rail__header-action"
            type="button"
            disabled
            title="添加环境 · 未实现"
            aria-label="添加环境"
          >
            <Plus size={14} aria-hidden="true" />
          </button>
        }
      >
        <RailRow
          icon={FileCode2}
          label="变更"
          title={
            workspaceDiff
              ? `${workspaceDiff.files.length} 个变更文件`
              : "暂无变更数据"
          }
          onClick={onOpenDiff}
          value={
            workspaceDiff ? (
              <span className="right-context-rail__diff-stats">
                <span className="is-addition">+{diffStats.additions}</span>
                <span className="is-deletion">-{diffStats.deletions}</span>
              </span>
            ) : (
              <StatusText muted>暂无</StatusText>
            )
          }
        />
        <RailRow
          icon={Laptop}
          label="本地"
          title={workspaceRoot ?? "暂无工作区"}
          onClick={onOpenFiles}
          value={
            <span className="right-context-rail__inline-value">
              <StatusText muted={!workspaceRoot}>
                {workspaceRoot ? "" : "暂无"}
              </StatusText>
              {workspaceRoot && <ChevronDown size={13} aria-hidden="true" />}
            </span>
          }
        />
        <RailRow
          icon={GitBranch}
          label="分支"
          title={`${branch} · 分支切换未实现`}
          disabled
          value={
            <span className="right-context-rail__inline-value">
              <StatusText muted={!workspaceDiff?.branch}>{branch}</StatusText>
              {workspaceDiff?.branch && (
                <ChevronDown size={13} aria-hidden="true" />
              )}
            </span>
          }
        />
        <RailRow
          icon={UploadCloud}
          label="提交或推送"
          value={<StatusText muted>未实现</StatusText>}
          title="提交或推送 · 未实现"
          disabled
        />
        <RailRow
          icon={Github}
          label="GitHub CLI"
          value={<StatusText muted>不可用</StatusText>}
          title="GitHub CLI 不可用；打开扩展"
          onClick={onOpenExtensions}
        />
        <RailRow
          icon={GitPullRequest}
          label="比较分支"
          value={<StatusText muted>未实现</StatusText>}
          title="比较分支 · 未实现"
          disabled
        />
      </RailSection>

      <RailSection title={`子智能体 ${subagents.length}`}>
        {subagents.length ? (
          subagents.slice(0, 4).map((agent) => (
            <RailRow
              key={agent.id}
              icon={Bot}
              label={agent.label}
              title={agent.label}
              value={
                <StatusText
                  muted={agent.status !== "运行中"}
                  danger={agent.status === "失败"}
                >
                  {agent.status}
                </StatusText>
              }
            />
          ))
        ) : (
          <EmptyRow icon={Bot} label="暂无" />
        )}
      </RailSection>

      <RailSection title={`后台进程 ${activeProcesses.length}`}>
        {activeProcesses.length ? (
          activeProcesses.map((process) => (
            <RailRow
              key={process.id}
              icon={process.kind === "session" ? SquareTerminal : CircleDot}
              label={process.label}
              title={process.title}
              onClick={onOpenTerminal}
              value={<StatusText active>运行中</StatusText>}
            />
          ))
        ) : (
          <EmptyRow icon={SquareTerminal} label="暂无" />
        )}
      </RailSection>

      <RailSection
        title="来源"
        action={
          <button
            className="right-context-rail__header-action"
            type="button"
            disabled
            title="添加来源 · 未实现"
            aria-label="添加来源"
          >
            <Plus size={14} aria-hidden="true" />
          </button>
        }
      >
        {sources.length ? (
          sources.map((source) => (
            <RailRow
              key={source.id}
              icon={source.icon}
              label={source.label}
              title={source.title}
              onClick={onOpenFiles}
              className="is-source"
            />
          ))
        ) : (
          <EmptyRow icon={File} label="暂无" />
        )}
        <button
          className="right-context-rail__view-all"
          type="button"
          disabled
          title="查看全部 · 未实现"
        >
          <span>查看全部</span>
          <ChevronRight size={13} aria-hidden="true" />
        </button>
      </RailSection>
    </div>
  );
}

function RailSection({
  title,
  titleHint,
  action,
  children,
}: {
  title: string;
  titleHint?: string;
  action?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="right-context-rail__section">
      <header className="right-context-rail__section-header">
        <span title={titleHint}>{title}</span>
        {action}
      </header>
      <div className="right-context-rail__rows">{children}</div>
    </section>
  );
}

function RailRow({
  icon: Icon,
  label,
  value,
  title,
  className = "",
  disabled = false,
  onClick,
}: RailRowProps) {
  const content = (
    <>
      <Icon size={14} aria-hidden="true" />
      <span className="right-context-rail__row-label">{label}</span>
      {value}
    </>
  );
  const classes = `right-context-rail__row ${className}`.trim();

  if (onClick || disabled) {
    return (
      <button
        className={classes}
        type="button"
        onClick={onClick}
        disabled={disabled}
        title={title}
      >
        {content}
      </button>
    );
  }

  return (
    <div className={classes} title={title}>
      {content}
    </div>
  );
}

function StatusText({
  children,
  muted = false,
  active = false,
  danger = false,
}: {
  children: ReactNode;
  muted?: boolean;
  active?: boolean;
  danger?: boolean;
}) {
  const state = danger
    ? "is-danger"
    : active
      ? "is-active"
      : muted
        ? "is-muted"
        : "";
  return (
    <span className={`right-context-rail__status ${state}`.trim()}>
      {children}
    </span>
  );
}

function EmptyRow({ icon, label }: { icon: LucideIcon; label: string }) {
  return <RailRow icon={icon} label={label} className="is-empty" />;
}

function countDiffLines(diff: string): {
  additions: number;
  deletions: number;
} {
  let additions = 0;
  let deletions = 0;

  for (const line of diff.split(/\r?\n/)) {
    if (line.startsWith("+++") || line.startsWith("---")) continue;
    if (line.startsWith("+")) additions += 1;
    if (line.startsWith("-")) deletions += 1;
  }

  return { additions, deletions };
}

function collectActiveProcesses(
  terminalSession: TerminalSession | null,
  events: TerminalEvent[],
): ActiveProcess[] {
  const processes: ActiveProcess[] = [];
  if (terminalSession?.status === "running") {
    const shell = terminalSession.shell.trim() || "终端会话";
    const processDetails = [
      terminalSession.shell,
      terminalSession.cwd,
      terminalSession.processId ? `PID ${terminalSession.processId}` : null,
    ].filter(Boolean);
    processes.push({
      id: `session:${terminalSession.sessionId}`,
      label: executableName(shell),
      title: processDetails.join("\n"),
      startedAt: terminalSession.startedAt,
      kind: "session",
    });
  }

  const terminalEventsByCommand = new Map<string, TerminalEvent[]>();
  for (const event of events) {
    const commandEvents = terminalEventsByCommand.get(event.commandId) ?? [];
    commandEvents.push(event);
    terminalEventsByCommand.set(event.commandId, commandEvents);
  }

  const commands: ActiveProcess[] = [];
  for (const [commandId, commandEvents] of terminalEventsByCommand) {
    const started = [...commandEvents]
      .reverse()
      .find((event) => event.type === "started");
    const hasTerminalEvent = commandEvents.some((event) =>
      ["finished", "cancelled", "error"].includes(event.type),
    );
    if (!started || hasTerminalEvent) continue;

    const command = started.command?.trim() || commandId;
    const mirrorsInteractiveSession =
      terminalSession &&
      (commandId === terminalSession.sessionId ||
        /^interactive\b/i.test(command));
    if (mirrorsInteractiveSession) continue;
    commands.push({
      id: `command:${commandId}`,
      label: command,
      title: [command, started.cwd].filter(Boolean).join("\n"),
      startedAt: started.createdAt,
      kind: "command",
    });
  }

  commands.sort(
    (left, right) => timestamp(right.startedAt) - timestamp(left.startedAt),
  );
  return [...processes, ...commands];
}

function collectSubagents(events: AgentEvent[]): SubagentItem[] {
  const finishedCalls = new Map(
    events
      .filter((event) => event.payload.type === "tool_call_finished")
      .map((event) =>
        event.payload.type === "tool_call_finished"
          ? ([event.payload.result.callId, event.payload.result] as const)
          : (["", null] as const),
      ),
  );

  return events
    .filter((event) => event.payload.type === "tool_call_started")
    .flatMap((event): SubagentItem[] => {
      if (event.payload.type !== "tool_call_started") return [];
      const { call } = event.payload;
      if (!isSubagentTool(call.name)) return [];
      const result = finishedCalls.get(call.id);
      return [
        {
          id: call.id,
          label: subagentLabel(call.input) ?? call.name,
          status: result
            ? toolResultFailed(result.metadata)
              ? "失败"
              : "已返回"
            : "运行中",
          createdAt: event.createdAt,
        },
      ];
    })
    .sort(
      (left, right) => timestamp(right.createdAt) - timestamp(left.createdAt),
    );
}

function collectSources(
  events: AgentEvent[],
  artifacts: ArtifactDescriptor[],
): SourceItem[] {
  const candidates: SourceItem[] = [];

  for (const event of events) {
    if (event.payload.type !== "file_changed") continue;
    const path = event.payload.path.trim();
    if (!path) continue;
    candidates.push({
      id: `change:${event.id}`,
      label: path,
      title: path,
      createdAt: event.createdAt,
      icon: sourceIcon(path, ""),
      dedupeKey: `path:${normalizePath(path)}`,
    });
  }

  for (const artifact of artifacts) {
    const path = artifactPath(artifact);
    const label =
      path || artifact.kind.trim() || `产物 ${shortId(artifact.id)}`;
    candidates.push({
      id: `artifact:${artifact.id}`,
      label,
      title: path
        ? `${path}\n${artifact.contentType}`
        : `${artifact.kind || "产物"}\n${artifact.id}\n${artifact.contentType}`,
      createdAt: artifact.createdAt,
      icon: sourceIcon(label, artifact.contentType),
      dedupeKey: path
        ? `path:${normalizePath(path)}`
        : `artifact:${artifact.id}`,
    });
  }

  candidates.sort(
    (left, right) => timestamp(right.createdAt) - timestamp(left.createdAt),
  );

  const seen = new Set<string>();
  return candidates.filter((source) => {
    if (seen.has(source.dedupeKey)) return false;
    seen.add(source.dedupeKey);
    return true;
  });
}

function artifactPath(artifact: ArtifactDescriptor): string | null {
  if (
    artifact.storage?.type === "path" &&
    typeof artifact.storage.path === "string"
  ) {
    return artifact.storage.path.trim() || null;
  }
  return pathFromMetadata(artifact.metadata);
}

function pathFromMetadata(metadata: unknown): string | null {
  if (!isRecord(metadata)) return null;
  for (const key of ["path", "filePath", "filename"]) {
    const value = metadata[key];
    if (typeof value === "string" && value.trim()) return value.trim();
  }
  for (const key of ["toolResultMetadata", "artifact"]) {
    const nestedPath = pathFromMetadata(metadata[key]);
    if (nestedPath) return nestedPath;
  }
  return null;
}

function sourceIcon(label: string, contentType: string): LucideIcon {
  const value = `${label} ${contentType}`.toLocaleLowerCase();
  if (/image|\.png\b|\.jpe?g\b|\.gif\b|\.webp\b/.test(value)) {
    return FileImage;
  }
  if (
    /json|javascript|typescript|rust|python|\.tsx?\b|\.jsx?\b|\.rs\b|\.py\b/.test(
      value,
    )
  ) {
    return FileCode2;
  }
  if (/text|markdown|\.md\b|\.txt\b|\.log\b/.test(value)) return FileText;
  if (/artifact|output|package/.test(value)) return Package;
  return Folder;
}

function isSubagentTool(name: string): boolean {
  const normalized = name.toLocaleLowerCase().replace(/[.-]/g, "_");
  return /(^|_)(sub_?agent|spawn_agent|create_agent|run_agent|delegate_task)(_|$)/.test(
    normalized,
  );
}

function subagentLabel(input: unknown): string | null {
  if (!isRecord(input)) return null;
  for (const key of ["agentName", "agent_name", "name", "title", "role"]) {
    const value = input[key];
    if (typeof value === "string" && value.trim()) return value.trim();
  }
  return null;
}

function toolResultFailed(metadata: unknown): boolean {
  if (!isRecord(metadata)) return false;
  return metadata.success === false || metadata.isError === true;
}

function executableName(command: string): string {
  const segments = command.replace(/[\\/]+$/, "").split(/[\\/]/);
  return segments.at(-1) || command;
}

function normalizePath(path: string): string {
  const normalized = path.trim().replace(/\\/g, "/").replace(/\/+/g, "/");
  return /^[a-z]:\//i.test(normalized)
    ? normalized.toLocaleLowerCase()
    : normalized;
}

function shortId(id: string): string {
  return id.slice(0, 8);
}

function timestamp(value: string): number {
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : 0;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}
