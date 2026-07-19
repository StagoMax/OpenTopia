import { useCallback, useEffect, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import {
  AlertCircle,
  Bot,
  Check,
  ChevronDown,
  ChevronRight,
  CircleDot,
  File,
  FileCode2,
  FileImage,
  FileText,
  Folder,
  GitBranch,
  GitCommitHorizontal,
  Laptop,
  Package,
  Plus,
  RefreshCw,
  SquareTerminal,
  UploadCloud,
  X,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import type { ApiClient } from "../api/client";
import type {
  AgentEvent,
  ArtifactDescriptor,
  GitStatusSummary,
  GitWorkflowAction,
  GitWorkflowResponse,
  Message,
  TerminalEvent,
  TerminalSession,
  WorkspaceDiff,
  SubagentRun,
} from "../types";
import "../styles/right-context-rail.css";

export type RightContextRailProps = {
  client: ApiClient | null;
  threadId: string | null;
  workspaceRoot: string | null;
  workspaceDiff: WorkspaceDiff | null;
  terminalEvents: TerminalEvent[];
  terminalSession: TerminalSession | null;
  agentEvents: AgentEvent[];
  subagentRuns: SubagentRun[];
  artifacts: ArtifactDescriptor[];
  messages: Message[];
  onOpenDiff(): void;
  onOpenTerminal(): void;
  onOpenFiles(): void;
  onAddSource(): void;
  onSpawnSubagent(name: string, input: string): Promise<void>;
  onCancelSubagent(runId: string): void;
  onGitChanged(): void;
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
  status: "排队中" | "运行中" | "已返回" | "失败" | "已取消" | "已超时";
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

type GitRepositoryState = "unknown" | "ready" | "missing";

const SOURCE_LIMIT = 4;

export function RightContextRail({
  client,
  threadId,
  workspaceRoot,
  workspaceDiff,
  terminalEvents,
  terminalSession,
  agentEvents,
  subagentRuns,
  artifacts,
  messages,
  onOpenDiff,
  onOpenTerminal,
  onOpenFiles,
  onAddSource,
  onSpawnSubagent,
  onCancelSubagent,
  onGitChanged,
}: RightContextRailProps) {
  const [subagentDialogOpen, setSubagentDialogOpen] = useState(false);
  const [subagentName, setSubagentName] = useState("worker");
  const [subagentInput, setSubagentInput] = useState("");
  const [isSpawningSubagent, setIsSpawningSubagent] = useState(false);
  const [subagentError, setSubagentError] = useState<string | null>(null);
  const [gitStatus, setGitStatus] = useState<GitStatusSummary | null>(null);
  const [gitRepositoryState, setGitRepositoryState] =
    useState<GitRepositoryState>("unknown");
  const [gitLoading, setGitLoading] = useState(false);
  const [gitBusy, setGitBusy] = useState<GitWorkflowAction["type"] | null>(
    null,
  );
  const [gitError, setGitError] = useState<string | null>(null);
  const [gitNotice, setGitNotice] = useState<string | null>(null);
  const [gitDialogOpen, setGitDialogOpen] = useState(false);
  const diffStats = countDiffLines(workspaceDiff?.diff ?? "");
  const branch =
    gitStatus?.branch ??
    (gitStatus?.detached ? "detached HEAD" : null) ??
    workspaceDiff?.branch?.trim() ??
    "HEAD";
  const activeProcesses = collectActiveProcesses(
    terminalSession,
    terminalEvents,
  );
  const subagents = collectSubagents(subagentRuns, agentEvents);
  const allSources = collectSources(messages, agentEvents, artifacts);
  const sources = allSources.slice(0, SOURCE_LIMIT);
  const gitAvailable = gitRepositoryState === "ready" && Boolean(gitStatus);

  const refreshGit = useCallback(async () => {
    if (!client || !threadId || !workspaceRoot) {
      setGitStatus(null);
      setGitRepositoryState("unknown");
      setGitError(null);
      return;
    }
    setGitRepositoryState("unknown");
    setGitLoading(true);
    setGitError(null);
    try {
      const status = await client.getGitStatus(threadId);
      setGitStatus(status);
      setGitRepositoryState("ready");
    } catch (error) {
      setGitStatus(null);
      const message = readableError(error);
      if (isNotGitRepositoryError(message)) {
        setGitRepositoryState("missing");
        setGitError(null);
      } else {
        setGitRepositoryState("unknown");
        setGitError(message);
      }
    } finally {
      setGitLoading(false);
    }
  }, [client, threadId, workspaceRoot]);

  useEffect(() => {
    void refreshGit();
  }, [refreshGit]);

  async function runGitAction(
    action: GitWorkflowAction,
    successMessage: string,
  ): Promise<GitWorkflowResponse | null> {
    if (!client || !threadId || gitBusy) return null;
    setGitBusy(action.type);
    setGitError(null);
    setGitNotice(null);
    try {
      const result = await client.runGitWorkflow(threadId, action);
      setGitNotice(successMessage);
      await refreshGit();
      onGitChanged();
      return result;
    } catch (error) {
      setGitError(readableError(error));
      return null;
    } finally {
      setGitBusy(null);
    }
  }

  function openGitDialog() {
    setGitDialogOpen(true);
    setGitError(null);
    setGitNotice(null);
  }

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
        {gitAvailable && (
          <>
            <RailRow
              icon={GitBranch}
              label="分支"
              title={gitStatusTitle(gitStatus, workspaceRoot)}
              value={
                <StatusText muted={!gitStatus?.branch}>{branch}</StatusText>
              }
            />
            <RailRow
              icon={GitCommitHorizontal}
              label="提交与推送"
              value={
                <StatusText muted={!gitStatus?.ahead}>
                  {gitStatus?.ahead ? `待推送 ${gitStatus.ahead}` : ""}
                </StatusText>
              }
              title="提交当前改动或推送当前分支"
              onClick={openGitDialog}
            />
          </>
        )}
        {gitError && (
          <button
            className="right-context-rail__git-error"
            type="button"
            title={gitError}
            onClick={() => void refreshGit()}
          >
            <AlertCircle size={13} aria-hidden="true" />
            <span>{gitError}</span>
            <RefreshCw size={12} aria-hidden="true" />
          </button>
        )}
      </RailSection>

      {gitDialogOpen && gitStatus &&
        createPortal(
          <GitCommitDialog
            status={gitStatus}
            diffStats={diffStats}
            changedFiles={workspaceDiff?.files.length ?? gitStatus.changed}
            busy={gitBusy}
            error={gitError}
            notice={gitNotice}
            onClose={() => setGitDialogOpen(false)}
            onCommit={async (message, allTracked, pushAfterCommit) => {
              const result = await runGitAction(
                { type: "commit", request: { message, allTracked } },
                "提交已创建",
              );
              if (!result?.success) return false;
              if (pushAfterCommit) {
                const pushResult = await runGitAction(
                  {
                    type: "push",
                    request: {
                      remote: "origin",
                      branch: gitStatus.branch ?? "HEAD",
                      setUpstream: !gitStatus.upstream,
                    },
                  },
                  `已推送 ${gitStatus.branch ?? "当前分支"}`,
                );
                return Boolean(pushResult?.success);
              }
              return true;
            }}
            generateCommitMessage={() =>
              `更新 ${Math.max(
                workspaceDiff?.files.length ?? gitStatus.changed,
                1,
              )} 个文件`
            }
            onPush={async () => {
              const result = await runGitAction(
                {
                  type: "push",
                  request: {
                    remote: "origin",
                    branch: gitStatus.branch ?? "HEAD",
                    setUpstream: !gitStatus.upstream,
                  },
                },
                `已推送 ${gitStatus.branch ?? "当前分支"}`,
              );
              return Boolean(result?.success);
            }}
          />,
          document.body,
        )}

      <RailSection
        title={`子智能体 ${subagents.length}`}
        action={
          <button
            className="right-context-rail__header-action"
            type="button"
            title="启动子智能体"
            aria-label="启动子智能体"
            onClick={() => setSubagentDialogOpen(true)}
          >
            <Plus size={14} aria-hidden="true" />
          </button>
        }
      >
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
              onClick={
                agent.status === "运行中" || agent.status === "排队中"
                  ? () => onCancelSubagent(agent.id)
                  : undefined
              }
            />
          ))
        ) : (
          <EmptyRow icon={Bot} label="暂无" />
        )}
      </RailSection>
      {subagentDialogOpen &&
        createPortal(
          <div
            className="right-context-rail__dialog-backdrop"
            role="presentation"
            onClick={() => setSubagentDialogOpen(false)}
          >
            <form
              className="right-context-rail__dialog"
              role="dialog"
              aria-modal="true"
              aria-label="启动子智能体"
              onClick={(event) => event.stopPropagation()}
              onSubmit={(event) => {
                event.preventDefault();
                if (
                  !subagentName.trim() ||
                  !subagentInput.trim() ||
                  isSpawningSubagent
                )
                  return;
                setIsSpawningSubagent(true);
                setSubagentError(null);
                void onSpawnSubagent(subagentName.trim(), subagentInput.trim())
                  .then(() => {
                    setSubagentDialogOpen(false);
                    setSubagentInput("");
                  })
                  .catch((error) =>
                    setSubagentError(
                      error instanceof Error ? error.message : String(error),
                    ),
                  )
                  .finally(() => setIsSpawningSubagent(false));
              }}
            >
              <header>
                <strong>启动子智能体</strong>
                <span>继承当前工作区、沙箱、权限和模型</span>
              </header>
              <label>
                名称
                <input
                  value={subagentName}
                  maxLength={64}
                  pattern="[a-z0-9_]+"
                  title="使用小写字母、数字和下划线"
                  onChange={(event) => setSubagentName(event.target.value)}
                />
              </label>
              <label>
                任务
                <textarea
                  value={subagentInput}
                  rows={5}
                  onChange={(event) => setSubagentInput(event.target.value)}
                />
              </label>
              {subagentError && <p>{subagentError}</p>}
              <footer>
                <button
                  type="button"
                  onClick={() => setSubagentDialogOpen(false)}
                >
                  取消
                </button>
                <button
                  type="submit"
                  disabled={
                    !subagentName.trim() ||
                    !subagentInput.trim() ||
                    isSpawningSubagent
                  }
                >
                  {isSpawningSubagent ? "启动中..." : "启动"}
                </button>
              </footer>
            </form>
          </div>,
          document.body,
        )}

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
            title="添加来源"
            aria-label="添加来源"
            onClick={onAddSource}
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
          disabled={allSources.length === 0}
          title={
            allSources.length
              ? `查看全部 ${allSources.length} 个来源`
              : "暂无来源"
          }
          onClick={onOpenFiles}
        >
          <span>查看全部</span>
          <ChevronRight size={13} aria-hidden="true" />
        </button>
      </RailSection>
    </div>
  );
}

