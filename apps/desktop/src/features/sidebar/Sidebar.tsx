import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import {
  Archive,
  Activity,
  Bot,
  BriefcaseBusiness,
  Cable,
  Check,
  ChevronDown,
  CircleHelp,
  Clock3,
  Code2,
  FileText,
  Folder,
  FolderOpen,
  GitFork,
  GitPullRequest,
  Inbox,
  Library,
  LayoutDashboard,
  Loader2,
  MessageCircle,
  MoreHorizontal,
  Pencil,
  Pin,
  Plug,
  Plus,
  Search,
  Settings,
  ShieldCheck,
  SquarePen,
  Workflow,
  X,
} from "lucide-react";

import { formatPathForDisplay } from "../../pathDisplay";
import {
  readSidebarNavigationState,
  updateSidebarNavigationState,
} from "../../workbenchPreferences";
import type { ThreadActivityStore } from "../../threadActivityStore";
import type { ExperienceMode, Project, Thread } from "../../types";
import type { FlowPrimaryView } from "../../workspaceNavigation";
import { useDismissiblePopover } from "../../hooks/useDismissiblePopover";
import { Button, IconButton } from "../../components/ui";
import { SidebarThreadRow } from "./SidebarThreadRow";
import { useApplicationLanguage } from "../../ApplicationLanguageProvider";

type ProjectHoverState = {
  id: string;
  name: string;
  threadCount: number;
  workspaceRoot: string | null;
  pinned: boolean;
  left: number;
  top: number;
};

const PROJECT_THREAD_PREVIEW_LIMIT = 10;

const FLOW_NAVIGATION_ITEMS = [
  {
    view: "overview",
    labelKey: "flow.nav.overview",
    descriptionKey: "flow.nav.overview.description",
    icon: LayoutDashboard,
  },
  {
    view: "inbox",
    labelKey: "flow.nav.inbox",
    descriptionKey: "flow.nav.inbox.description",
    icon: Inbox,
  },
  {
    view: "agents",
    labelKey: "flow.nav.agents",
    descriptionKey: "flow.nav.agents.description",
    icon: Bot,
  },
  {
    view: "workflow-templates",
    labelKey: "flow.nav.flows",
    descriptionKey: "flow.nav.flows.description",
    icon: Workflow,
  },
  {
    view: "runs",
    labelKey: "flow.nav.runs",
    descriptionKey: "flow.nav.runs.description",
    icon: Activity,
  },
  {
    view: "connections",
    labelKey: "flow.nav.connections",
    descriptionKey: "flow.nav.connections.description",
    icon: Cable,
  },
  {
    view: "trust",
    labelKey: "flow.nav.trust",
    descriptionKey: "flow.nav.trust.description",
    icon: ShieldCheck,
  },
  {
    view: "knowledge",
    labelKey: "flow.nav.knowledge",
    descriptionKey: "flow.nav.knowledge.description",
    icon: Library,
  },
] as const satisfies ReadonlyArray<{
  view: Exclude<FlowPrimaryView, "conversation">;
  labelKey: `flow.nav.${string}`;
  descriptionKey: `flow.nav.${string}.description`;
  icon: typeof Workflow;
}>;

