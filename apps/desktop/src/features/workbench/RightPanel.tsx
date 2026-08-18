import {
  CirclePlus,
  GitFork,
  Maximize2,
  Minimize2,
  PanelRightClose,
  Plus,
  X,
} from "lucide-react";
import { ApiClient } from "../../api/client";
import type {
  DiffReviewFileContent,
  DiffReviewGitAction,
} from "../../components/DiffReviewPanel";
import { FileTypeIcon } from "../../components/FileTypeIcon";
import { FlowWorkspacePanel } from "../../components/FlowWorkspacePanel";
import { ComputerPanel } from "../../components/ComputerPanel";
import {
  InlineImagePreview,
  type ImagePreviewSource,
  PreviewHost,
} from "../../components/PreviewHost";
import { RightContextRail } from "../../components/RightContextRail";
import { UsageLogDashboard } from "../../components/UsageLogDashboard";
import { WebPreviewSurface } from "../../components/WebPreviewSurface";
import {
  terminalShellName,
  WorkbenchPanel,
  type WorkbenchTab,
} from "../../components/WorkbenchPanel";
import { Button, IconButton, Popover } from "../../components/ui";
import { ConversationSessionRegistry } from "../../conversationSessionController";
import { PreviewSessionStore } from "../../previewSessionStore";
import type { ThreadActivityStatus } from "../../threadActivityStatus";
import {
  toolStageLauncherKinds,
  toolTabIcon,
  toolTabMenuItems,
  toolTabTitle,
  type ToolTab,
  type ToolTabKind,
} from "../../toolTabs";
import type {
  AgentEvent,
  AgentListItem,
  AppSettings,
  ArtifactContent,
  ArtifactDescriptor,
  CollaborationMode,
  ContextStatus,
  ExperienceMode,
  LibraryProviderId,
  McpServerInput,
  McpServerView,
  Message,
  PluginView,
  PreviewTarget,
  Project,
  ReviewFileRequest,
  SandboxDescriptor,
  SkillDescriptor,
  TerminalEvent,
  TerminalSession,
  Thread,
  ThreadMcpServerView,
  WorkspaceDiff,
  WorkspaceDiffHunk,
  WorkspaceDiffHunkAction,
  WorkspaceEntry,
  WorkspaceFilePreview,
  WorkspaceTree,
} from "../../types";
import type { ExecutionPermissionMode } from "../composer/Composer";
import { ConversationLoadingState } from "../conversation/ConversationHeader";
import { SideTaskConversation } from "../side-task/SideTaskConversation";