function GitCommitDialog({
  status,
  diffStats,
  changedFiles,
  busy,
  error,
  notice,
  onClose,
  generateCommitMessage,
  onCommit,
  onPush,
}: {
  status: GitStatusSummary;
  diffStats: { additions: number; deletions: number };
  changedFiles: number;
  busy: GitWorkflowAction["type"] | null;
  error: string | null;
  notice: string | null;
  onClose(): void;
  generateCommitMessage(): string;
  onCommit(
    message: string,
    allTracked: boolean,
    pushAfterCommit: boolean,
  ): Promise<boolean>;
  onPush(): Promise<boolean>;
}) {
  const [commitMessage, setCommitMessage] = useState("");
  const [allTracked, setAllTracked] = useState(true);
  const busyNow = Boolean(busy);
  const hasChanges = status.changed > 0 || changedFiles > 0;

  useEffect(() => {
    function closeOnEscape(event: KeyboardEvent) {
      if (event.key === "Escape" && !busyNow) onClose();
    }
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [busyNow, onClose]);

  async function commit(pushAfterCommit: boolean) {
    if (busyNow || !hasChanges) return;
    const message = commitMessage.trim() || generateCommitMessage();
    const succeeded = await onCommit(message, allTracked, pushAfterCommit);
    if (succeeded) setCommitMessage("");
  }

  return (
    <div
      className="right-context-rail__dialog-backdrop"
      role="presentation"
      onClick={() => !busyNow && onClose()}
    >
      <section
        className="right-context-rail__commit-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="git-commit-title"
        onClick={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
            event.preventDefault();
            void commit(false);
          }
        }}
      >
        <header className="right-context-rail__commit-header">
          <span className="right-context-rail__commit-branch">
            <GitBranch size={15} aria-hidden="true" />
            <strong id="git-commit-title">
              {status.branch ?? (status.detached ? "detached HEAD" : "HEAD")}
            </strong>
            <ChevronDown size={13} aria-hidden="true" />
          </span>
          <span className="right-context-rail__commit-stats">
            <span className="is-addition">+{diffStats.additions}</span>
            <span className="is-deletion">-{diffStats.deletions}</span>
          </span>
          <button
            className="right-context-rail__icon-button"
            type="button"
            title="关闭"
            aria-label="关闭"
            disabled={busyNow}
            onClick={onClose}
          >
            <X size={15} aria-hidden="true" />
          </button>
        </header>

        <textarea
          autoFocus
          className="right-context-rail__commit-message"
          maxLength={32768}
          rows={4}
          value={commitMessage}
          placeholder="提交信息（留空将自动生成）…"
          aria-label="提交信息"
          onChange={(event) => setCommitMessage(event.target.value)}
        />

        <label className="right-context-rail__commit-checkbox">
          <input
            type="checkbox"
            checked={allTracked}
            disabled={busyNow}
            onChange={(event) => setAllTracked(event.target.checked)}
          />
          <Check size={13} aria-hidden="true" />
          <span>包含未暂存的更改</span>
        </label>

        {error && (
          <div className="right-context-rail__git-message is-error" role="alert">
            <AlertCircle size={14} aria-hidden="true" />
            <span>{error}</span>
          </div>
        )}
        {notice && !error && (
          <div className="right-context-rail__git-message is-success" role="status">
            <Check size={14} aria-hidden="true" />
            <span>{notice}</span>
          </div>
        )}

        <div className="right-context-rail__commit-actions">
          <button
            type="button"
            className="is-primary"
            disabled={busyNow || !hasChanges}
            onClick={() => void commit(false)}
          >
            <GitCommitHorizontal size={15} aria-hidden="true" />
            <span>{busy === "commit" ? "提交中…" : "提交"}</span>
            <kbd>Ctrl+↵</kbd>
          </button>
          <button
            type="button"
            disabled={busyNow || !hasChanges}
            onClick={() => void commit(true)}
          >
            <UploadCloud size={15} aria-hidden="true" />
            <span>提交并推送</span>
          </button>
          <button
            type="button"
            disabled={busyNow || status.ahead === 0}
            onClick={() => void onPush()}
          >
            <UploadCloud size={15} aria-hidden="true" />
            <span>{busy === "push" ? "推送中…" : "推送"}</span>
          </button>
        </div>
      </section>
    </div>
  );
}

