import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
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
  GitCompareArrows,
  GitCommitHorizontal,
  Github,
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
  GitBranchInfo,
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
  onOpenExtensions(): void;
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

type GitDialogMode = "branches" | "commit" | "push" | "compare";

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
  onOpenExtensions,
  onAddSource,
  onSpawnSubagent,
  onCancelSubagent,
  onGitChanged,
}: RightContextRailProps) {
  const [subagentDialogOpen, setSubagentDialogOpen] = useState(false);
  const [subagentName, setSubagentName] = useState("Worker");
  const [subagentInput, setSubagentInput] = useState("");
  const [isSpawningSubagent, setIsSpawningSubagent] = useState(false);
  const [subagentError, setSubagentError] = useState<string | null>(null);
  const [gitStatus, setGitStatus] = useState<GitStatusSummary | null>(null);
  const [gitBranches, setGitBranches] = useState<GitBranchInfo[]>([]);
  const [gitLoading, setGitLoading] = useState(false);
  const [gitBusy, setGitBusy] = useState<GitWorkflowAction["type"] | null>(
    null,
  );
  const [gitError, setGitError] = useState<string | null>(null);
  const [gitNotice, setGitNotice] = useState<string | null>(null);
  const [gitDialog, setGitDialog] = useState<GitDialogMode | null>(null);
  const [compareResult, setCompareResult] =
    useState<GitWorkflowResponse | null>(null);
  const diffStats = countDiffLines(workspaceDiff?.diff ?? "");
  const branch =
    gitStatus?.branch ??
    (gitStatus?.detached ? "detached HEAD" : null) ??
    workspaceDiff?.branch?.trim() ??
    "非 Git 仓库";
  const activeProcesses = collectActiveProcesses(
    terminalSession,
    terminalEvents,
  );
  const subagents = collectSubagents(subagentRuns, agentEvents);
  const allSources = collectSources(messages, agentEvents, artifacts);
  const sources = allSources.slice(0, SOURCE_LIMIT);
  const gitAvailable = Boolean(client && threadId && workspaceRoot);

  const refreshGit = useCallback(async () => {
    if (!client || !threadId || !workspaceRoot) {
      setGitStatus(null);
      setGitBranches([]);
      setGitError(null);
      return;
    }
    setGitLoading(true);
    setGitError(null);
    try {
      const [status, branches] = await Promise.all([
        client.getGitStatus(threadId),
        client.listGitBranches(threadId),
      ]);
      setGitStatus(status);
      setGitBranches(branches);
    } catch (error) {
      setGitStatus(null);
      setGitBranches([]);
      setGitError(readableError(error));
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

  function openGitDialog(mode: GitDialogMode) {
    setGitDialog(mode);
    setGitError(null);
    setGitNotice(null);
    if (mode !== "compare") setCompareResult(null);
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
        <RailRow
          icon={GitBranch}
          label="分支"
          title={gitStatusTitle(gitStatus, workspaceRoot)}
          disabled={!gitAvailable || gitLoading}
          onClick={() => openGitDialog("branches")}
          value={
            <span className="right-context-rail__inline-value">
              <StatusText muted={!gitStatus?.branch && !workspaceDiff?.branch}>
                {gitLoading ? "读取中" : branch}
              </StatusText>
              {gitAvailable && !gitLoading && (
                <ChevronDown size={13} aria-hidden="true" />
              )}
            </span>
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
          title="创建本地提交或推送当前分支"
          disabled={!gitAvailable || gitLoading}
          onClick={() => openGitDialog("commit")}
        />
        <RailRow
          icon={Github}
          label="GitHub CLI"
          value={<StatusText muted>不可用</StatusText>}
          title="GitHub CLI 不可用；打开扩展"
          onClick={onOpenExtensions}
        />
        <RailRow
          icon={GitCompareArrows}
          label="比较分支"
          value={<StatusText muted>Diff</StatusText>}
          title="比较两个 Git 引用"
          disabled={!gitAvailable || gitLoading}
          onClick={() => openGitDialog("compare")}
        />
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

      {gitDialog &&
        createPortal(
          <GitWorkflowDialog
            mode={gitDialog}
            status={gitStatus}
            branches={gitBranches}
            busy={gitBusy}
            error={gitError}
            notice={gitNotice}
            compareResult={compareResult}
            onClose={() => setGitDialog(null)}
            onRefresh={() => void refreshGit()}
            onChangeMode={openGitDialog}
            onCreateBranch={async (newBranch, startPoint) => {
              await runGitAction(
                {
                  type: "create_branch",
                  request: { branch: newBranch, startPoint: startPoint || null },
                },
                `已创建分支 ${newBranch}`,
              );
            }}
            onSwitchBranch={async (nextBranch) => {
              if (
                !window.confirm(
                  `确认切换到分支“${nextBranch}”？\n\n此操作会修改当前工作区文件；Git 会在存在冲突时拒绝切换。`,
                )
              )
                return;
              await runGitAction(
                { type: "switch_branch", request: { branch: nextBranch } },
                `已切换到 ${nextBranch}`,
              );
            }}
            onCommit={async (message, allTracked) => {
              if (
                !window.confirm(
                  `确认创建本地提交？\n\n提交信息：${message}${allTracked ? "\n将同时包含所有已跟踪文件的改动。" : ""}`,
                )
              )
                return;
              await runGitAction(
                { type: "commit", request: { message, allTracked } },
                "提交已创建",
              );
            }}
            onPush={async (remote, pushBranch, setUpstream) => {
              if (
                !window.confirm(
                  `确认推送 ${pushBranch} 到 ${remote}？${setUpstream ? "\n同时设置 upstream 跟踪关系。" : ""}\n\n此操作会更新远程仓库。`,
                )
              )
                return;
              await runGitAction(
                {
                  type: "push",
                  request: {
                    remote,
                    branch: pushBranch,
                    setUpstream,
                  },
                },
                `已推送 ${pushBranch} 到 ${remote}`,
              );
            }}
            onCompare={async (base, head, mode) => {
              setCompareResult(null);
              const result = await runGitAction(
                { type: "compare", request: { base, head, mode } },
                `已比较 ${base} 与 ${head}`,
              );
              setCompareResult(result);
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
                  maxLength={80}
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

function GitWorkflowDialog({
  mode,
  status,
  branches,
  busy,
  error,
  notice,
  compareResult,
  onClose,
  onRefresh,
  onChangeMode,
  onCreateBranch,
  onSwitchBranch,
  onCommit,
  onPush,
  onCompare,
}: {
  mode: GitDialogMode;
  status: GitStatusSummary | null;
  branches: GitBranchInfo[];
  busy: GitWorkflowAction["type"] | null;
  error: string | null;
  notice: string | null;
  compareResult: GitWorkflowResponse | null;
  onClose(): void;
  onRefresh(): void;
  onChangeMode(mode: GitDialogMode): void;
  onCreateBranch(branch: string, startPoint: string): Promise<void>;
  onSwitchBranch(branch: string): Promise<void>;
  onCommit(message: string, allTracked: boolean): Promise<void>;
  onPush(
    remote: string,
    branch: string,
    setUpstream: boolean,
  ): Promise<void>;
  onCompare(
    base: string,
    head: string,
    mode: "direct" | "merge_base",
  ): Promise<void>;
}) {
  const localBranches = useMemo(
    () => branches.filter((branch) => !branch.remote),
    [branches],
  );
  const remoteBranches = useMemo(
    () => branches.filter((branch) => branch.remote && !branch.symbolicTarget),
    [branches],
  );
  const currentBranch = status?.branch ?? "";
  const defaultBase =
    localBranches.find(
      (branch) =>
        branch.name !== currentBranch && ["main", "master"].includes(branch.name),
    )?.name ??
    localBranches.find((branch) => branch.name !== currentBranch)?.name ??
    remoteBranches.find((branch) => branch.name !== currentBranch)?.name ??
    "main";
  const [newBranch, setNewBranch] = useState("");
  const [startPoint, setStartPoint] = useState("");
  const [commitMessage, setCommitMessage] = useState("");
  const [allTracked, setAllTracked] = useState(false);
  const [remote, setRemote] = useState("origin");
  const [pushBranch, setPushBranch] = useState(currentBranch);
  const [setUpstream, setSetUpstream] = useState(!status?.upstream);
  const [compareBase, setCompareBase] = useState(defaultBase);
  const [compareHead, setCompareHead] = useState(currentBranch || "HEAD");
  const [compareMode, setCompareMode] = useState<"direct" | "merge_base">(
    "merge_base",
  );

  useEffect(() => {
    if (currentBranch) {
      setPushBranch((value) => value || currentBranch);
      setCompareHead((value) => (value === "HEAD" ? currentBranch : value));
    }
  }, [currentBranch]);

  useEffect(() => {
    setCompareBase((value) =>
      !value || value === currentBranch ? defaultBase : value,
    );
  }, [currentBranch, defaultBase]);

  useEffect(() => {
    function closeOnEscape(event: KeyboardEvent) {
      if (event.key === "Escape" && !busy) onClose();
    }
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [busy, onClose]);

  const title = {
    branches: "分支管理",
    commit: "提交改动",
    push: "推送分支",
    compare: "比较分支",
  }[mode];

  return (
    <div
      className="right-context-rail__dialog-backdrop"
      role="presentation"
      onClick={() => !busy && onClose()}
    >
      <section
        className="right-context-rail__dialog right-context-rail__git-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="git-workflow-title"
        onClick={(event) => event.stopPropagation()}
      >
        <header className="right-context-rail__git-dialog-header">
          <span>
            <strong id="git-workflow-title">{title}</strong>
            <small>{gitStatusTitle(status, null)}</small>
          </span>
          <span className="right-context-rail__git-dialog-actions">
            <button
              type="button"
              className="right-context-rail__icon-button"
              title="刷新 Git 状态"
              aria-label="刷新 Git 状态"
              disabled={Boolean(busy)}
              onClick={onRefresh}
            >
              <RefreshCw size={14} aria-hidden="true" />
            </button>
            <button
              type="button"
              className="right-context-rail__icon-button"
              title="关闭"
              aria-label="关闭"
              disabled={Boolean(busy)}
              onClick={onClose}
            >
              <X size={14} aria-hidden="true" />
            </button>
          </span>
        </header>

        <nav className="right-context-rail__git-tabs" aria-label="Git 工作流">
          {(
            ["branches", "commit", "push", "compare"] as GitDialogMode[]
          ).map((item) => (
            <button
              key={item}
              type="button"
              className={item === mode ? "is-active" : ""}
              aria-current={item === mode ? "page" : undefined}
              disabled={Boolean(busy)}
              onClick={() => onChangeMode(item)}
            >
              {{
                branches: "分支",
                commit: "提交",
                push: "推送",
                compare: "比较",
              }[item]}
            </button>
          ))}
        </nav>

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

        {mode === "branches" && (
          <div className="right-context-rail__git-pane">
            <div className="right-context-rail__branch-list" role="list">
              {localBranches.length ? (
                localBranches.map((branch) => (
                  <div
                    className={`right-context-rail__branch-item ${branch.current ? "is-current" : ""}`}
                    key={branch.fullRef}
                    role="listitem"
                  >
                    <GitBranch size={14} aria-hidden="true" />
                    <span title={branch.fullRef}>
                      <strong>{branch.name}</strong>
                      <small>{branch.upstream || "仅本地"}</small>
                    </span>
                    {branch.current ? (
                      <span className="right-context-rail__branch-current">
                        <Check size={12} aria-hidden="true" /> 当前
                      </span>
                    ) : (
                      <button
                        type="button"
                        disabled={Boolean(busy)}
                        onClick={() => void onSwitchBranch(branch.name)}
                      >
                        切换
                      </button>
                    )}
                  </div>
                ))
              ) : (
                <p className="right-context-rail__git-empty">暂无本地分支</p>
              )}
              {remoteBranches.length > 0 && (
                <details className="right-context-rail__remote-branches">
                  <summary>远程分支 {remoteBranches.length}</summary>
                  {remoteBranches.map((branch) => (
                    <div key={branch.fullRef} title={branch.fullRef}>
                      {branch.name}
                    </div>
                  ))}
                </details>
              )}
            </div>
            <form
              className="right-context-rail__git-form is-inline"
              onSubmit={(event) => {
                event.preventDefault();
                if (!newBranch.trim() || busy) return;
                void onCreateBranch(newBranch.trim(), startPoint.trim());
              }}
            >
              <label>
                新分支名称
                <input
                  autoComplete="off"
                  value={newBranch}
                  placeholder="feature/my-change"
                  onChange={(event) => setNewBranch(event.target.value)}
                />
              </label>
              <label>
                起点（可选）
                <input
                  autoComplete="off"
                  value={startPoint}
                  placeholder={currentBranch || "HEAD"}
                  list="git-branch-refs"
                  onChange={(event) => setStartPoint(event.target.value)}
                />
              </label>
              <button
                className="right-context-rail__primary-button"
                type="submit"
                disabled={!newBranch.trim() || Boolean(busy)}
              >
                {busy === "create_branch" ? "创建中..." : "创建分支"}
              </button>
            </form>
          </div>
        )}

        {mode === "commit" && (
          <form
            className="right-context-rail__git-pane right-context-rail__git-form"
            onSubmit={(event) => {
              event.preventDefault();
              if (!commitMessage.trim() || busy) return;
              void onCommit(commitMessage.trim(), allTracked);
            }}
          >
            <div className="right-context-rail__git-summary">
              <span>{status?.staged ?? 0} 个已暂存</span>
              <span>{status?.unstaged ?? 0} 个未暂存</span>
              <span>{status?.untracked ?? 0} 个未跟踪</span>
            </div>
            <label>
              提交信息
              <textarea
                autoFocus
                maxLength={32768}
                rows={5}
                value={commitMessage}
                placeholder="简要说明本次改动"
                onChange={(event) => setCommitMessage(event.target.value)}
              />
            </label>
            <label className="right-context-rail__checkbox-row">
              <input
                type="checkbox"
                checked={allTracked}
                onChange={(event) => setAllTracked(event.target.checked)}
              />
              <span>
                包含所有已跟踪文件改动
                <small>等同于 git commit --all；不会包含未跟踪文件</small>
              </span>
            </label>
            <footer>
              <button type="button" onClick={() => onChangeMode("push")}>
                前往推送
              </button>
              <button
                type="submit"
                disabled={!commitMessage.trim() || Boolean(busy)}
              >
                {busy === "commit" ? "提交中..." : "创建提交"}
              </button>
            </footer>
          </form>
        )}

        {mode === "push" && (
          <form
            className="right-context-rail__git-pane right-context-rail__git-form"
            onSubmit={(event) => {
              event.preventDefault();
              if (!remote.trim() || !pushBranch.trim() || busy) return;
              void onPush(remote.trim(), pushBranch.trim(), setUpstream);
            }}
          >
            <div className="right-context-rail__git-hero-icon">
              <UploadCloud size={20} aria-hidden="true" />
              <span>
                <strong>{status?.ahead ?? 0} 个本地提交待推送</strong>
                <small>{status?.upstream || "当前分支尚未设置 upstream"}</small>
              </span>
            </div>
            <div className="right-context-rail__git-field-grid">
              <label>
                远程仓库
                <input
                  autoFocus
                  autoComplete="off"
                  value={remote}
                  onChange={(event) => setRemote(event.target.value)}
                />
              </label>
              <label>
                分支
                <input
                  autoComplete="off"
                  value={pushBranch}
                  list="git-local-branch-refs"
                  onChange={(event) => setPushBranch(event.target.value)}
                />
              </label>
            </div>
            <label className="right-context-rail__checkbox-row">
              <input
                type="checkbox"
                checked={setUpstream}
                onChange={(event) => setSetUpstream(event.target.checked)}
              />
              <span>设置为当前分支的 upstream</span>
            </label>
            <footer>
              <button type="button" onClick={() => onChangeMode("commit")}>
                返回提交
              </button>
              <button
                type="submit"
                disabled={
                  !remote.trim() || !pushBranch.trim() || Boolean(busy)
                }
              >
                {busy === "push" ? "推送中..." : "确认并推送"}
              </button>
            </footer>
          </form>
        )}

        {mode === "compare" && (
          <form
            className="right-context-rail__git-pane right-context-rail__git-form"
            onSubmit={(event) => {
              event.preventDefault();
              if (!compareBase.trim() || !compareHead.trim() || busy) return;
              void onCompare(
                compareBase.trim(),
                compareHead.trim(),
                compareMode,
              );
            }}
          >
            <div className="right-context-rail__git-field-grid">
              <label>
                基准分支
                <input
                  autoFocus
                  autoComplete="off"
                  value={compareBase}
                  list="git-branch-refs"
                  onChange={(event) => setCompareBase(event.target.value)}
                />
              </label>
              <label>
                目标分支
                <input
                  autoComplete="off"
                  value={compareHead}
                  list="git-branch-refs"
                  onChange={(event) => setCompareHead(event.target.value)}
                />
              </label>
            </div>
            <div
              className="right-context-rail__segmented"
              role="group"
              aria-label="比较方式"
            >
              <button
                type="button"
                className={compareMode === "merge_base" ? "is-active" : ""}
                onClick={() => setCompareMode("merge_base")}
              >
                从共同祖先
              </button>
              <button
                type="button"
                className={compareMode === "direct" ? "is-active" : ""}
                onClick={() => setCompareMode("direct")}
              >
                直接比较
              </button>
            </div>
            <button
              className="right-context-rail__primary-button"
              type="submit"
              disabled={
                !compareBase.trim() || !compareHead.trim() || Boolean(busy)
              }
            >
              {busy === "compare" ? "比较中..." : "生成 Diff"}
            </button>
            {compareResult && (
              <div className="right-context-rail__compare-result">
                <header>
                  <strong>比较结果</strong>
                  <span>{compareResult.truncated ? "输出已截断" : "完整输出"}</span>
                </header>
                <pre>{compareResult.stdout || "两个引用之间没有文件差异。"}</pre>
              </div>
            )}
          </form>
        )}

        <datalist id="git-branch-refs">
          {branches.map((branch) => (
            <option key={branch.fullRef} value={branch.name} />
          ))}
          <option value="HEAD" />
        </datalist>
        <datalist id="git-local-branch-refs">
          {localBranches.map((branch) => (
            <option key={branch.fullRef} value={branch.name} />
          ))}
        </datalist>
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
      label: run.name,
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
