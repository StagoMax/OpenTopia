import { useEffect, useState, type ReactNode } from "react";
import {
  Archive,
  ArrowLeft,
  ChevronDown,
  Cloud,
  FileCode2,
  Folder,
  FolderOpen,
  GitBranch,
  GitFork,
  Loader2,
  MoreHorizontal,
  PanelRight,
  PanelRightOpen,
  Pause,
  Pencil,
  SlidersHorizontal,
  Target,
  TerminalSquare,
  X,
  Zap,
} from "lucide-react";

import type { ToolTabKind } from "../../toolTabs";
import type { GoalSnapshot, GoalStatus, Thread } from "../../types";
import { conversationHeaderTitle } from "../../threadTitle";
import { useDismissiblePopover } from "../../hooks/useDismissiblePopover";
import { Button, IconButton } from "../../components/ui";
import { ConversationLoadingIndicator } from "./ConversationLoadingIndicator";

export function ThreadHeader({
  thread,
  title,
  headingIcon,
  showThreadControls = true,
  onBack,
  backLabel = "返回",
  toolStageOpen,
  contextRailOpen,
  onOpenLocation,
  onOpenTool,
  onToggleContextRail,
  onToggleToolStage,
  onRename,
  onArchive,
}: {
  thread: Thread | null;
  title?: string;
  headingIcon?: ReactNode;
  showThreadControls?: boolean;
  onBack?: () => void;
  backLabel?: string;
  toolStageOpen: boolean;
  contextRailOpen: boolean;
  onOpenLocation(): void;
  onOpenTool(kind: ToolTabKind): void;
  onToggleContextRail(): void;
  onToggleToolStage(): void;
  onRename(): void;
  onArchive(): void;
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  const [taskMenuOpen, setTaskMenuOpen] = useState(false);
  const menuRef = useDismissiblePopover(menuOpen, () => setMenuOpen(false));
  const taskMenuRef = useDismissiblePopover(taskMenuOpen, () =>
    setTaskMenuOpen(false),
  );

  useEffect(() => {
    if (showThreadControls) return;
    setMenuOpen(false);
    setTaskMenuOpen(false);
  }, [showThreadControls]);

  function selectTool(kind: ToolTabKind) {
    onOpenTool(kind);
    setMenuOpen(false);
  }

  const headerTitle = conversationHeaderTitle(
    title ?? thread?.title ?? "新任务",
  );

  return (
    <div className="thread-header">
      <div className="thread-heading">
        {onBack ? (
          <IconButton
            aria-label={backLabel}
            onClick={onBack}
            size="compact"
            title={backLabel}
            variant="quiet"
          >
            <ArrowLeft aria-hidden="true" size={15} />
          </IconButton>
        ) : null}
        {headingIcon ?? <Folder size={15} />}
        <h1>{headerTitle}</h1>
        {showThreadControls ? (
          <div className="thread-heading-menu-wrap" ref={taskMenuRef}>
            <button
              className="thread-more"
              disabled={!thread}
              aria-label="任务菜单"
              aria-expanded={taskMenuOpen}
              onClick={() => {
                setTaskMenuOpen((current) => !current);
                setMenuOpen(false);
              }}
            >
              <MoreHorizontal size={15} />
            </button>
            {taskMenuOpen && thread && (
              <div className="tool-popover thread-heading-popover" role="menu">
                <button
                  role="menuitem"
                  onClick={() => {
                    onOpenLocation();
                    setTaskMenuOpen(false);
                  }}
                >
                  <FolderOpen size={14} />
                  <span>在文件管理器中打开</span>
                </button>
                <button
                  role="menuitem"
                  onClick={() => {
                    onRename();
                    setTaskMenuOpen(false);
                  }}
                >
                  <Pencil size={14} />
                  <span>重命名任务</span>
                </button>
                <button disabled title="Git 工作树管理尚未实现">
                  <GitFork size={14} />
                  <span>创建工作树</span>
                  <small>未实现</small>
                </button>
                <button
                  role="menuitem"
                  onClick={() => {
                    onArchive();
                    setTaskMenuOpen(false);
                  }}
                >
                  <Archive size={14} />
                  <span>归档任务</span>
                </button>
              </div>
            )}
          </div>
        ) : null}
      </div>
      {showThreadControls ? (
        <div className="thread-actions">
          <div className="thread-tool-menu-wrap" ref={menuRef}>
            <button
              className="thread-tool-button"
              disabled={!thread}
              aria-expanded={menuOpen}
              aria-haspopup="menu"
              onClick={() => {
                setMenuOpen((current) => !current);
                setTaskMenuOpen(false);
              }}
            >
              <PanelRight size={14} />
              <span>打开位置</span>
              <ChevronDown size={12} />
            </button>
            {menuOpen && thread && (
              <div className="tool-popover thread-tool-popover" role="menu">
                <button
                  role="menuitem"
                  onClick={() => {
                    onOpenLocation();
                    setMenuOpen(false);
                  }}
                >
                  <FolderOpen size={14} />
                  <span>文件管理器</span>
                </button>
                <button role="menuitem" onClick={() => selectTool("terminal")}>
                  <TerminalSquare size={14} />
                  <span>终端</span>
                </button>
                <button disabled title="VS Code 启动集成尚未实现">
                  <FileCode2 size={14} />
                  <span>VS Code</span>
                  <small>未实现</small>
                </button>
                <button disabled title="Git Bash 启动集成尚未实现">
                  <GitBranch size={14} />
                  <span>Git Bash</span>
                  <small>未实现</small>
                </button>
                <button disabled title="WSL 启动集成尚未实现">
                  <Cloud size={14} />
                  <span>WSL</span>
                  <small>未实现</small>
                </button>
                <div className="tool-popover-separator" />
                <button role="menuitem" onClick={() => selectTool("files")}>
                  <Folder size={14} />
                  <span>文件工具</span>
                </button>
                <button role="menuitem" onClick={() => selectTool("diff")}>
                  <GitBranch size={14} />
                  <span>审查变更</span>
                </button>
              </div>
            )}
          </div>
          <IconButton
            className={`context-rail-toggle ${contextRailOpen ? "is-active" : ""}`}
            size="compact"
            variant="quiet"
            aria-label={contextRailOpen ? "折叠环境信息" : "展开环境信息"}
            aria-controls="workspace-context-rail"
            aria-expanded={contextRailOpen}
            disabled={!thread}
            title={contextRailOpen ? "折叠环境信息" : "展开环境信息"}
            onClick={onToggleContextRail}
          >
            <SlidersHorizontal size={15} aria-hidden="true" />
          </IconButton>
          {!toolStageOpen ? (
            <IconButton
              className="tool-stage-toggle"
              size="compact"
              variant="quiet"
              aria-label="展开工具窗口"
              aria-controls="workspace-right-panel"
              aria-expanded={false}
              title="展开工具窗口"
              onClick={onToggleToolStage}
            >
              <PanelRightOpen size={15} aria-hidden="true" />
            </IconButton>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

export function GoalStrip({
  snapshot,
  isRunning,
  action,
  onRun,
  onPause,
  onCancel,
}: {
  snapshot: GoalSnapshot;
  isRunning: boolean;
  action: GoalStatus | "run" | null;
  onRun(): void;
  onPause(): void;
  onCancel(): void;
}) {
  const items = snapshot.workForm.items;
  const status = snapshot.workForm.status;
  const completed = items.filter((item) => item.status === "completed").length;
  const resolved = items.filter((item) =>
    ["completed", "deferred", "blocked", "cancelled"].includes(item.status),
  ).length;
  const total = items.length;
  const progress = total ? Math.round((completed / total) * 100) : 0;
  const succeededIds = new Set(
    items.filter((item) => item.status === "completed").map((item) => item.id),
  );
  let currentTaskIndex = items.findIndex(
    (item) => item.status === "in_progress",
  );
  if (currentTaskIndex < 0) {
    currentTaskIndex = items.findIndex(
      (item) =>
        item.status === "pending" &&
        item.dependsOn.every((dependency) => succeededIds.has(dependency)),
    );
  }
  const terminal = ["completed", "cancelled"].includes(status);
  const canRun = !isRunning && ["active", "paused", "blocked"].includes(status);
  return (
    <section className={`goal-strip is-${status}`}>
      <details open>
        <summary>
          <span className="goal-strip-icon" aria-hidden="true">
            <Target size={15} />
          </span>
          <span className="goal-strip-objective">
            {snapshot.workForm.objective}
          </span>
          <span className={`goal-status is-${status}`}>
            {goalStatusLabel(status)}
          </span>
          {total ? (
            <span className="goal-count">
              {currentTaskIndex >= 0
                ? `第 ${currentTaskIndex + 1}/${total} 步`
                : `${resolved}/${total} 已处理`}
            </span>
          ) : null}
        </summary>
        <div className="goal-strip-body">
          <div
            className="goal-progress"
            role="progressbar"
            aria-label="目标进度"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={progress}
          >
            <span style={{ width: `${progress}%` }} />
          </div>
          {items.length ? (
            <ol className="goal-task-list">
              {items.map((item) => (
                <li className={`is-${item.status}`} key={item.id}>
                  <span className="goal-task-state" aria-hidden="true" />
                  <span className="goal-task-content">
                    <span>{item.title}</span>
                    {item.note ? <small>{item.note}</small> : null}
                  </span>
                </li>
              ))}
            </ol>
          ) : null}
          {!terminal ? (
            <div className="goal-actions">
              {canRun ? (
                <button
                  type="button"
                  disabled={Boolean(action)}
                  onClick={onRun}
                >
                  {action === "run" ? (
                    <Loader2 size={14} className="spin" />
                  ) : (
                    <Zap size={14} />
                  )}
                  <span>继续</span>
                </button>
              ) : null}
              {status === "active" && isRunning ? (
                <button
                  type="button"
                  disabled={Boolean(action)}
                  onClick={onPause}
                >
                  {action === "paused" ? (
                    <Loader2 size={14} className="spin" />
                  ) : (
                    <Pause size={14} />
                  )}
                  <span>暂停</span>
                </button>
              ) : null}
              <button
                className="goal-cancel-button"
                type="button"
                title="取消目标"
                aria-label="取消目标"
                disabled={Boolean(action)}
                onClick={onCancel}
              >
                {action === "cancelled" ? (
                  <Loader2 size={14} className="spin" />
                ) : (
                  <X size={14} />
                )}
              </button>
            </div>
          ) : null}
        </div>
      </details>
    </section>
  );
}

function goalStatusLabel(status: GoalStatus): string {
  const labels: Record<GoalStatus, string> = {
    active: "执行中",
    paused: "已暂停",
    completed: "已完成",
    blocked: "受阻",
    cancelled: "已取消",
  };
  return labels[status];
}

export function ConversationLoadingState() {
  return (
    <section className="conversation-loading">
      <ConversationLoadingIndicator label="正在加载会话内容" />
    </section>
  );
}

export function ConversationLoadErrorState({
  error,
  onRetry,
}: {
  error: string;
  onRetry(): void;
}) {
  return (
    <section className="conversation-load-error" role="alert">
      <div className="conversation-load-error__content">
        <strong>无法加载会话内容</strong>
        <p>{error}</p>
        <Button variant="secondary" size="compact" onClick={onRetry}>
          重试
        </Button>
      </div>
    </section>
  );
}