function gitStatusTitle(
  status: GitStatusSummary | null,
  workspaceRoot: string | null,
): string {
  if (!status) return workspaceRoot ? "Git 状态不可用" : "暂无工作区";
  const branch = status.branch ?? (status.detached ? "detached HEAD" : "未知分支");
  const tracking = status.upstream
    ? `${status.upstream} · 领先 ${status.ahead} / 落后 ${status.behind}`
    : "未设置 upstream";
  return `${branch} · ${tracking} · ${status.changed} 个改动`;
}

function readableError(error: unknown): string {
  const raw = error instanceof Error ? error.message : String(error);
  try {
    const parsed = JSON.parse(raw) as { error?: unknown; message?: unknown };
    if (typeof parsed.error === "string") return parsed.error;
    if (typeof parsed.message === "string") return parsed.message;
  } catch {
    // The backend can also return a plain-text Git error.
  }
  return raw || "Git 操作失败";
}

function isNotGitRepositoryError(message: string): boolean {
  const normalized = message.toLocaleLowerCase();
  return (
    normalized.includes("not a git repository") ||
    normalized.includes("不是 git 仓库") ||
    normalized.includes("并非 git 仓库")
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

function collectSubagents(
  runs: SubagentRun[],
  events: AgentEvent[],
): SubagentItem[] {
  if (runs.length) {
    return runs.map((run) => ({
      id: run.id,
      label: `${run.agentPath} · ${run.agentType}`,
      status: subagentStatusLabel(run.status),
      createdAt: run.createdAt,
    }));
  }
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

function subagentStatusLabel(
  status: SubagentRun["status"],
): SubagentItem["status"] {
  switch (status) {
    case "queued":
      return "排队中";
    case "running":
      return "运行中";
    case "completed":
      return "已返回";
    case "cancelled":
      return "已取消";
    case "timed_out":
      return "已超时";
    default:
      return "失败";
  }
}

function collectSources(
  messages: Message[],
  events: AgentEvent[],
  artifacts: ArtifactDescriptor[],
): SourceItem[] {
  const candidates: SourceItem[] = [];

  for (const message of messages) {
    for (const part of message.parts) {
      if (part.type === "source_ref") {
        const source = part.source;
        candidates.push({
          id: `source:${source.id}`,
          label: source.name,
          title: `${source.path}\n${source.contentType}\n${formatSourceBytes(source.bytes)}${source.truncated ? " · 已截断" : ""}`,
          createdAt: message.createdAt,
          icon: sourceIcon(source.name, source.contentType),
          dedupeKey: `path:${normalizePath(source.path)}`,
        });
      } else if (part.type === "skill_ref") {
        candidates.push({
          id: `skill:${part.skill.id}`,
          label: part.skill.name,
          title: `${part.skill.description || "Skill"}\n${part.skill.path}`,
          createdAt: message.createdAt,
          icon: Package,
          dedupeKey: `skill:${part.skill.id}`,
        });
      }
    }
  }

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

function formatSourceBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let index = 0;
  while (value >= 1024 && index < units.length - 1) {
    value /= 1024;
    index += 1;
  }
  return `${value.toFixed(value >= 10 ? 0 : 1)} ${units[index]}`;
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