export function Sidebar({
  projects,
  threads,
  threadActivityStore,
  activeThreadId,
  activeProjectId,
  workspaceError,
  isPickingWorkspace,
  experienceMode,
  flowModeEnabled,
  newTaskOpen,
  activeFlowPrimaryView,
  pluginsOpen,
  contextualCollection,
  onExperienceModeChange,
  onOpenFlowPrimaryView,
  onSelect,
  onNew,
  onPickWorkspace,
  onCreateProject,
  onRemoveProject,
  onRenameProject,
  onToggleProjectPinned,
  onSelectProject,
  onOpenThreadWorkspace,
  onNewThreadForProject,
  onRenameThread,
  onOpenThreadUsage,
  onArchiveThread,
  onRestoreThread,
  onOpenExtensions,
  onOpenTaskSearch,
  onSettings,
}: {
  projects: Project[];
  threads: Thread[];
  threadActivityStore: ThreadActivityStore;
  activeThreadId: string | null;
  activeProjectId: string | null;
  workspaceError: string | null;
  isPickingWorkspace: boolean;
  experienceMode: ExperienceMode;
  flowModeEnabled: boolean;
  newTaskOpen: boolean;
  activeFlowPrimaryView: FlowPrimaryView | null;
  pluginsOpen: boolean;
  contextualCollection?: ReactNode;
  onExperienceModeChange(mode: ExperienceMode): void;
  onOpenFlowPrimaryView(view: Exclude<FlowPrimaryView, "conversation">): void;
  onSelect(id: string): void;
  onNew(): void;
  onPickWorkspace(): void;
  onCreateProject(name: string): Promise<Project | null>;
  onRemoveProject(project: Project): void;
  onRenameProject(project: Project): void;
  onToggleProjectPinned(project: Project): void;
  onSelectProject(project: Project): void;
  onOpenThreadWorkspace(workspaceRoot: string): void;
  onNewThreadForProject?(project: Project): void;
  onRenameThread(thread: Thread): void;
  onOpenThreadUsage(thread: Thread): void;
  onArchiveThread(thread: Thread): void;
  onRestoreThread(thread: Thread): void;
  onOpenExtensions(): void;
  onOpenTaskSearch(): void;
  onSettings(): void;
}) {
  const { t } = useApplicationLanguage();
  const initialNavigationState = useMemo(readSidebarNavigationState, []);
  const [experienceMenuOpen, setExperienceMenuOpen] = useState(false);
  const [projectMenuOpen, setProjectMenuOpen] = useState(false);
  const [newProjectOpen, setNewProjectOpen] = useState(false);
  const [newProjectName, setNewProjectName] = useState("New project");
  const [isCreatingProject, setIsCreatingProject] = useState(false);
  const [expandedProjects, setExpandedProjects] = useState<Set<string>>(
    () => new Set(initialNavigationState.expandedProjectIds),
  );
  const [projectThreadDisplayLimits, setProjectThreadDisplayLimits] = useState<
    Map<string, number>
  >(() => new Map());
  const [moreMenuProjectId, setMoreMenuProjectId] = useState<string | null>(
    null,
  );
  const [unassignedExpanded, setUnassignedExpanded] = useState(
    initialNavigationState.unassignedExpanded,
  );
  const [archivedExpanded, setArchivedExpanded] = useState(
    initialNavigationState.archivedExpanded,
  );
  const [hoveredProject, setHoveredProject] =
    useState<ProjectHoverState | null>(null);
  const moreMenuRef = useDismissiblePopover(moreMenuProjectId !== null, () =>
    setMoreMenuProjectId(null),
  );
  const projectMenuRef = useDismissiblePopover(projectMenuOpen, () =>
    setProjectMenuOpen(false),
  );
  const experienceMenuRef = useDismissiblePopover(experienceMenuOpen, () =>
    setExperienceMenuOpen(false),
  );
  const threadRowActionsRef = useRef({
    onSelect,
    onRenameThread,
    onOpenThreadUsage,
    onArchiveThread,
    onRestoreThread,
  });
  threadRowActionsRef.current = {
    onSelect,
    onRenameThread,
    onOpenThreadUsage,
    onArchiveThread,
    onRestoreThread,
  };
  const selectThreadRow = useCallback(
    (thread: Thread) => threadRowActionsRef.current.onSelect(thread.id),
    [],
  );
  const renameThreadRow = useCallback(
    (thread: Thread) => threadRowActionsRef.current.onRenameThread(thread),
    [],
  );
  const openThreadUsage = useCallback(
    (thread: Thread) => threadRowActionsRef.current.onOpenThreadUsage(thread),
    [],
  );
  const archiveThreadRow = useCallback(
    (thread: Thread) => threadRowActionsRef.current.onArchiveThread(thread),
    [],
  );
  const restoreThreadRow = useCallback(
    (thread: Thread) => threadRowActionsRef.current.onRestoreThread(thread),
    [],
  );
  const modeThreads = threads.filter(
    (thread) => thread.experienceMode === experienceMode,
  );
  const unassignedThreads = modeThreads.filter(
    (thread) => !thread.projectId && !thread.archivedAt,
  );
  const archivedThreads = modeThreads.filter((thread) => thread.archivedAt);
  const experienceModeOptions = (
    [
      { id: "work", label: "Work", icon: BriefcaseBusiness },
      { id: "code", label: "Code", icon: Code2 },
      { id: "flow", label: "Flow", icon: Workflow },
    ] as const
  ).filter((option) => option.id !== "flow" || flowModeEnabled);
  const activeExperienceMode =
    experienceModeOptions.find((option) => option.id === experienceMode) ??
    experienceModeOptions.find((option) => option.id === "code")!;
  const ActiveExperienceModeIcon = activeExperienceMode.icon;
  function toggleExpandedProject(projectId: string) {
    setExpandedProjects((prev) => {
      const next = new Set(prev);
      if (next.has(projectId)) {
        next.delete(projectId);
      } else {
        next.add(projectId);
      }
      return next;
    });
  }

  useEffect(() => {
    updateSidebarNavigationState({
      expandedProjectIds: [...expandedProjects],
      unassignedExpanded,
      archivedExpanded,
    });
  }, [archivedExpanded, expandedProjects, unassignedExpanded]);

  useEffect(() => {
    if (!newProjectOpen) return;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setNewProjectOpen(false);
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [newProjectOpen]);

  async function createProject() {
    const name = newProjectName.trim();
    if (!name || isCreatingProject) return;
    setIsCreatingProject(true);
    const project = await onCreateProject(name);
    setIsCreatingProject(false);
    if (!project) return;
    setNewProjectOpen(false);
    setProjectMenuOpen(false);
    setNewProjectName("New project");
    onSelectProject(project);
  }

  return (
    <>
      <aside className="sidebar" id="workspace-sidebar">
        <div className="sidebar-brand-row">
          <div className="experience-mode-menu" ref={experienceMenuRef}>
            <button
              type="button"
              className="experience-mode-trigger"
              aria-label={`当前模式：${activeExperienceMode.label}`}
              aria-haspopup="menu"
              aria-expanded={experienceMenuOpen}
              onClick={() => setExperienceMenuOpen((current) => !current)}
            >
              <ActiveExperienceModeIcon size={15} aria-hidden="true" />
              <span>{activeExperienceMode.label}</span>
              <ChevronDown
                className={experienceMenuOpen ? "open" : undefined}
                size={14}
                aria-hidden="true"
              />
            </button>
            {experienceMenuOpen && (
              <div className="tool-popover experience-mode-popover" role="menu">
                {experienceModeOptions.map((option) => {
                  const Icon = option.icon;
                  const selected = option.id === experienceMode;
                  return (
                    <button
                      key={option.id}
                      type="button"
                      role="menuitemradio"
                      aria-checked={selected}
                      className={selected ? "active" : undefined}
                      onClick={() => {
                        onExperienceModeChange(option.id);
                        setExperienceMenuOpen(false);
                      }}
                    >
                      <Icon size={14} aria-hidden="true" />
                      <span>{option.label}</span>
                      {selected && <Check size={13} aria-hidden="true" />}
                    </button>
                  );
                })}
              </div>
            )}
          </div>
          <IconButton
            className="sidebar-icon-button"
            size="compact"
            title="搜索任务 (Ctrl+K)"
            aria-label="搜索任务"
            onClick={onOpenTaskSearch}
          >
            <Search size={15} aria-hidden="true" />
          </IconButton>
        </div>
        <nav className="primary-nav" aria-label="主要导航">
          <button
            aria-current={newTaskOpen ? "page" : undefined}
            onClick={onNew}
          >
            <FileText size={15} />
            <span>新建任务</span>
          </button>
          {experienceMode === "flow" ? (
            <>
              {FLOW_NAVIGATION_ITEMS.map((item) => {
                const Icon = item.icon;
                return (
                  <button
                    aria-current={
                      activeFlowPrimaryView === item.view ? "page" : undefined
                    }
                    key={item.view}
                    onClick={() => onOpenFlowPrimaryView(item.view)}
                  >
                    <Icon aria-hidden="true" size={15} />
                    <span>{t(item.labelKey)}</span>
                    <small>{t(item.descriptionKey)}</small>
                  </button>
                );
              })}
            </>
          ) : null}
          <button disabled title="拉取请求 · 未实现">
            <GitPullRequest size={15} />
            <span>拉取请求</span>
          </button>
          <button disabled title="已安排 · 未实现">
            <Clock3 size={15} />
            <span>已安排</span>
          </button>
          <button
            aria-current={pluginsOpen ? "page" : undefined}
            onClick={onOpenExtensions}
            title="管理插件"
          >
            <Plug size={15} />
            <span>插件</span>
          </button>
        </nav>

        {contextualCollection ? (
          <div className="sidebar-contextual-collection sidebar-scroll-region">
            {contextualCollection}
          </div>
        ) : (
          <>
            <div className="project-heading">
              <span>项目</span>
              <div className="sidebar-project-menu-wrap" ref={projectMenuRef}>
                <button
                  className="sidebar-icon-button"
                  disabled={isPickingWorkspace}
                  onClick={() => setProjectMenuOpen((current) => !current)}
                  title="添加项目"
                  aria-label="添加项目"
                  aria-expanded={projectMenuOpen}
                >
                  {isPickingWorkspace ? (
                    <Loader2 size={14} className="spin" />
                  ) : (
                    <Plus size={14} />
                  )}
                </button>
                {projectMenuOpen && (
                  <div
                    className="tool-popover sidebar-project-popover"
                    role="menu"
                  >
                    <button
                      role="menuitem"
                      onClick={() => {
                        setNewProjectOpen(true);
                        setProjectMenuOpen(false);
                      }}
                    >
                      <Plus size={14} />
                      <span>新建空白项目</span>
                    </button>
                    <button
                      role="menuitem"
                      onClick={() => {
                        onPickWorkspace();
                        setProjectMenuOpen(false);
                      }}
                    >
                      <FolderOpen size={14} />
                      <span>使用现有文件夹</span>
                    </button>
                  </div>
                )}
              </div>
            </div>
            <div className="project-tree sidebar-scroll-region">
              {projects.map((project, projectIndex) => {
                const projectThreads = modeThreads.filter(
                  (thread) =>
                    thread.projectId === project.id && !thread.archivedAt,
                );
                const isActive = project.id === activeProjectId;
                const isExpanded = expandedProjects.has(project.id);
                const threadDisplayLimit =
                  projectThreadDisplayLimits.get(project.id) ??
                  PROJECT_THREAD_PREVIEW_LIMIT;
                const visibleProjectThreads = projectThreads.slice(
                  0,
                  threadDisplayLimit,
                );
                const isMoreMenuOpen = moreMenuProjectId === project.id;
                const projectInfoId = `project-hover-card-${projectIndex}`;
                return (
                  <section
                    className={`project-node ${isActive ? "active" : ""}`}
                    key={project.id}
                  >
                    <div className="project-row">
                      <button
                        className="project-select"
                        title={
                          project.workspaceRoot
                            ? formatPathForDisplay(project.workspaceRoot)
                            : project.name
                        }
                        aria-label={`项目 ${project.name}`}
                        aria-describedby={projectInfoId}
                        onMouseEnter={(event) => {
                          const bounds =
                            event.currentTarget.getBoundingClientRect();
                          const sidebarRight =
                            event.currentTarget
                              .closest("#workspace-sidebar")
                              ?.getBoundingClientRect().right ?? bounds.right;
                          const cardWidth = 320;
                          const left = Math.min(
                            sidebarRight + 8,
                            window.innerWidth - cardWidth - 8,
                          );
                          setHoveredProject({
                            id: projectInfoId,
                            name: project.name,
                            threadCount: projectThreads.length,
                            workspaceRoot: project.workspaceRoot,
                            pinned: project.pinned,
                            left: Math.max(8, left),
                            top: Math.max(
                              36,
                              Math.min(bounds.top, window.innerHeight - 174),
                            ),
                          });
                        }}
                        onMouseLeave={() => setHoveredProject(null)}
                        onClick={() => {
                          if (isExpanded) {
                            setProjectThreadDisplayLimits((current) => {
                              if (!current.has(project.id)) return current;
                              const next = new Map(current);
                              next.delete(project.id);
                              return next;
                            });
                          }
                          toggleExpandedProject(project.id);
                          onSelectProject(project);
                        }}
                      >
                        {isExpanded ? (
                          <FolderOpen size={14} />
                        ) : (
                          <Folder size={14} />
                        )}
                        <span>{project.name}</span>
                      </button>
                      <div className="project-row-actions">
                        <div
                          className="project-menu-wrap"
                          ref={isMoreMenuOpen ? moreMenuRef : undefined}
                        >
                          <button
                            className="project-more"
                            aria-label={`菜单 ${project.name}`}
                            aria-expanded={isMoreMenuOpen}
                            onClick={() =>
                              setMoreMenuProjectId(
                                isMoreMenuOpen ? null : project.id,
                              )
                            }
                          >
                            <MoreHorizontal size={13} />
                          </button>
                          {isMoreMenuOpen && (
                            <div
                              className="tool-popover project-row-popover"
                              role="menu"
                            >
                              <button
                                role="menuitem"
                                disabled={!project.workspaceRoot}
                                onClick={() => {
                                  if (project.workspaceRoot) {
                                    onOpenThreadWorkspace(
                                      project.workspaceRoot,
                                    );
                                  }
                                  setMoreMenuProjectId(null);
                                }}
                              >
                                <FolderOpen size={14} />
                                <span>在文件管理器中打开</span>
                              </button>
                              <button
                                role="menuitem"
                                onClick={() => {
                                  onRenameProject(project);
                                  setMoreMenuProjectId(null);
                                }}
                              >
                                <Pencil size={14} />
                                <span>重命名</span>
                              </button>
                              <button
                                role="menuitem"
                                onClick={() => {
                                  onToggleProjectPinned(project);
                                  setMoreMenuProjectId(null);
                                }}
                              >
                                <Pin size={14} />
                                <span>
                                  {project.pinned ? "取消固定项目" : "固定项目"}
                                </span>
                              </button>
                              <button disabled title="Git 工作树管理尚未实现">
                                <GitFork size={14} />
                                <span>创建工作树</span>
                                <small>未实现</small>
                              </button>
                              <div className="tool-popover-separator" />
                              <button
                                role="menuitem"
                                onClick={() => {
                                  onRemoveProject(project);
                                  setMoreMenuProjectId(null);
                                }}
                              >
                                <Archive size={14} />
                                <span>归档项目</span>
                              </button>
                            </div>
                          )}
                        </div>
                        <button
                          className="project-new-thread"
                          title="新建对话"
                          aria-label={`在 ${project.name} 中新建对话`}
                          onClick={() => {
                            onNewThreadForProject?.(project);
                          }}
                        >
                          <SquarePen size={13} />
                        </button>
                      </div>
                    </div>
                    {isExpanded && (
                      <div className="project-tasks">
                        {visibleProjectThreads.map((thread) => (
                          <SidebarThreadRow
                            active={thread.id === activeThreadId}
                            activityStore={threadActivityStore}
                            key={thread.id}
                            thread={thread}
                            onSelect={selectThreadRow}
                            onRename={renameThreadRow}
                            onOpenUsage={openThreadUsage}
                            onArchive={archiveThreadRow}
                          />
                        ))}
                        {projectThreads.length > threadDisplayLimit && (
                          <Button
                            className="project-show-more"
                            size="compact"
                            variant="quiet"
                            onClick={() =>
                              setProjectThreadDisplayLimits((current) => {
                                const next = new Map(current);
                                next.set(
                                  project.id,
                                  (current.get(project.id) ??
                                    PROJECT_THREAD_PREVIEW_LIMIT) +
                                    PROJECT_THREAD_PREVIEW_LIMIT,
                                );
                                return next;
                              })
                            }
                          >
                            展开显示
                          </Button>
                        )}
                        {projectThreads.length === 0 && (
                          <span className="project-empty">无任务</span>
                        )}
                      </div>
                    )}
                  </section>
                );
              })}
              {unassignedThreads.length > 0 && (
                <section className="project-node">
                  <div className="project-row">
                    <button
                      className="project-select"
                      title="尚未归属到项目的任务"
                      onClick={() =>
                        setUnassignedExpanded((current) => !current)
                      }
                    >
                      {unassignedExpanded ? (
                        <FolderOpen size={14} />
                      ) : (
                        <Folder size={14} />
                      )}
                      <span>未归属任务 ({unassignedThreads.length})</span>
                    </button>
                  </div>
                  {unassignedExpanded && (
                    <div className="project-tasks">
                      {unassignedThreads.map((thread) => (
                        <SidebarThreadRow
                          active={thread.id === activeThreadId}
                          activityStore={threadActivityStore}
                          key={thread.id}
                          thread={thread}
                          onSelect={selectThreadRow}
                          onRename={renameThreadRow}
                          onOpenUsage={openThreadUsage}
                          onArchive={archiveThreadRow}
                        />
                      ))}
                    </div>
                  )}
                </section>
              )}
              {archivedThreads.length > 0 && (
                <section className="project-node">
                  <div className="project-row">
                    <button
                      className="project-select"
                      title="查看可恢复的归档任务"
                      onClick={() => setArchivedExpanded((current) => !current)}
                    >
                      <Archive size={14} />
                      <span>已归档 ({archivedThreads.length})</span>
                    </button>
                  </div>
                  {archivedExpanded && (
                    <div className="project-tasks">
                      {archivedThreads.map((thread) => (
                        <SidebarThreadRow
                          archived
                          active={false}
                          activityStore={threadActivityStore}
                          key={thread.id}
                          thread={thread}
                          onSelect={restoreThreadRow}
                          onRename={renameThreadRow}
                          onOpenUsage={openThreadUsage}
                          onRestore={restoreThreadRow}
                        />
                      ))}
                    </div>
                  )}
                </section>
              )}
              {projects.length === 0 && (
                <p className="workspace-empty">尚未打开项目</p>
              )}
              {workspaceError && (
                <p className="workspace-error">{workspaceError}</p>
              )}
            </div>
          </>
        )}

        <div className="sidebar-footer">
          <button
            className="sidebar-settings-button"
            title="设置"
            aria-label="设置"
            onClick={onSettings}
          >
            <Settings size={15} />
            <span className="opentopia-wordmark" aria-hidden="true">
              <span className="brand-open">Open</span>
              <span>Topia</span>
            </span>
          </button>
          <button disabled title="帮助 · 未实现" aria-label="帮助">
            <CircleHelp size={15} />
          </button>
        </div>
      </aside>
      {hoveredProject &&
        createPortal(
          <div
            className="project-hover-card"
            id={hoveredProject.id}
            role="tooltip"
            style={{ left: hoveredProject.left, top: hoveredProject.top }}
          >
            <header>
              <span>
                <Folder size={17} aria-hidden="true" />
                <strong>{hoveredProject.name}</strong>
              </span>
              <button
                disabled
                className={hoveredProject.pinned ? "active" : undefined}
                title={hoveredProject.pinned ? "已固定" : "未固定"}
                aria-label={hoveredProject.pinned ? "已固定" : "未固定"}
              >
                <Pin
                  size={14}
                  fill={hoveredProject.pinned ? "currentColor" : "none"}
                  aria-hidden="true"
                />
              </button>
            </header>
            <div className="project-hover-card__row">
              <MessageCircle size={15} aria-hidden="true" />
              <span>{hoveredProject.threadCount} 个对话串</span>
            </div>
            <div className="project-hover-card__divider" />
            <div className="project-hover-card__row">
              <Folder size={15} aria-hidden="true" />
              <span
                title={
                  hoveredProject.workspaceRoot
                    ? formatPathForDisplay(hoveredProject.workspaceRoot)
                    : undefined
                }
              >
                {hoveredProject.workspaceRoot
                  ? formatPathForDisplay(hoveredProject.workspaceRoot)
                  : "尚未选择工作区"}
              </span>
            </div>
          </div>,
          document.body,
        )}
      {newProjectOpen && (
        <div
          className="modal-backdrop project-modal-backdrop"
          role="presentation"
          onClick={() => setNewProjectOpen(false)}
        >
          <form
            className="project-name-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="project-name-title"
            onClick={(event) => event.stopPropagation()}
            onSubmit={(event) => {
              event.preventDefault();
              void createProject();
            }}
          >
            <header>
              <div>
                <h2 id="project-name-title">为项目命名</h2>
                <p>项目可以稍后再选择工作区。</p>
              </div>
              <button
                className="icon-button small"
                type="button"
                aria-label="关闭项目弹窗"
                onClick={() => setNewProjectOpen(false)}
              >
                <X size={14} />
              </button>
            </header>
            <input
              autoFocus
              aria-label="项目名称"
              value={newProjectName}
              onChange={(event) => setNewProjectName(event.target.value)}
            />
            <footer>
              <button
                className="secondary-button"
                type="button"
                onClick={() => setNewProjectOpen(false)}
              >
                取消
              </button>
              <button
                className="primary-button"
                type="submit"
                disabled={!newProjectName.trim() || isCreatingProject}
              >
                {isCreatingProject ? "保存中..." : "保存"}
              </button>
            </footer>
          </form>
        </div>
      )}
    </>
  );
}