export function RightPanel({
  client,
  conversationRegistry,
  experienceMode,
  threads,
  toolTabs,
  activeToolTab,
  toolStageOpen,
  conversationCollapsed,
  activeToolRequiresFullWorkspace,
  contextRailOpen,
  contextRailAutoVisible,
  thread,
  settings,
  projects,
  skills,
  collaborationMode,
  libraryProvider,
  workspaceRoot,
  agentItems,
  messages,
  events,
  conversationLoading,
  terminalEvents,
  terminalSession,
  workspaceTree,
  filePreview,
  workspaceDiff,
  sandbox,
  plugins,
  selectedSkillIds,
  mcpServers,
  threadMcpServers,
  workbenchError,
  isRefreshingWorkbench,
  pendingApprovalIds,
  decidingApprovalId,
  artifacts,
  contextStatus,
  isCompactingContext,
  revertingDiffPath,
  hunkActionKey,
  reviewFileRequest,
  onDecideApproval,
  onRefreshWorkbench,
  onOpenWorkspacePath,
  onOpenWorkspaceEntry,
  onToggleThreadMcp,
  onCreateMcpServer,
  onUpdateMcpServer,
  onRestartMcpServer,
  onDeleteMcpServer,
  onInstallPlugin,
  onUninstallPlugin,
  onToggleThreadPlugin,
  onUsePluginSkills,
  onOpenWorkspace,
  onEnsureTerminalSession,
  onWriteTerminalSession,
  onResizeTerminalSession,
  onCloseTerminalSession,
  onCompactContext,
  onOpenArtifact,
  onOpenImagePreview,
  onOpenPreview,
  onOpenMarkdownLink,
  onRevertDiffFile,
  onApplyDiffHunk,
  onOpenFileTab,
  onLoadFileContent,
  onLoadTurnFileDiff,
  onGitAction,
  onGetArtifact,
  onOpenToolTab,
  onOpenSideTask,
  onThreadUpdated,
  onSetThreadActivity,
  onMarkThreadActivityRead,
  onChangePermissionMode,
  onChangeSandboxMode,
  onChangeLibraryProvider,
  onOpenSettings,
  onActivateToolTab,
  onCloseToolTab,
  previewSessionStore,
  onToggleConversation,
  onHideToolStage,
  onAddContextSources,
  onInterruptAgent,
}: {
  client: ApiClient | null;
  conversationRegistry: ConversationSessionRegistry | null;
  experienceMode: ExperienceMode;
  threads: Thread[];
  toolTabs: ToolTab[];
  activeToolTab: ToolTab | null;
  toolStageOpen: boolean;
  conversationCollapsed: boolean;
  activeToolRequiresFullWorkspace: boolean;
  contextRailOpen: boolean;
  contextRailAutoVisible: boolean;
  thread: Thread | null;
  settings: AppSettings | null;
  projects: Project[];
  skills: SkillDescriptor[];
  collaborationMode: CollaborationMode;
  libraryProvider: LibraryProviderId | null;
  workspaceRoot: string | null;
  messages: Message[];
  agentItems: AgentListItem[];
  events: AgentEvent[];
  conversationLoading: boolean;
  terminalEvents: TerminalEvent[];
  terminalSession: TerminalSession | null;
  workspaceTree: WorkspaceTree | null;
  filePreview: WorkspaceFilePreview | null;
  workspaceDiff: WorkspaceDiff | null;
  sandbox: SandboxDescriptor | null;
  plugins: PluginView[];
  selectedSkillIds: string[];
  mcpServers: McpServerView[];
  threadMcpServers: ThreadMcpServerView[];
  workbenchError: string | null;
  isRefreshingWorkbench: boolean;
  pendingApprovalIds: string[];
  decidingApprovalId: string | null;
  artifacts: ArtifactDescriptor[];
  contextStatus: ContextStatus | null;
  isCompactingContext: boolean;
  revertingDiffPath: string | null;
  hunkActionKey: string | null;
  reviewFileRequest: ReviewFileRequest | null;
  onDecideApproval(approvalId: string, approved: boolean): void;
  onRefreshWorkbench(): void;
  onOpenWorkspacePath(path?: string): void;
  onOpenWorkspaceEntry(entry: WorkspaceEntry): void;
  onToggleThreadMcp(serverId: string, enabled: boolean): void;
  onCreateMcpServer(input: McpServerInput): Promise<void>;
  onUpdateMcpServer(serverId: string, input: McpServerInput): Promise<void>;
  onRestartMcpServer(serverId: string): Promise<void>;
  onDeleteMcpServer(serverId: string): Promise<void>;
  onInstallPlugin(): Promise<void>;
  onUninstallPlugin(pluginId: string): Promise<void>;
  onToggleThreadPlugin(pluginId: string, enabled: boolean): Promise<void>;
  onUsePluginSkills(pluginId: string, enabled: boolean): void;
  onOpenWorkspace(workspaceRoot: string): void;
  onEnsureTerminalSession(threadId: string): Promise<TerminalSession>;
  onWriteTerminalSession(
    threadId: string,
    sessionId: string,
    data: string,
  ): void;
  onResizeTerminalSession(
    threadId: string,
    sessionId: string,
    cols: number,
    rows: number,
  ): void;
  onCloseTerminalSession(threadId: string, sessionId: string): void;
  onCompactContext(): void;
  onOpenArtifact(threadId: string, artifactId: string): void;
  onOpenImagePreview(
    threadId: string,
    sourceId: string,
    image: ImagePreviewSource,
  ): void;
  onOpenPreview(threadId: string, target: PreviewTarget, title: string): void;
  onOpenMarkdownLink(href: string, baseWorkspacePath?: string | null): void;
  onRevertDiffFile(path: string): void;
  onApplyDiffHunk(
    hunk: WorkspaceDiffHunk,
    action: WorkspaceDiffHunkAction,
  ): void;
  onOpenFileTab(path: string): void;
  onLoadFileContent(path: string): Promise<DiffReviewFileContent>;
  onLoadTurnFileDiff(turnId: string, path: string): Promise<string>;
  onGitAction(action: DiffReviewGitAction, message: string): Promise<string>;
  onGetArtifact(threadId: string, artifactId: string): Promise<ArtifactContent>;
  onOpenToolTab(kind: ToolTabKind): void;
  onOpenSideTask(): void;
  onThreadUpdated(thread: Thread): void;
  onSetThreadActivity(
    threadId: string,
    status: ThreadActivityStatus | null,
  ): void;
  onMarkThreadActivityRead(threadId: string): void;
  onChangePermissionMode(mode: ExecutionPermissionMode): void;
  onChangeSandboxMode(mode: AppSettings["sandbox"]["sandboxMode"]): void;
  onChangeLibraryProvider(provider: LibraryProviderId | null): void;
  onOpenSettings(): void;
  onActivateToolTab(tabId: string): void;
  onCloseToolTab(tabId: string): void;
  previewSessionStore: PreviewSessionStore;
  onToggleConversation(): void;
  onHideToolStage(): void;
  onAddContextSources(): void;
  onInterruptAgent(agentThreadId: string): void;
}) {
  const renderWorkbench = (
    mode: "panel" | "stage",
    activeTab?: WorkbenchTab,
  ) => (
    <WorkbenchPanel
      client={client}
      mode={mode}
      activeTab={activeTab}
      thread={thread}
      workspaceRoot={workspaceRoot}
      events={events}
      terminalEvents={terminalEvents}
      terminalSession={terminalSession}
      workspaceTree={workspaceTree}
      filePreview={filePreview}
      workspaceDiff={workspaceDiff}
      sandbox={sandbox}
      plugins={plugins}
      selectedSkillIds={selectedSkillIds}
      mcpServers={mcpServers}
      threadMcpServers={threadMcpServers}
      workbenchError={workbenchError}
      isRefreshingWorkbench={isRefreshingWorkbench}
      decidingApprovalId={decidingApprovalId}
      artifacts={artifacts}
      contextStatus={contextStatus}
      isCompactingContext={isCompactingContext}
      revertingDiffPath={revertingDiffPath}
      hunkActionKey={hunkActionKey}
      reviewFileRequest={reviewFileRequest}
      onDecideApproval={onDecideApproval}
      onRefreshWorkbench={onRefreshWorkbench}
      onOpenWorkspacePath={onOpenWorkspacePath}
      onOpenWorkspaceEntry={onOpenWorkspaceEntry}
      onToggleThreadMcp={onToggleThreadMcp}
      onCreateMcpServer={onCreateMcpServer}
      onUpdateMcpServer={onUpdateMcpServer}
      onRestartMcpServer={onRestartMcpServer}
      onDeleteMcpServer={onDeleteMcpServer}
      onInstallPlugin={onInstallPlugin}
      onUninstallPlugin={onUninstallPlugin}
      onToggleThreadPlugin={onToggleThreadPlugin}
      onUsePluginSkills={onUsePluginSkills}
      onOpenPath={onOpenWorkspace}
      onEnsureTerminalSession={onEnsureTerminalSession}
      onWriteTerminalSession={onWriteTerminalSession}
      onResizeTerminalSession={onResizeTerminalSession}
      onCloseTerminalSession={onCloseTerminalSession}
      onCompactContext={onCompactContext}
      onOpenArtifact={onOpenArtifact}
      onRevertDiffFile={onRevertDiffFile}
      onApplyDiffHunk={onApplyDiffHunk}
      onOpenFileTab={onOpenFileTab}
      onLoadFileContent={onLoadFileContent}
      onLoadTurnFileDiff={onLoadTurnFileDiff}
      onGitAction={onGitAction}
      onGetArtifact={onGetArtifact}
    />
  );

  if (toolStageOpen) {
    return (
      <aside className="right-panel tool-stage" id="workspace-right-panel">
        <ToolTabStrip
          tabs={toolTabs}
          activeTabId={activeToolTab?.id ?? null}
          canOpenFlow={thread?.experienceMode === "flow"}
          terminalTitle={
            terminalSession ? terminalShellName(terminalSession.shell) : null
          }
          onActivate={onActivateToolTab}
          onClose={onCloseToolTab}
          onOpen={onOpenToolTab}
          onOpenSideTask={onOpenSideTask}
          canOpenSideTask={Boolean(thread)}
          conversationCollapsed={conversationCollapsed}
          conversationToggleAvailable={!activeToolRequiresFullWorkspace}
          onToggleConversation={onToggleConversation}
          onHide={onHideToolStage}
        />
        <div className="tool-stage-body">
          {!activeToolTab ? (
            <ToolStageLauncher
              canOpenFlow={thread?.experienceMode === "flow"}
              onOpen={onOpenToolTab}
            />
          ) : activeToolTab.kind === "flow" ? (
            thread?.experienceMode === "flow" ? (
              <FlowWorkspacePanel
                client={client}
                threadId={thread.id}
                workspaceRoot={workspaceRoot}
                settings={settings}
              />
            ) : (
              <div className="unavailable-tool-state">
                <GitFork aria-hidden="true" size={20} />
                <h2>Flow View 仅用于 Flow 模式</h2>
                <p>切换到 Flow 任务后，可在这里设计、运行和审阅 Flow。</p>
              </div>
            )
          ) : activeToolTab.kind === "side-task" ? (
            activeToolTab.sideTaskThreadId ? (
              <SideTaskConversation
                key={activeToolTab.sideTaskThreadId}
                client={client}
                conversationRegistry={conversationRegistry}
                thread={
                  threads.find(
                    (item) => item.id === activeToolTab.sideTaskThreadId,
                  ) ?? null
                }
                settings={settings}
                projects={projects}
                skills={skills}
                initialCollaborationMode={collaborationMode}
                onThreadUpdated={onThreadUpdated}
                onSetThreadActivity={onSetThreadActivity}
                onMarkThreadActivityRead={onMarkThreadActivityRead}
                onChangePermissionMode={onChangePermissionMode}
                onChangeSandboxMode={onChangeSandboxMode}
                onOpenSettings={onOpenSettings}
                onOpenArtifact={onOpenArtifact}
                onOpenImagePreview={onOpenImagePreview}
                onOpenPreview={onOpenPreview}
                onOpenMarkdownLink={onOpenMarkdownLink}
                onOpenToolTab={onOpenToolTab}
                onOpenFileReview={onOpenFileTab}
              />
            ) : (
              <ConversationLoadingState />
            )
          ) : activeToolTab.kind === "browser" ? (
            <WebPreviewSurface
              client={client}
              threadId={thread?.id ?? null}
              events={events}
              navigationRequest={activeToolTab.browserNavigation ?? null}
            />
          ) : activeToolTab.kind === "computer" ? (
            <ComputerPanel
              client={client}
              threadId={thread?.id ?? null}
              events={events}
            />
          ) : activeToolTab.kind === "usage" ? (
            thread ? (
              <UsageLogDashboard
                thread={thread}
                events={events}
                isLoading={conversationLoading}
              />
            ) : (
              <ConversationLoadingState />
            )
          ) : activeToolTab.kind === "image" && activeToolTab.imagePreview ? (
            <InlineImagePreview image={activeToolTab.imagePreview} />
          ) : activeToolTab.kind === "preview" &&
            activeToolTab.previewTarget ? (
            <PreviewHost
              client={client}
              previewSessionStore={previewSessionStore}
              sessionId={activeToolTab.id}
              threadId={thread?.id ?? null}
              workspaceRoot={workspaceRoot}
              target={activeToolTab.previewTarget}
              onOpenMarkdownLink={onOpenMarkdownLink}
            />
          ) : (
            activeToolTab.kind !== "image" &&
            activeToolTab.kind !== "preview" &&
            renderWorkbench("stage", activeToolTab.kind)
          )}
        </div>
      </aside>
    );
  }

  return (
    <aside
      className={`context-rail-shell ${
        contextRailOpen ? "is-visible" : ""
      } ${contextRailOpen ? (contextRailAutoVisible ? "is-inline" : "is-menu") : ""}`}
      id="workspace-context-rail"
      aria-label="右侧上下文摘要"
      aria-hidden={!contextRailOpen}
    >
      <RightContextRail
        client={client}
        threadId={thread?.id ?? null}
        workspaceRoot={workspaceRoot}
        workspaceDiff={workspaceDiff}
        terminalEvents={terminalEvents}
        terminalSession={terminalSession}
        agentEvents={events}
        agentItems={agentItems}
        artifacts={artifacts}
        messages={messages}
        libraryPickerEnabled={experienceMode === "flow"}
        libraryProvider={libraryProvider}
        onOpenDiff={() => onOpenToolTab("diff")}
        onOpenTerminal={() => onOpenToolTab("terminal")}
        onOpenFiles={() => onOpenToolTab("files")}
        onOpenEnvironment={() => onOpenToolTab("sandbox")}
        onChangeLibraryProvider={onChangeLibraryProvider}
        onOpenPreview={(target, title) => {
          if (thread) onOpenPreview(thread.id, target, title);
        }}
        onAddSource={onAddContextSources}
        onInterruptAgent={onInterruptAgent}
        onGitChanged={onRefreshWorkbench}
      />
    </aside>
  );
}

