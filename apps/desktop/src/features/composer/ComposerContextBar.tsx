import {
  Check,
  ChevronDown,
  Cloud,
  Folder,
  FolderOpen,
  GitBranch,
  GitFork,
  Laptop,
  TerminalSquare,
} from "lucide-react";
import { formatPathForDisplay } from "../../pathDisplay";
import { workspaceName } from "../../workspaceName";
import type { AppSettings, Project } from "../../types";
import { sandboxModeLabel, sandboxModeOptions } from "./composerModes";
import {
  newTaskLaunchModeLabel,
  type ComposerOpenMenu,
  type NewTaskLaunchMode,
} from "./composerTypes";

export function ComposerContextBar({
  openMenu,
  workspaceRoot,
  projectName,
  projects,
  launchMode,
  sandboxMode,
  onToggleMenu,
  onCloseMenu,
  onPickWorkspace,
  onSelectProject,
  onChangeLaunchMode,
  onChangeSandboxMode,
}: {
  openMenu: ComposerOpenMenu;
  workspaceRoot: string | null;
  projectName: string | null;
  projects: Project[];
  launchMode?: NewTaskLaunchMode;
  sandboxMode: AppSettings["sandbox"]["sandboxMode"];
  onToggleMenu(menu: Exclude<ComposerOpenMenu, null>): void;
  onCloseMenu(): void;
  onPickWorkspace(): void;
  onSelectProject(projectId: string): void;
  onChangeLaunchMode?(mode: NewTaskLaunchMode): void;
  onChangeSandboxMode(mode: AppSettings["sandbox"]["sandboxMode"]): void;
}) {
  if (!workspaceRoot && !projectName) return null;

  return (
    <div className="composer-context">
      <div className="composer-menu-wrap">
        <button
          className="composer-context-button"
          type="button"
          title={
            workspaceRoot
              ? formatPathForDisplay(workspaceRoot)
              : (projectName ?? "项目")
          }
          aria-expanded={openMenu === "workspace"}
          onClick={() => onToggleMenu("workspace")}
        >
          <Folder size={12} />
          <span>{projectName ?? workspaceName(workspaceRoot ?? "")}</span>
          <ChevronDown size={11} />
        </button>
        {openMenu === "workspace" ? (
          <div className="tool-popover workspace-popover" role="menu">
            <div className="tool-popover-note">
              <strong>选择工作区</strong>
              <span>当前任务将使用所选文件夹</span>
            </div>
            {projects
              .filter((project) => project.workspaceRoot)
              .map((project) => (
                <button
                  key={project.id}
                  role="menuitemradio"
                  aria-checked={project.workspaceRoot === workspaceRoot}
                  onClick={() => {
                    onSelectProject(project.id);
                    onCloseMenu();
                  }}
                >
                  {project.workspaceRoot === workspaceRoot ? (
                    <Check size={13} />
                  ) : (
                    <Folder size={13} />
                  )}
                  <span>{project.name}</span>
                </button>
              ))}
            <div className="tool-popover-separator" />
            <button
              role="menuitem"
              onClick={() => {
                onPickWorkspace();
                onCloseMenu();
              }}
            >
              <FolderOpen size={14} />
              <span>选择其他文件夹</span>
            </button>
          </div>
        ) : null}
      </div>
      <div className="composer-menu-wrap">
        {launchMode && onChangeLaunchMode ? (
          <>
            <button
              className="composer-context-button"
              type="button"
              aria-label="选择启动模式"
              aria-expanded={openMenu === "environment"}
              onClick={() => onToggleMenu("environment")}
            >
              {launchMode === "local" ? (
                <Laptop size={12} />
              ) : (
                <GitFork size={12} />
              )}
              <span>{newTaskLaunchModeLabel(launchMode)}</span>
              <ChevronDown size={11} />
            </button>
            {openMenu === "environment" ? (
              <div className="tool-popover launch-mode-popover" role="menu">
                <div className="tool-popover-note">
                  <strong>启动模式</strong>
                  <span>选择新任务使用的工作区方式</span>
                </div>
                <button
                  className={launchMode === "local" ? "active" : ""}
                  role="menuitemradio"
                  aria-checked={launchMode === "local"}
                  onClick={() => {
                    onChangeLaunchMode("local");
                    onCloseMenu();
                  }}
                >
                  <Laptop size={14} />
                  <span>在本地处理</span>
                  {launchMode === "local" ? <Check size={13} /> : null}
                </button>
                <button
                  className={launchMode === "new_worktree" ? "active" : ""}
                  role="menuitemradio"
                  aria-checked={launchMode === "new_worktree"}
                  title="线程级工作树创建尚未接入"
                  onClick={() => {
                    onChangeLaunchMode("new_worktree");
                    onCloseMenu();
                  }}
                >
                  <GitFork size={14} />
                  <span>新工作树</span>
                  <small>内部未实现</small>
                </button>
                <button disabled role="menuitem" title="云端任务执行尚未实现">
                  <Cloud size={14} />
                  <span>发送至云端</span>
                  <small>未实现</small>
                </button>
              </div>
            ) : null}
          </>
        ) : (
          <>
            <button
              className="composer-context-button"
              type="button"
              aria-expanded={openMenu === "environment"}
              onClick={() => onToggleMenu("environment")}
            >
              <TerminalSquare size={12} />
              <span>{sandboxModeLabel(sandboxMode)}</span>
              <ChevronDown size={11} />
            </button>
            {openMenu === "environment" ? (
              <div className="tool-popover environment-popover" role="menu">
                {sandboxModeOptions.map((option) => (
                  <button
                    className={sandboxMode === option.value ? "active" : ""}
                    key={option.value}
                    role="menuitemradio"
                    aria-checked={sandboxMode === option.value}
                    onClick={() => {
                      onChangeSandboxMode(option.value);
                      onCloseMenu();
                    }}
                  >
                    {sandboxMode === option.value ? (
                      <Check size={13} />
                    ) : (
                      <span className="menu-icon-spacer" />
                    )}
                    <span>{option.label}</span>
                    <small>{option.detail}</small>
                  </button>
                ))}
                <div className="tool-popover-separator" />
                <button disabled title="Git 工作树创建尚未实现">
                  <GitFork size={14} />
                  <span>新工作树</span>
                  <small>未实现</small>
                </button>
                <button disabled title="远程执行环境尚未实现">
                  <Cloud size={14} />
                  <span>云环境</span>
                  <small>未实现</small>
                </button>
              </div>
            ) : null}
          </>
        )}
      </div>
      <button
        className="composer-context-button"
        type="button"
        disabled
        title="分支读取尚未实现"
      >
        <GitBranch size={12} />
        <span>分支未接入</span>
      </button>
    </div>
  );
}