function ToolStageLauncher({
  canOpenFlow,
  onOpen,
}: {
  canOpenFlow: boolean;
  onOpen(kind: Exclude<ToolTabKind, "image" | "preview" | "side-task">): void;
}) {
  return (
    <div className="tool-stage-empty">
      <nav className="tool-stage-launcher" aria-label="打开工具">
        {toolStageLauncherKinds
          .filter(({ kind }) => kind !== "flow" || canOpenFlow)
          .map(({ kind, label }) => {
            const Icon = toolTabIcon(kind);
            return (
              <Button
                className="tool-stage-launcher-button"
                key={kind}
                variant="quiet"
                onClick={() => onOpen(kind)}
              >
                <Icon size={16} aria-hidden="true" />
                <span>{label}</span>
              </Button>
            );
          })}
      </nav>
    </div>
  );
}

function ToolTabStrip({
  tabs,
  activeTabId,
  canOpenFlow,
  terminalTitle,
  onActivate,
  onClose,
  onOpen,
  onOpenSideTask,
  canOpenSideTask,
  conversationCollapsed,
  conversationToggleAvailable,
  onToggleConversation,
  onHide,
}: {
  tabs: ToolTab[];
  activeTabId: string | null;
  canOpenFlow: boolean;
  terminalTitle: string | null;
  onActivate(tabId: string): void;
  onClose(tabId: string): void;
  onOpen(kind: ToolTabKind): void;
  onOpenSideTask(): void;
  canOpenSideTask: boolean;
  conversationCollapsed: boolean;
  conversationToggleAvailable: boolean;
  onToggleConversation(): void;
  onHide(): void;
}) {
  function open(kind: ToolTabKind, close: () => void) {
    onOpen(kind);
    close();
  }

  return (
    <div className="tool-tab-strip">
      <div className="tool-tab-list" role="tablist" aria-label="工作工具">
        {tabs.map((tab) => {
          const Icon = toolTabIcon(tab.kind);
          const title =
            tab.kind === "terminal" && terminalTitle
              ? terminalTitle
              : tab.title;
          return (
            <div
              className={`tool-stage-tab ${tab.id === activeTabId ? "active" : ""}`}
              key={tab.id}
            >
              <button
                className="tool-tab-main"
                type="button"
                role="tab"
                aria-selected={tab.id === activeTabId}
                title={title}
                onClick={() => onActivate(tab.id)}
              >
                {tab.kind === "preview" &&
                tab.previewTarget?.type === "attachment" ? (
                  <FileTypeIcon name={title} size={14} />
                ) : (
                  <Icon size={13} />
                )}
                <span>{title}</span>
              </button>
              <button
                className="tool-tab-close"
                type="button"
                aria-label={`关闭 ${title}`}
                onClick={(event) => {
                  event.stopPropagation();
                  onClose(tab.id);
                }}
              >
                <X size={12} />
              </button>
            </div>
          );
        })}
      </div>
      <div className="tool-tab-add-wrap">
        <Popover
          label="打开工具"
          align="end"
          placement="bottom"
          trigger={(props) => (
            <IconButton
              className="tool-tab-add"
              size="compact"
              variant="quiet"
              title="打开工具"
              aria-label="打开工具"
              {...props}
            >
              <Plus size={14} aria-hidden="true" />
            </IconButton>
          )}
        >
          {({ close }) => (
            <div className="tool-popover tool-tab-add-popover" role="menu">
              {toolTabMenuItems
                .filter(({ kind }) => kind !== "flow" || canOpenFlow)
                .map(({ kind, shortcut }) => {
                  const Icon = toolTabIcon(kind);
                  return (
                    <button
                      key={kind}
                      role="menuitem"
                      onClick={() => open(kind, close)}
                    >
                      <Icon size={14} aria-hidden="true" />
                      <span>{toolTabTitle(kind)}</span>
                      {shortcut ? <kbd>{shortcut}</kbd> : null}
                    </button>
                  );
                })}
              <button
                type="button"
                role="menuitem"
                disabled={!canOpenSideTask}
                title={canOpenSideTask ? "新建侧边会话" : "请先打开一个任务"}
                onClick={() => {
                  close();
                  onOpenSideTask();
                }}
              >
                <CirclePlus size={14} aria-hidden="true" />
                <span>侧边任务</span>
                <kbd>Ctrl+Alt+S</kbd>
              </button>
            </div>
          )}
        </Popover>
      </div>
      <div className="tool-tab-actions">
        {conversationToggleAvailable ? (
          <IconButton
            className="tool-tab-action"
            size="compact"
            variant="quiet"
            title={conversationCollapsed ? "还原工具工作区" : "扩展工具工作区"}
            aria-label={
              conversationCollapsed ? "还原工具工作区" : "扩展工具工作区"
            }
            aria-pressed={conversationCollapsed}
            onClick={onToggleConversation}
          >
            {conversationCollapsed ? (
              <Minimize2 size={14} aria-hidden="true" />
            ) : (
              <Maximize2 size={14} aria-hidden="true" />
            )}
          </IconButton>
        ) : null}
        <IconButton
          className="tool-tab-action"
          size="compact"
          variant="quiet"
          title="折叠工具窗口"
          aria-label="折叠工具窗口"
          onClick={onHide}
        >
          <PanelRightClose size={14} aria-hidden="true" />
        </IconButton>
      </div>
    </div>
  );
}
