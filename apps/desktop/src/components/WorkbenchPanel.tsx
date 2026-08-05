import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type FormEvent,
} from "react";
import {
  Box,
  Check,
  ChevronDown,
  ChevronRight,
  ExternalLink,
  FileCode2,
  FileText,
  Folder,
  FolderOpen,
  GitBranch,
  Loader2,
  Pencil,
  PackagePlus,
  Plus,
  Puzzle,
  RefreshCw,
  RotateCcw,
  Save,
  Search,
  Settings2,
  ShieldAlert,
  Square,
  TerminalSquare,
  Trash2,
  Workflow,
  Wrench,
  X,
  XCircle,
} from "lucide-react";
import type {
  AgentEvent,
  ArtifactContent,
  ArtifactDescriptor,
  ChangedFile,
  ContextStatus,
  McpServerInput,
  McpServerView,
  Message,
  PluginView,
  ReviewFileRequest,
  SandboxDescriptor,
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
} from "../types";
import type { ApiClient } from "../api/client";
import { formatPathForDisplay } from "../pathDisplay";
import { shouldShowRecordedTurnChanges } from "../turnChangeOwnership";
import { ArtifactGallery } from "./ArtifactGallery";
import { PluginControlPanel } from "./PluginControlPanel";
import {
  DiffReviewPanel,
  type DiffReviewFileContent,
  type DiffReviewGitAction,
  type DiffReviewTurnScope,
} from "./DiffReviewPanel";
import type { XtermTerminalHandle } from "./XtermTerminal";
import { XtermTerminal } from "./XtermTerminal";
import { detectLanguage, MonacoEditor } from "./MonacoEditor";
import { Badge, Button, IconButton } from "./ui";
import { writePendingTerminalEvents } from "../terminalEventReplay";

export type WorkbenchTab =
  | "files"
  | "diff"
  | "terminal"
  | "extensions"
  | "sandbox";

type WorkbenchPanelProps = {
  client: ApiClient | null;
  mode?: "panel" | "stage";
  activeTab?: WorkbenchTab;
  thread: Thread | null;
  workspaceRoot: string | null;
  events: AgentEvent[];
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
  decidingApprovalId: string | null;
  artifacts: ArtifactDescriptor[];
  contextStatus: ContextStatus | null;
  isCompactingContext: boolean;
  revertingDiffPath: string | null;
  hunkActionKey: string | null;
  reviewFileRequest?: ReviewFileRequest | null;
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
  onOpenPath(targetPath: string): void;
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
  onRevertDiffFile(path: string): void;
  onApplyDiffHunk(
    hunk: WorkspaceDiffHunk,
    action: WorkspaceDiffHunkAction,
  ): void;
  /** Opens a workspace file in its own tool tab from the review panel. */
  onOpenFileTab(path: string): void;
  onLoadFileContent(path: string): Promise<DiffReviewFileContent>;
  onLoadTurnFileDiff(turnId: string, path: string): Promise<string>;
  onGitAction(action: DiffReviewGitAction, message: string): Promise<string>;
  onGetArtifact(threadId: string, artifactId: string): Promise<ArtifactContent>;
};

const tabs: Array<{
  id: WorkbenchTab;
  label: string;
  icon: typeof Folder;
}> = [
  { id: "files", label: "Files", icon: Folder },
  { id: "diff", label: "Diff", icon: GitBranch },
  { id: "terminal", label: "Terminal", icon: TerminalSquare },
  { id: "extensions", label: "Plugins", icon: Puzzle },
  { id: "sandbox", label: "Sandbox", icon: Box },
];

export function WorkbenchPanel({
  client,
  mode = "panel",
  activeTab: controlledActiveTab,
  thread,
  workspaceRoot,
  events,
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
  onOpenPath,
  onEnsureTerminalSession,
  onWriteTerminalSession,
  onResizeTerminalSession,
  onCloseTerminalSession,
  onCompactContext,
  onOpenArtifact,
  onRevertDiffFile,
  onApplyDiffHunk,
  onOpenFileTab,
  onLoadFileContent,
  onLoadTurnFileDiff,
  onGitAction,
  onGetArtifact,
}: WorkbenchPanelProps) {
  const [internalActiveTab, setInternalActiveTab] =
    useState<WorkbenchTab>("files");
  const activeTab = controlledActiveTab ?? internalActiveTab;
  const shownWorkspaceRoot = workspaceRoot ?? thread?.workspaceRoot ?? null;
  const latestApproval = [...events]
    .reverse()
    .find((event) => event.payload.type === "approval_requested");
  const latestApprovalPayload =
    latestApproval?.payload.type === "approval_requested"
      ? latestApproval.payload
      : null;

  const tabContent = (
    <>
      {activeTab === "files" && (
        <FilesView
          client={client}
          threadId={thread?.id ?? null}
          workspaceRoot={shownWorkspaceRoot}
          workspaceTree={workspaceTree}
          filePreview={filePreview}
          isRefreshing={isRefreshingWorkbench}
          onRefresh={onRefreshWorkbench}
          onOpenWorkspacePath={onOpenWorkspacePath}
          onOpenWorkspaceEntry={onOpenWorkspaceEntry}
          onOpenPath={onOpenPath}
        />
      )}
      {activeTab === "diff" && (
        <DiffView
          client={client}
          threadId={thread?.id ?? null}
          events={events}
          workspaceDiff={workspaceDiff}
          reviewFileRequest={reviewFileRequest ?? null}
          isRefreshing={isRefreshingWorkbench}
          revertingDiffPath={revertingDiffPath}
          hunkActionKey={hunkActionKey}
          canRunGit={Boolean(thread)}
          onRefresh={onRefreshWorkbench}
          onOpenFileTab={onOpenFileTab}
          onLoadFileContent={onLoadFileContent}
          onLoadTurnFileDiff={onLoadTurnFileDiff}
          onGitAction={onGitAction}
          onRevertDiffFile={onRevertDiffFile}
          onApplyDiffHunk={onApplyDiffHunk}
        />
      )}
      {activeTab === "terminal" && (
        <TerminalView
          thread={thread}
          events={events}
          terminalEvents={terminalEvents}
          terminalSession={terminalSession}
          onEnsureSession={onEnsureTerminalSession}
          onWriteSession={onWriteTerminalSession}
          onResizeSession={onResizeTerminalSession}
          onCloseSession={onCloseTerminalSession}
          onOpenArtifact={onOpenArtifact}
        />
      )}
      {activeTab === "extensions" && (
        <ExtensionsView
          client={client}
          hasThread={Boolean(thread)}
          threadId={thread?.id ?? null}
          workspaceRoot={shownWorkspaceRoot}
          plugins={plugins}
          selectedSkillIds={selectedSkillIds}
          mcpServers={mcpServers}
          threadMcpServers={threadMcpServers}
          onToggleThreadMcp={onToggleThreadMcp}
          onCreateMcpServer={onCreateMcpServer}
          onUpdateMcpServer={onUpdateMcpServer}
          onRestartMcpServer={onRestartMcpServer}
          onDeleteMcpServer={onDeleteMcpServer}
          onInstallPlugin={onInstallPlugin}
          onUninstallPlugin={onUninstallPlugin}
          onToggleThreadPlugin={onToggleThreadPlugin}
          onUsePluginSkills={onUsePluginSkills}
          onOpenPath={onOpenPath}
        />
      )}
      {activeTab === "sandbox" && <SandboxView sandbox={sandbox} />}
    </>
  );

  if (mode === "stage") {
    return (
      <section
        className={`workbench-stage-panel ${workbenchError ? "has-error" : ""}`}
      >
        {workbenchError && <p className="workspace-error">{workbenchError}</p>}
        <div
          className={`workbench-tab-panel stage workbench-tab-panel--${activeTab}`}
        >
          {tabContent}
        </div>
      </section>
    );
  }

  return (
    <>
      <section className="panel-card workspace-summary-card">
        <div className="panel-title">
          <FileCode2 size={16} />
          Workspace
        </div>
        <p className="workspace-summary-path" title={shownWorkspaceRoot ?? ""}>
          {shownWorkspaceRoot ?? "No workspace selected."}
        </p>
        {shownWorkspaceRoot && (
          <div className="workspace-actions">
            <button
              className="secondary-button"
              onClick={() => onOpenPath(shownWorkspaceRoot)}
            >
              <FolderOpen size={15} />
              Open
            </button>
            <button
              className="secondary-button"
              disabled={isRefreshingWorkbench || !thread}
              onClick={onRefreshWorkbench}
            >
              <RefreshCw
                size={15}
                className={isRefreshingWorkbench ? "spin" : ""}
              />
              Refresh
            </button>
          </div>
        )}
        {workbenchError && <p className="workspace-error">{workbenchError}</p>}
      </section>

      <ContextCard
        contextStatus={contextStatus}
        disabled={!thread || isCompactingContext}
        isCompacting={isCompactingContext}
        onCompactContext={onCompactContext}
      />

      <section className="panel-card workbench-panel-card">
        <div className="workbench-tabs" role="tablist" aria-label="Workbench">
          {tabs.map((tab) => {
            const Icon = tab.icon;
            return (
              <button
                className={`workbench-tab ${activeTab === tab.id ? "active" : ""}`}
                key={tab.id}
                type="button"
                role="tab"
                aria-selected={activeTab === tab.id}
                title={tab.label}
                onClick={() => setInternalActiveTab(tab.id)}
              >
                <Icon size={14} />
                <span>{tab.label}</span>
              </button>
            );
          })}
        </div>

        <div className="workbench-tab-panel">{tabContent}</div>
      </section>

      <ArtifactGallery
        artifacts={artifacts}
        onGetArtifact={onGetArtifact}
        threadId={thread?.id ?? null}
        onOpenPath={onOpenPath}
      />

      {latestApprovalPayload && (
        <section className="panel-card approval-card">
          <div className="panel-title">
            <ShieldAlert size={16} />
            Approval Needed
          </div>
          <p>{latestApprovalPayload.reason}</p>
          <code>{latestApprovalPayload.action}</code>
          <div className="approval-actions">
            <button
              className="secondary-button"
              disabled={
                decidingApprovalId === latestApprovalPayload.approval_id
              }
              onClick={() =>
                onDecideApproval(latestApprovalPayload.approval_id, false)
              }
            >
              Deny
            </button>
            <button
              className="primary-button"
              disabled={
                decidingApprovalId === latestApprovalPayload.approval_id
              }
              onClick={() =>
                onDecideApproval(latestApprovalPayload.approval_id, true)
              }
            >
              Allow Once
            </button>
          </div>
        </section>
      )}
    </>
  );
}

function ContextCard({
  contextStatus,
  disabled,
  isCompacting,
  onCompactContext,
}: {
  contextStatus: ContextStatus | null;
  disabled: boolean;
  isCompacting: boolean;
  onCompactContext(): void;
}) {
  const budget = contextStatus?.budget;
  const usage = budget?.estimatedUsage ?? 0;
  const latestSummary = contextStatus?.latestSummary;
  const providerUsage = contextStatus?.usage;
  const projection = contextStatus?.projection;

  return (
    <section className="panel-card context-card">
      <div className="panel-title">
        <FileText size={16} />
        Context
      </div>
      <div className="context-budget-row">
        <span>{budget ? `${usage}% used` : "No estimate"}</span>
        <span>{budget ? `${budget.messageCount} messages` : "No thread"}</span>
        {budget && <span>{formatNumber(budget.usedTokens)} tokens</span>}
      </div>
      {budget && (
        <div className="context-meter" aria-label="Context usage">
          <span style={{ width: `${Math.min(usage, 100)}%` }} />
        </div>
      )}
      {providerUsage && providerUsage.modelRequests > 0 ? (
        <div className="context-budget-row">
          <span>{providerUsage.modelRequests} requests</span>
          <span>{formatNumber(providerUsage.cachedInputTokens)} cached</span>
          {providerUsage.cacheWriteTokens > 0 ? (
            <span>{formatNumber(providerUsage.cacheWriteTokens)} written</span>
          ) : null}
          {providerUsage.compactions > 0 ? (
            <span>{providerUsage.compactions} compactions</span>
          ) : null}
          {providerUsage.lastFactRetentionPercent > 0 ? (
            <span>
              {providerUsage.lastFactRetentionPercent}% facts retained
            </span>
          ) : null}
          {providerUsage.nativeCompactions > 0 ? (
            <span>{providerUsage.nativeCompactions} native</span>
          ) : null}
          {providerUsage.providerFallbacks > 0 ? (
            <span>{providerUsage.providerFallbacks} fallbacks</span>
          ) : null}
          {providerUsage.warnings > 0 ? (
            <span>{providerUsage.warnings} warnings</span>
          ) : null}
        </div>
      ) : null}
      {projection ? (
        <div className="context-budget-row">
          <Badge variant="info">
            {(projection.checkpointMode ?? "uncompacted").replaceAll("_", " ")}
          </Badge>
          <Badge
            variant={projection.providerStateAvailable ? "success" : "neutral"}
          >
            {projection.providerStateAvailable
              ? (projection.providerStateKind ?? "provider state").replaceAll(
                  "_",
                  " ",
                )
              : projection.nativeCompactionSupported
                ? "native ready"
                : "local checkpoint"}
          </Badge>
          <Badge>{formatNumber(projection.checkpointTokens)} checkpoint</Badge>
          <Badge>{formatNumber(projection.recentTailTokens)} recent</Badge>
          {projection.unsummarizedMessageCount > 0 ? (
            <Badge variant="warning">
              {projection.unsummarizedMessageCount} messages uncovered
            </Badge>
          ) : null}
          {projection.unsummarizedEventCount > 0 ? (
            <Badge variant="warning">
              {projection.unsummarizedEventCount} events uncovered
            </Badge>
          ) : null}
        </div>
      ) : null}
      {latestSummary ? (
        <details className="context-summary">
          <summary>
            Checkpoint through event {latestSummary.coveredThroughSeq}
            <ChevronDown size={12} />
          </summary>
          <p>{latestSummary.checkpoint?.goal ?? latestSummary.summary}</p>
        </details>
      ) : (
        <p>No context checkpoint yet.</p>
      )}
      <Button
        size="compact"
        variant="secondary"
        disabled={disabled}
        onClick={onCompactContext}
      >
        <RefreshCw size={13} className={isCompacting ? "spin" : ""} />
        {isCompacting ? "Compacting" : "Compact"}
      </Button>
    </section>
  );
}

function FilesView({
  client,
  threadId,
  workspaceRoot,
  workspaceTree,
  filePreview,
  isRefreshing,
  onRefresh,
  onOpenWorkspacePath,
  onOpenWorkspaceEntry,
  onOpenPath,
}: {
  client: ApiClient | null;
  threadId: string | null;
  workspaceRoot: string | null;
  workspaceTree: WorkspaceTree | null;
  filePreview: WorkspaceFilePreview | null;
  isRefreshing: boolean;
  onRefresh(): void;
  onOpenWorkspacePath(path?: string): void;
  onOpenWorkspaceEntry(entry: WorkspaceEntry): void;
  onOpenPath(targetPath: string): void;
}) {
  const currentPath = workspaceTree?.path ?? "";
  const entries = workspaceTree?.entries ?? [];
  const [query, setQuery] = useState("");
  const [expandedPaths, setExpandedPaths] = useState<ReadonlySet<string>>(
    new Set(),
  );
  const [childrenByPath, setChildrenByPath] = useState<
    Readonly<Record<string, WorkspaceEntry[]>>
  >({});
  const [loadingPaths, setLoadingPaths] = useState<ReadonlySet<string>>(
    new Set(),
  );
  const [treeError, setTreeError] = useState<string | null>(null);
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const treeRows = useMemo(
    () => flattenWorkspaceTree(entries, expandedPaths, childrenByPath),
    [childrenByPath, entries, expandedPaths],
  );
  const visibleRows = normalizedQuery
    ? treeRows.filter(({ entry }) =>
        `${entry.name} ${entry.path}`
          .toLocaleLowerCase()
          .includes(normalizedQuery),
      )
    : treeRows;

  useEffect(() => {
    setExpandedPaths(new Set());
    setChildrenByPath({});
    setLoadingPaths(new Set());
    setTreeError(null);
  }, [currentPath, threadId]);

  const toggleDirectory = useCallback(
    async (entry: WorkspaceEntry) => {
      if (entry.kind !== "directory") return;
      if (expandedPaths.has(entry.path)) {
        setExpandedPaths((current) => {
          const next = new Set(current);
          next.delete(entry.path);
          return next;
        });
        return;
      }

      setExpandedPaths((current) => new Set(current).add(entry.path));
      if (childrenByPath[entry.path] || !client || !threadId) return;

      setLoadingPaths((current) => new Set(current).add(entry.path));
      setTreeError(null);
      try {
        const tree = await client.listWorkspaceTree(threadId, entry.path);
        setChildrenByPath((current) => ({
          ...current,
          [entry.path]: tree.entries,
        }));
      } catch (cause) {
        setExpandedPaths((current) => {
          const next = new Set(current);
          next.delete(entry.path);
          return next;
        });
        setTreeError(errorMessage(cause));
      } finally {
        setLoadingPaths((current) => {
          const next = new Set(current);
          next.delete(entry.path);
          return next;
        });
      }
    },
    [childrenByPath, client, expandedPaths, threadId],
  );

  const refreshTree = useCallback(() => {
    setExpandedPaths(new Set());
    setChildrenByPath({});
    setLoadingPaths(new Set());
    setTreeError(null);
    onRefresh();
  }, [onRefresh]);

  return (
    <div className="files-view">
      <div className="file-browser-toolbar">
        <Breadcrumb
          path={filePreview?.path ?? currentPath}
          rootLabel={workspaceRoot ? workspaceName(workspaceRoot) : "工作区"}
          leafIsFile={Boolean(filePreview)}
          onOpenPath={onOpenWorkspacePath}
        />
        {filePreview?.truncated ? (
          <Badge variant="warning">已截断</Badge>
        ) : null}
        {filePreview?.readonly ? <Badge>只读</Badge> : null}
        <IconButton
          size="compact"
          variant="quiet"
          aria-label="刷新文件"
          title="刷新文件"
          disabled={isRefreshing}
          onClick={refreshTree}
        >
          <RefreshCw size={13} className={isRefreshing ? "spin" : ""} />
        </IconButton>
        <IconButton
          size="compact"
          variant="quiet"
          aria-label="在系统中打开工作区"
          title="在系统中打开工作区"
          disabled={!workspaceRoot}
          onClick={() => workspaceRoot && onOpenPath(workspaceRoot)}
        >
          <FolderOpen size={14} aria-hidden="true" />
        </IconButton>
        {filePreview ? (
          <Button
            className="file-browser-open"
            size="compact"
            variant="secondary"
            disabled={!workspaceRoot}
            title={`打开 ${filePreview.path}`}
            onClick={() =>
              workspaceRoot &&
              onOpenPath(
                toWorkspaceAbsolutePath(workspaceRoot, filePreview.path),
              )
            }
          >
            <ExternalLink size={14} aria-hidden="true" />
            <span>打开</span>
          </Button>
        ) : null}
      </div>

      <div className="file-browser-layout">
        <section className="file-preview-workspace" aria-label="文件预览">
          {filePreview ? (
            <div className="workbench-file-editor">
              <MonacoEditor
                value={filePreview.content}
                language={detectLanguage(filePreview.path)}
                readOnly
                theme={
                  document.documentElement.dataset.theme === "dark"
                    ? "vs-dark"
                    : "vs"
                }
              />
            </div>
          ) : (
            <div className="file-preview-empty">
              <FolderOpen size={32} aria-hidden="true" />
              <strong>打开文件</strong>
              <span>从工作区目录树中选择文件</span>
            </div>
          )}
        </section>

        <aside className="file-explorer-pane" aria-label="工作区目录树">
          <label className="file-explorer-search">
            <Search size={14} aria-hidden="true" />
            <span className="sr-only">筛选文件</span>
            <input
              value={query}
              placeholder="筛选文件..."
              onChange={(event) => setQuery(event.target.value)}
            />
          </label>
          <div className="workbench-file-list">
            {visibleRows.length ? (
              visibleRows.map(({ entry, depth }) => {
                const isDirectory = entry.kind === "directory";
                const expanded = expandedPaths.has(entry.path);
                const loading = loadingPaths.has(entry.path);
                return (
                  <button
                    className={`file-row workbench-file-row ${
                      filePreview?.path === entry.path ? "active" : ""
                    }`}
                    key={entry.path}
                    type="button"
                    title={entry.path}
                    aria-expanded={isDirectory ? expanded : undefined}
                    style={{ "--file-tree-depth": depth } as CSSProperties}
                    onClick={() => {
                      if (isDirectory) void toggleDirectory(entry);
                      else onOpenWorkspaceEntry(entry);
                    }}
                  >
                    {loading ? (
                      <Loader2 className="spin" size={14} aria-hidden="true" />
                    ) : isDirectory ? (
                      <ChevronRight
                        className={expanded ? "is-expanded" : ""}
                        size={14}
                        aria-hidden="true"
                      />
                    ) : (
                      <span className="file-explorer-indent" />
                    )}
                    {isDirectory ? (
                      <Folder size={14} aria-hidden="true" />
                    ) : (
                      <FileText size={14} aria-hidden="true" />
                    )}
                    <span>{entry.name}</span>
                    <small>{isDirectory ? "" : formatBytes(entry.size)}</small>
                  </button>
                );
              })
            ) : (
              <span className="file-explorer-empty">
                {entries.length ? "没有匹配的文件。" : "当前目录为空。"}
              </span>
            )}
          </div>
          {treeError ? (
            <p className="file-explorer-error" role="alert">
              {treeError}
            </p>
          ) : null}
        </aside>
      </div>
    </div>
  );
}

function Breadcrumb({
  path,
  rootLabel,
  leafIsFile = false,
  onOpenPath,
}: {
  path: string;
  rootLabel: string;
  leafIsFile?: boolean;
  onOpenPath(path?: string): void;
}) {
  const parts = splitWorkspacePath(path);

  return (
    <nav className="path-breadcrumb" aria-label="Current folder">
      <button type="button" onClick={() => onOpenPath(undefined)}>
        {rootLabel}
      </button>
      {parts.map((part, index) => {
        const partPath = parts.slice(0, index + 1).join("/");
        const isFileLeaf = leafIsFile && index === parts.length - 1;
        return (
          <span key={`${part}-${index}`} className="breadcrumb-part">
            <ChevronRight size={12} aria-hidden="true" />
            {isFileLeaf ? (
              <strong title={partPath}>{part}</strong>
            ) : (
              <button
                type="button"
                title={partPath}
                onClick={() => onOpenPath(partPath)}
              >
                {part}
              </button>
            )}
          </span>
        );
      })}
    </nav>
  );
}

function workspaceName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).at(-1) ?? path;
}

type WorkspaceTreeRow = {
  entry: WorkspaceEntry;
  depth: number;
};

function flattenWorkspaceTree(
  entries: WorkspaceEntry[],
  expandedPaths: ReadonlySet<string>,
  childrenByPath: Readonly<Record<string, WorkspaceEntry[]>>,
  depth = 0,
): WorkspaceTreeRow[] {
  const rows: WorkspaceTreeRow[] = [];
  for (const entry of entries) {
    rows.push({ entry, depth });
    if (entry.kind !== "directory" || !expandedPaths.has(entry.path)) continue;
    rows.push(
      ...flattenWorkspaceTree(
        childrenByPath[entry.path] ?? [],
        expandedPaths,
        childrenByPath,
        depth + 1,
      ),
    );
  }
  return rows;
}

function errorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

type DiffSubTab = "diff" | "review";

function DiffView({
  client,
  threadId,
  events,
  workspaceDiff,
  reviewFileRequest,
  isRefreshing,
  revertingDiffPath,
  hunkActionKey,
  canRunGit,
  onRefresh,
  onOpenFileTab,
  onLoadFileContent,
  onLoadTurnFileDiff,
  onGitAction,
  onRevertDiffFile,
  onApplyDiffHunk,
}: {
  client: ApiClient | null;
  threadId: string | null;
  events: AgentEvent[];
  workspaceDiff: WorkspaceDiff | null;
  reviewFileRequest: ReviewFileRequest | null;
  isRefreshing: boolean;
  revertingDiffPath: string | null;
  hunkActionKey: string | null;
  canRunGit: boolean;
  onRefresh(): void;
  onOpenFileTab(path: string): void;
  onLoadFileContent(path: string): Promise<DiffReviewFileContent>;
  onLoadTurnFileDiff(turnId: string, path: string): Promise<string>;
  onGitAction(action: DiffReviewGitAction, message: string): Promise<string>;
  onRevertDiffFile(path: string): void;
  onApplyDiffHunk(
    hunk: WorkspaceDiffHunk,
    action: WorkspaceDiffHunkAction,
  ): void;
}) {
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [diffSubTab, setDiffSubTab] = useState<DiffSubTab>("diff");
  // A review request usually lands before the diff that contains the file has
  // been fetched, so the path is parked until a diff newer than the one on
  // screen at request time arrives.
  const [pendingReview, setPendingReview] = useState<ReviewFileRequest | null>(
    null,
  );
  const [focusRequest, setFocusRequest] = useState<ReviewFileRequest | null>(
    null,
  );
  const reviewBaselineRef = useRef<WorkspaceDiff | null>(null);
  const workspaceDiffRef = useRef<WorkspaceDiff | null>(workspaceDiff);
  workspaceDiffRef.current = workspaceDiff;

  useEffect(() => {
    if (!reviewFileRequest) return;
    reviewBaselineRef.current = workspaceDiffRef.current;
    setPendingReview(reviewFileRequest);
    setDiffSubTab("diff");
  }, [reviewFileRequest]);

  useEffect(() => {
    if (!pendingReview) return;
    const match = workspaceDiff?.files.find(
      (file) =>
        sameDiffPath(file.path, pendingReview.path) ||
        sameDiffPath(file.originalPath, pendingReview.path),
    );
    if (match) {
      setSelectedPath(match.path);
      setFocusRequest({ path: match.path, nonce: pendingReview.nonce });
      setPendingReview(null);
      return;
    }
    // The file is genuinely absent from a freshly loaded diff (committed,
    // reverted, or outside the workspace): stop waiting for it.
    if (workspaceDiff && workspaceDiff !== reviewBaselineRef.current) {
      setPendingReview(null);
    }
  }, [pendingReview, workspaceDiff]);

  useEffect(() => {
    if (pendingReview) return;
    if (!workspaceDiff?.files.length) {
      setSelectedPath(null);
      return;
    }
    if (
      selectedPath &&
      !workspaceDiff.files.some((file) => file.path === selectedPath)
    ) {
      setSelectedPath(workspaceDiff.files[0].path);
    }
  }, [pendingReview, selectedPath, workspaceDiff]);

  const turnScopes = useMemo(() => buildTurnReviewScopes(events), [events]);
  const selectedFile =
    selectedPath && workspaceDiff
      ? (workspaceDiff.files.find((file) => file.path === selectedPath) ?? null)
      : null;

  const revertBlockedReason = useCallback(
    (path: string) => {
      const file = workspaceDiff?.files.find((entry) =>
        sameDiffPath(entry.path, path),
      );
      if (!file) return "该文件不在当前工作区改动中。";
      return restoreDisabledReason(file);
    },
    [workspaceDiff],
  );

  return (
    <div className="diff-view">
      {diffSubTab === "diff" && (
        <DiffReviewPanel
          workspaceDiff={workspaceDiff}
          turnScopes={turnScopes}
          focusRequest={focusRequest}
          isRefreshing={isRefreshing}
          revertingPath={revertingDiffPath}
          canRunGit={canRunGit}
          onRefresh={onRefresh}
          onOpenFileTab={onOpenFileTab}
          onLoadFileContent={onLoadFileContent}
          onLoadTurnFileDiff={onLoadTurnFileDiff}
          onRevertFile={onRevertDiffFile}
          revertBlockedReason={revertBlockedReason}
          onGitAction={onGitAction}
          onListGitBranches={() =>
            client && threadId
              ? client.listGitBranches(threadId)
              : Promise.resolve([])
          }
          onSwitchGitBranch={async (branch) => {
            if (!client || !threadId) {
              throw new Error("当前任务没有可用的 Git 工作区。");
            }
            await client.runGitWorkflow(threadId, {
              type: "switch_branch",
              request: { branch },
            });
            onRefresh();
          }}
        />
      )}

      {diffSubTab === "review" &&
        (workspaceDiff ? (
          <>
            <div className="diff-summary-row">
              <span>{workspaceDiff.files.length} changed</span>
              <span>{workspaceDiff.command}</span>
              {workspaceDiff.truncated && (
                <span className="truncated-pill">Truncated</span>
              )}
              {workspaceDiff.stagedTruncated && (
                <span className="truncated-pill">Staged truncated</span>
              )}
              {workspaceDiff.unstagedTruncated && (
                <span className="truncated-pill">Unstaged truncated</span>
              )}
            </div>
            <ReviewPanel
              workspaceDiff={workspaceDiff}
              selectedPath={selectedPath}
              selectedFile={selectedFile}
              revertingDiffPath={revertingDiffPath}
              hunkActionKey={hunkActionKey}
              onSelectPath={setSelectedPath}
              onRevertDiffFile={onRevertDiffFile}
              onApplyDiffHunk={onApplyDiffHunk}
            />
          </>
        ) : (
          <div className="workbench-empty-state">No diff loaded.</div>
        ))}
    </div>
  );
}

/**
 * Review baselines taken from the turns this thread already recorded, newest
 * first. Only finalized change sets can be reviewed; a capturing or failed one
 * has no file list to read.
 */
function buildTurnReviewScopes(events: AgentEvent[]): DiffReviewTurnScope[] {
  const byTurn = new Map<string, DiffReviewTurnScope>();
  for (const event of events) {
    if (event.payload.type !== "turn_changes_recorded") continue;
    const changeSet = event.payload.change_set;
    if (!shouldShowRecordedTurnChanges(events, changeSet.turnId)) continue;
    if (changeSet.status !== "ready") continue;
    const files = changeSet.files
      .map((file) => ({
        path: file.newPath ?? file.oldPath ?? "",
        binary: file.binary,
      }))
      .filter((file) => file.path);
    if (!files.length) continue;
    byTurn.set(changeSet.turnId, {
      turnId: changeSet.turnId,
      label: `${formatTime(changeSet.createdAt)} 的回合`,
      additions: changeSet.additions,
      deletions: changeSet.deletions,
      files,
    });
  }
  return [...byTurn.values()].reverse();
}

function ReviewPanel({
  workspaceDiff,
  selectedPath,
  selectedFile,
  revertingDiffPath,
  hunkActionKey,
  onSelectPath,
  onRevertDiffFile,
  onApplyDiffHunk,
}: {
  workspaceDiff: WorkspaceDiff;
  selectedPath: string | null;
  selectedFile: ChangedFile | null;
  revertingDiffPath: string | null;
  hunkActionKey: string | null;
  onSelectPath(path: string): void;
  onRevertDiffFile(path: string): void;
  onApplyDiffHunk(
    hunk: WorkspaceDiffHunk,
    action: WorkspaceDiffHunkAction,
  ): void;
}) {
  const [confirmRevert, setConfirmRevert] = useState(false);
  const hunks = useMemo(
    () => reviewHunksForSelection(workspaceDiff, selectedFile),
    [selectedFile, workspaceDiff],
  );
  const revertDisabledReason = selectedFile
    ? restoreDisabledReason(selectedFile)
    : "Choose one changed file to restore.";
  const canRevert = Boolean(selectedFile && !revertDisabledReason);
  const isReverting = selectedFile?.path === revertingDiffPath;

  useEffect(() => {
    setConfirmRevert(false);
  }, [selectedPath]);

  return (
    <div className="diff-review-panel">
      <div className="diff-review-files">
        <span className="diff-review-section-label">Modified files</span>
        <div className="changed-file-list">
          {workspaceDiff.files.map((file) => (
            <button
              className={`changed-file-row ${file.path === selectedPath ? "active" : ""}`}
              key={`${file.status}-${file.path}`}
              type="button"
              title={file.path}
              onClick={() => onSelectPath(file.path)}
            >
              <ChangedFileStatusBadges file={file} />
              <span>{file.path}</span>
            </button>
          ))}
        </div>
      </div>

      <div className="diff-action-boundary">
        <div>
          <strong>{selectedFile?.path ?? "Select a file"}</strong>
          <span>
            {selectedFile
              ? canRevert
                ? "Restores this tracked working-tree file to HEAD with git restore --source=HEAD --worktree -- <path>."
                : revertDisabledReason
              : "Choose one changed file to review actions."}
          </span>
        </div>
        <label className="diff-confirm-row">
          <input
            type="checkbox"
            checked={confirmRevert}
            disabled={!canRevert || isReverting}
            onChange={(event) => setConfirmRevert(event.target.checked)}
          />
          Confirm restore to HEAD
        </label>
        <button
          className="secondary-button compact"
          type="button"
          disabled={
            !selectedFile || !canRevert || !confirmRevert || isReverting
          }
          onClick={() => selectedFile && onRevertDiffFile(selectedFile.path)}
        >
          <RotateCcw size={12} className={isReverting ? "spin" : ""} />
          {isReverting ? "Restoring" : "Restore worktree"}
        </button>
      </div>

      <div className="diff-review-hunks">
        <span className="diff-review-section-label">
          Patch hunks ({hunks.length})
        </span>
        {hunks.length ? (
          hunks.map((hunk, index) => {
            const primaryAction: WorkspaceDiffHunkAction =
              hunk.scope === "staged" ? "unstage" : "stage";
            const primaryKey = diffHunkActionKey(hunk, primaryAction);
            const discardKey = diffHunkActionKey(hunk, "discard");
            const isBusy =
              hunkActionKey === primaryKey || hunkActionKey === discardKey;
            return (
              <div
                className="diff-review-hunk"
                key={`${hunk.scope}-${hunk.path}-${hunk.header}-${index}`}
              >
                <div className="diff-review-hunk-header">
                  <div className="diff-review-hunk-title">
                    <span className={`diff-status ${statusClass(hunk.scope)}`}>
                      {hunk.scope}
                    </span>
                    <code title={`${hunk.path} ${hunk.header}`}>
                      {hunk.path} {hunk.header}
                    </code>
                  </div>
                  <div className="diff-review-actions">
                    <button
                      className="secondary-button compact"
                      type="button"
                      disabled={isBusy}
                      onClick={() => onApplyDiffHunk(hunk, primaryAction)}
                    >
                      <Check size={12} />
                      {hunkActionKey === primaryKey
                        ? "Applying"
                        : primaryAction === "stage"
                          ? "Stage"
                          : "Unstage"}
                    </button>
                    {hunk.scope === "unstaged" && (
                      <button
                        className="secondary-button compact danger"
                        type="button"
                        disabled={isBusy}
                        onClick={() => onApplyDiffHunk(hunk, "discard")}
                      >
                        <XCircle size={12} />
                        {hunkActionKey === discardKey
                          ? "Discarding"
                          : "Discard"}
                      </button>
                    )}
                  </div>
                </div>
                <pre className="diff-review-hunk-body">
                  {hunk.lines.join("\n")}
                </pre>
              </div>
            );
          })
        ) : (
          <span className="muted">No hunks parsed.</span>
        )}
      </div>
    </div>
  );
}

function ChangedFileStatusBadges({ file }: { file: ChangedFile }) {
  return (
    <span
      className="diff-status-group"
      aria-label={changedFileStatusTitle(file)}
    >
      {changedFileBadges(file).map((badge) => (
        <span
          className={`diff-status ${statusClass(badge.className)}`}
          key={`${badge.label}-${badge.title}`}
          title={badge.title}
        >
          {badge.label}
        </span>
      ))}
    </span>
  );
}

function TerminalView({
  thread,
  events,
  terminalEvents,
  terminalSession,
  onEnsureSession,
  onWriteSession,
  onResizeSession,
  onCloseSession,
  onOpenArtifact,
}: {
  thread: Thread | null;
  events: AgentEvent[];
  terminalEvents: TerminalEvent[];
  terminalSession: TerminalSession | null;
  onEnsureSession(threadId: string): Promise<TerminalSession>;
  onWriteSession(threadId: string, sessionId: string, data: string): void;
  onResizeSession(
    threadId: string,
    sessionId: string,
    cols: number,
    rows: number,
  ): void;
  onCloseSession(threadId: string, sessionId: string): void;
  onOpenArtifact(threadId: string, artifactId: string): void;
}) {
  const xtermRef = useRef<XtermTerminalHandle | null>(null);
  const readyTerminalRef = useRef<XtermTerminalHandle | null>(null);
  const writtenTerminalEventsRef = useRef<Set<string>>(new Set());
  const lastThreadIdRef = useRef<string | null>(null);
  const inputBufferRef = useRef("");
  const inputTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const resizeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [isStartingSession, setIsStartingSession] = useState(false);
  const terminalRows = useMemo(
    () => buildCombinedTerminalRows(events, terminalEvents),
    [events, terminalEvents],
  );
  const inputDisabled = !thread || !terminalSession || isStartingSession;

  useEffect(() => {
    const threadId = thread?.id ?? null;
    if (lastThreadIdRef.current === threadId) return;
    lastThreadIdRef.current = threadId;
    writtenTerminalEventsRef.current = new Set();
    readyTerminalRef.current?.clear();
  }, [thread?.id]);

  useEffect(() => {
    writePendingTerminalEvents(
      terminalEvents,
      readyTerminalRef.current,
      writtenTerminalEventsRef.current,
      writeTerminalEventToXterm,
    );
  }, [terminalEvents]);

  const handleTerminalReady = useCallback(
    (terminal: XtermTerminalHandle | null) => {
      readyTerminalRef.current = terminal;
      if (!terminal) return;
      const written = new Set<string>();
      writtenTerminalEventsRef.current = written;
      terminal.clear();
      writePendingTerminalEvents(
        terminalEvents,
        terminal,
        written,
        writeTerminalEventToXterm,
      );
    },
    [terminalEvents],
  );

  const handleData = useCallback(
    (data: string) => {
      if (!thread || !terminalSession) return;
      inputBufferRef.current += data;
      if (inputTimerRef.current) return;
      inputTimerRef.current = setTimeout(() => {
        inputTimerRef.current = null;
        const pending = inputBufferRef.current;
        inputBufferRef.current = "";
        if (pending) {
          onWriteSession(thread.id, terminalSession.sessionId, pending);
        }
      }, 12);
    },
    [onWriteSession, terminalSession, thread],
  );

  const handleResize = useCallback(
    (cols: number, rows: number) => {
      if (!thread || !terminalSession) return;
      if (resizeTimerRef.current) clearTimeout(resizeTimerRef.current);
      resizeTimerRef.current = setTimeout(() => {
        resizeTimerRef.current = null;
        onResizeSession(thread.id, terminalSession.sessionId, cols, rows);
      }, 80);
    },
    [onResizeSession, terminalSession, thread],
  );

  useEffect(
    () => () => {
      if (inputTimerRef.current) clearTimeout(inputTimerRef.current);
      if (resizeTimerRef.current) clearTimeout(resizeTimerRef.current);
    },
    [],
  );

  const handleRestart = useCallback(() => {
    if (!thread || isStartingSession) return;
    setIsStartingSession(true);
    void onEnsureSession(thread.id)
      .then(() => xtermRef.current?.focus())
      .finally(() => setIsStartingSession(false));
  }, [isStartingSession, onEnsureSession, thread]);

  return (
    <div className="terminal-view">
      <div className="terminal-toolbar" role="toolbar" aria-label="终端控制">
        <span
          className="terminal-session-label"
          aria-label={
            terminalSession
              ? `${terminalSession.shell}，正在运行`
              : "终端未启动"
          }
          title={terminalSession?.shell}
        >
          <TerminalSquare size={14} aria-hidden="true" />
          <strong>
            {terminalSession
              ? terminalShellName(terminalSession.shell)
              : "终端未启动"}
          </strong>
          {terminalSession && <Badge variant="success">正在运行</Badge>}
        </span>
        {terminalSession && (
          <span className="terminal-session-cwd" title={terminalSession.cwd}>
            {terminalSession.cwd}
          </span>
        )}
        <span className="terminal-toolbar-spacer" />
        {thread && terminalSession ? (
          <IconButton
            size="compact"
            variant="quiet"
            title="终止终端"
            aria-label="终止终端"
            onClick={() => onCloseSession(thread.id, terminalSession.sessionId)}
          >
            <Square size={14} aria-hidden="true" />
          </IconButton>
        ) : (
          <Button
            size="compact"
            variant="quiet"
            disabled={!thread || isStartingSession}
            onClick={handleRestart}
          >
            {isStartingSession ? (
              <Loader2 className="spin" size={14} aria-hidden="true" />
            ) : (
              <Plus size={14} aria-hidden="true" />
            )}
            {isStartingSession ? "启动中" : "新建终端"}
          </Button>
        )}
        <IconButton
          size="compact"
          variant="quiet"
          title="清空终端"
          aria-label="清空终端"
          onClick={() => xtermRef.current?.clear()}
        >
          <Trash2 size={14} aria-hidden="true" />
        </IconButton>
      </div>
      <div className="xterm-wrapper">
        <XtermTerminal
          ref={xtermRef}
          disabled={inputDisabled}
          onData={handleData}
          onReady={handleTerminalReady}
          onResize={handleResize}
        />
      </div>
      <details className="terminal-history">
        <summary>
          命令历史（{terminalRows.length}）
          <ChevronDown size={12} />
        </summary>
        <div className="terminal-screen" role="log" aria-live="polite">
          {terminalRows.length ? (
            terminalRows.map((row) => (
              <div className={`terminal-row ${row.kind}`} key={row.id}>
                <div className="terminal-row-meta">
                  <span>{row.time}</span>
                  <strong>{row.label}</strong>
                </div>
                {row.body && <pre>{row.body}</pre>}
                {thread && row.artifacts.length > 0 && (
                  <ArtifactReferenceList
                    artifacts={row.artifacts}
                    threadId={thread.id}
                    onOpenArtifact={onOpenArtifact}
                  />
                )}
              </div>
            ))
          ) : (
            <span className="muted">暂无命令历史。</span>
          )}
        </div>
      </details>
    </div>
  );
}

export function terminalShellName(shell: string): string {
  const executable = shell.split(/[\\/]/).at(-1) ?? shell;
  const name = executable.replace(/\.exe$/i, "");
  switch (name.toLowerCase()) {
    case "cmd":
      return "命令提示符";
    case "powershell":
      return "Windows PowerShell";
    case "pwsh":
      return "PowerShell";
    default:
      return name;
  }
}

function writeTerminalEventToXterm(
  event: TerminalEvent,
  terminal: XtermTerminalHandle | null,
) {
  if (!terminal) return;

  switch (event.type) {
    case "started":
      if (event.command && !event.command.startsWith("interactive ")) {
        terminal.write(`$ ${event.command}\r\n`);
      }
      return;
    case "stdout":
      terminal.write(toXtermText(event.data ?? ""));
      return;
    case "stderr":
      terminal.write(`\x1b[31m${toXtermText(event.data ?? "")}\x1b[0m`);
      return;
    case "finished":
      if (event.message) {
        terminal.write(`\r\n\x1b[31m${event.message}\x1b[0m`);
      }
      terminal.write("\r\n");
      return;
    case "cancelled":
      terminal.write(
        `\r\n\x1b[33m${event.message ?? "command cancelled"}\x1b[0m\r\n`,
      );
      return;
    case "error":
      terminal.write(
        `\r\n\x1b[31m${event.message ?? "terminal error"}\x1b[0m\r\n`,
      );
      return;
  }
}

function toXtermText(value: string): string {
  return value.replace(/\r?\n/g, "\r\n");
}

function isTerminalEndEvent(type: TerminalEvent["type"]): boolean {
  return type === "finished" || type === "cancelled" || type === "error";
}

function getRunningTerminalCommandId(events: TerminalEvent[]): string | null {
  const running = new Set<string>();
  for (const event of events) {
    if (event.type === "started") {
      running.add(event.commandId);
    } else if (isTerminalEndEvent(event.type)) {
      running.delete(event.commandId);
    }
  }
  return Array.from(running).at(-1) ?? null;
}

type McpEditorState = {
  serverId: string | null;
  name: string;
  command: string;
  args: string;
  cwd: string;
  envKeys: string;
  timeoutMs: string;
  enabled: boolean;
};

function emptyMcpEditor(): McpEditorState {
  return {
    serverId: null,
    name: "",
    command: "",
    args: "",
    cwd: "",
    envKeys: "",
    timeoutMs: "30000",
    enabled: true,
  };
}

function mcpEditorFor(view: McpServerView): McpEditorState {
  return {
    serverId: view.server.serverId,
    name: view.server.name,
    command: view.server.command,
    args: view.server.args.join("\n"),
    cwd: view.server.cwd ?? "",
    envKeys: view.server.envKeys.join("\n"),
    timeoutMs: String(view.server.timeoutMs),
    enabled: view.server.enabled,
  };
}

function lines(value: string): string[] {
  return value
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
}

function ExtensionsView({
  client,
  hasThread,
  threadId,
  workspaceRoot,
  plugins,
  selectedSkillIds,
  mcpServers,
  threadMcpServers,
  onToggleThreadMcp,
  onCreateMcpServer,
  onUpdateMcpServer,
  onRestartMcpServer,
  onDeleteMcpServer,
  onInstallPlugin,
  onUninstallPlugin,
  onToggleThreadPlugin,
  onUsePluginSkills,
  onOpenPath,
}: {
  client: ApiClient | null;
  hasThread: boolean;
  threadId: string | null;
  workspaceRoot: string | null;
  plugins: PluginView[];
  selectedSkillIds: string[];
  mcpServers: McpServerView[];
  threadMcpServers: ThreadMcpServerView[];
  onToggleThreadMcp(serverId: string, enabled: boolean): void;
  onCreateMcpServer(input: McpServerInput): Promise<void>;
  onUpdateMcpServer(serverId: string, input: McpServerInput): Promise<void>;
  onRestartMcpServer(serverId: string): Promise<void>;
  onDeleteMcpServer(serverId: string): Promise<void>;
  onInstallPlugin(): Promise<void>;
  onUninstallPlugin(pluginId: string): Promise<void>;
  onToggleThreadPlugin(pluginId: string, enabled: boolean): Promise<void>;
  onUsePluginSkills(pluginId: string, enabled: boolean): void;
  onOpenPath(targetPath: string): void;
}) {
  const [view, setView] = useState<"plugins" | "mcp">("plugins");
  const [query, setQuery] = useState("");
  const [source, setSource] = useState<"all" | PluginView["plugin"]["source"]>(
    "all",
  );
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selectedPluginId, setSelectedPluginId] = useState<string | null>(null);
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const filteredPlugins = useMemo(
    () =>
      plugins.filter(({ plugin }) => {
        if (source !== "all" && plugin.source !== source) return false;
        if (!normalizedQuery) return true;
        return `${plugin.displayName} ${plugin.name} ${plugin.description} ${plugin.author} ${plugin.category}`
          .toLocaleLowerCase()
          .includes(normalizedQuery);
      }),
    [normalizedQuery, plugins, source],
  );
  const activeCount = plugins.filter(
    (plugin) =>
      plugin.threadEnabled ||
      plugin.skillIds.some((id) => selectedSkillIds.includes(id)),
  ).length;
  const selectedPlugin = plugins.find(
    (item) => item.plugin.id === selectedPluginId,
  );

  useEffect(() => {
    if (selectedPluginId && !selectedPlugin) setSelectedPluginId(null);
  }, [selectedPlugin, selectedPluginId]);

  async function run(key: string, action: () => Promise<void>) {
    setBusyKey(key);
    setError(null);
    try {
      await action();
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    } finally {
      setBusyKey(null);
    }
  }

  async function removePlugin(view: PluginView) {
    if (
      !window.confirm(
        `Remove "${view.plugin.displayName}" from OpenTopia? The original source folder is not changed.`,
      )
    ) {
      return;
    }
    await run(`remove:${view.plugin.id}`, () =>
      onUninstallPlugin(view.plugin.id),
    );
  }

  if (selectedPlugin) {
    return (
      <PluginControlPanel
        client={client}
        pluginView={selectedPlugin}
        threadId={threadId}
        workspaceRoot={workspaceRoot}
        onBack={() => setSelectedPluginId(null)}
      />
    );
  }

  return (
    <div className="extensions-view plugins-browser">
      <div className="plugin-browser-header">
        <div
          className="plugin-view-switch"
          role="tablist"
          aria-label="Plugin view"
        >
          <button
            className={view === "plugins" ? "active" : ""}
            type="button"
            role="tab"
            aria-selected={view === "plugins"}
            onClick={() => setView("plugins")}
          >
            <Puzzle size={14} />
            Plugins
          </button>
          <button
            className={view === "mcp" ? "active" : ""}
            type="button"
            role="tab"
            aria-selected={view === "mcp"}
            onClick={() => setView("mcp")}
          >
            <Settings2 size={14} />
            MCP servers
          </button>
        </div>
        {view === "plugins" && (
          <button
            className="secondary-button compact plugin-install-button"
            type="button"
            disabled={busyKey !== null}
            onClick={() => void run("install", onInstallPlugin)}
          >
            <PackagePlus size={14} />
            {busyKey === "install" ? "Installing" : "Add local"}
          </button>
        )}
      </div>

      {view === "plugins" ? (
        <>
          <div className="plugin-directory-controls">
            <label className="plugin-search">
              <span className="sr-only">Search plugins</span>
              <Search size={14} />
              <input
                value={query}
                placeholder="Search installed plugins"
                onChange={(event) => setQuery(event.target.value)}
              />
              {query && (
                <button
                  type="button"
                  title="Clear search"
                  aria-label="Clear plugin search"
                  onClick={() => setQuery("")}
                >
                  <X size={13} />
                </button>
              )}
            </label>
            <label className="plugin-scope-filter">
              <span className="sr-only">Plugin source</span>
              <select
                value={source}
                onChange={(event) =>
                  setSource(event.target.value as typeof source)
                }
              >
                <option value="all">All sources</option>
                <option value="bundled">Bundled</option>
                <option value="workspace">Project</option>
                <option value="user">OpenTopia</option>
                <option value="codex">Codex</option>
              </select>
            </label>
          </div>
          <div className="plugin-summary">
            <span>{plugins.length} installed</span>
            <span>{activeCount} in this turn</span>
            <span>{filteredPlugins.length} shown</span>
          </div>
          {error && (
            <p className="workspace-error" role="alert">
              {error}
            </p>
          )}
          <div className="plugin-directory" aria-live="polite">
            {filteredPlugins.length ? (
              filteredPlugins.map((item) => {
                const plugin = item.plugin;
                const skillsSelected =
                  item.skillIds.length > 0 &&
                  item.skillIds.every((id) => selectedSkillIds.includes(id));
                const busy = busyKey?.endsWith(plugin.id) ?? false;
                return (
                  <article
                    className={`plugin-entry ${item.compatible ? "" : "is-incompatible"}`}
                    key={plugin.id}
                    style={{ borderLeftColor: plugin.brandColor ?? undefined }}
                  >
                    <div className="plugin-entry-heading">
                      <span className="plugin-monogram" aria-hidden="true">
                        {plugin.displayName.slice(0, 1).toLocaleUpperCase()}
                      </span>
                      <div className="plugin-entry-title">
                        <strong>{plugin.displayName}</strong>
                        <span>{plugin.description || plugin.name}</span>
                      </div>
                      <span className={`plugin-source is-${plugin.source}`}>
                        {plugin.source === "bundled"
                          ? "Bundled"
                          : plugin.source === "workspace"
                            ? "Project"
                            : plugin.source === "codex"
                              ? "Codex"
                              : "OpenTopia"}
                      </span>
                    </div>
                    <div
                      className="plugin-capabilities"
                      aria-label="Capabilities"
                    >
                      {plugin.skillCount > 0 && (
                        <span>
                          <Workflow size={12} /> {plugin.skillCount} Skills
                        </span>
                      )}
                      {plugin.mcpServerCount > 0 && (
                        <span>
                          <Wrench size={12} /> {plugin.supportedMcpServerCount}/
                          {plugin.mcpServerCount} MCP
                        </span>
                      )}
                      {plugin.nativeCapabilities.length > 0 && (
                        <span>
                          <Wrench size={12} />{" "}
                          {plugin.nativeCapabilities.length} native
                        </span>
                      )}
                      {plugin.source === "bundled" && (
                        <span>
                          {plugin.trust === "trusted_driver"
                            ? "Trusted driver"
                            : plugin.trust === "privileged"
                              ? "Privileged"
                              : "Official"}
                        </span>
                      )}
                      {plugin.hasApps && <span>App</span>}
                      {plugin.version && <span>v{plugin.version}</span>}
                      {plugin.category && <span>{plugin.category}</span>}
                    </div>
                    {plugin.issues.length > 0 && (
                      <details className="plugin-issues">
                        <summary>
                          <ShieldAlert size={13} />
                          {item.compatible
                            ? "Limited support"
                            : "Not available"}
                        </summary>
                        <ul>
                          {plugin.issues.map((issue) => (
                            <li key={issue}>{issue}</li>
                          ))}
                        </ul>
                      </details>
                    )}
                    <div className="plugin-entry-actions">
                      <div className="plugin-primary-actions">
                        {item.skillIds.length > 0 && (
                          <button
                            className={`secondary-button compact ${skillsSelected ? "is-selected" : ""}`}
                            type="button"
                            aria-pressed={skillsSelected}
                            onClick={() =>
                              onUsePluginSkills(plugin.id, !skillsSelected)
                            }
                          >
                            {skillsSelected ? (
                              <Check size={13} />
                            ) : (
                              <Plus size={13} />
                            )}
                            {skillsSelected ? "Skills added" : "Use Skills"}
                          </button>
                        )}
                        {(plugin.supportedMcpServerCount > 0 ||
                          plugin.nativeCapabilities.length > 0) && (
                          <label
                            className="plugin-task-toggle"
                            title={
                              hasThread
                                ? "Enable this plugin's tools for this task"
                                : "Open a task to enable plugin tools"
                            }
                          >
                            <input
                              type="checkbox"
                              checked={item.threadEnabled}
                              disabled={!hasThread || busy}
                              onChange={(event) =>
                                void run(`toggle:${plugin.id}`, () =>
                                  onToggleThreadPlugin(
                                    plugin.id,
                                    event.target.checked,
                                  ),
                                )
                              }
                            />
                            <span>Task tools</span>
                          </label>
                        )}
                      </div>
                      <div className="plugin-secondary-actions">
                        <button
                          className="icon-button"
                          type="button"
                          title="Configure plugin"
                          aria-label={`Configure ${plugin.displayName}`}
                          onClick={() => setSelectedPluginId(plugin.id)}
                        >
                          <Settings2 size={14} />
                        </button>
                        <button
                          className="icon-button"
                          type="button"
                          title="Open plugin folder"
                          aria-label={`Open ${plugin.displayName} folder`}
                          onClick={() => onOpenPath(plugin.path)}
                        >
                          <FolderOpen size={14} />
                        </button>
                        {plugin.managed && (
                          <button
                            className="icon-button danger"
                            type="button"
                            title="Remove plugin"
                            aria-label={`Remove ${plugin.displayName}`}
                            disabled={busyKey !== null}
                            onClick={() => void removePlugin(item)}
                          >
                            <Trash2 size={14} />
                          </button>
                        )}
                      </div>
                    </div>
                  </article>
                );
              })
            ) : (
              <div className="workbench-empty-state plugin-empty-state">
                <Puzzle size={22} />
                <strong>
                  {plugins.length ? "No plugins match" : "No plugins installed"}
                </strong>
                <span>
                  {plugins.length
                    ? "Try another search or source."
                    : "Add a local Codex-compatible plugin folder."}
                </span>
              </div>
            )}
          </div>
        </>
      ) : (
        <McpServersView
          hasThread={hasThread}
          mcpServers={mcpServers}
          threadMcpServers={threadMcpServers}
          onToggleThreadMcp={onToggleThreadMcp}
          onCreateMcpServer={onCreateMcpServer}
          onUpdateMcpServer={onUpdateMcpServer}
          onRestartMcpServer={onRestartMcpServer}
          onDeleteMcpServer={onDeleteMcpServer}
        />
      )}
    </div>
  );
}

function McpServersView({
  hasThread,
  mcpServers,
  threadMcpServers,
  onToggleThreadMcp,
  onCreateMcpServer,
  onUpdateMcpServer,
  onRestartMcpServer,
  onDeleteMcpServer,
}: {
  hasThread: boolean;
  mcpServers: McpServerView[];
  threadMcpServers: ThreadMcpServerView[];
  onToggleThreadMcp(serverId: string, enabled: boolean): void;
  onCreateMcpServer(input: McpServerInput): Promise<void>;
  onUpdateMcpServer(serverId: string, input: McpServerInput): Promise<void>;
  onRestartMcpServer(serverId: string): Promise<void>;
  onDeleteMcpServer(serverId: string): Promise<void>;
}) {
  const [editor, setEditor] = useState<McpEditorState | null>(null);
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const bindings = new Map(
    threadMcpServers.map((item) => [item.server.serverId, item]),
  );
  const enabledCount = threadMcpServers.filter((item) => item.enabled).length;

  async function run(key: string, action: () => Promise<void>) {
    setBusyKey(key);
    setError(null);
    try {
      await action();
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
      throw caught;
    } finally {
      setBusyKey(null);
    }
  }

  async function submitEditor(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!editor) return;
    const name = editor.name.trim();
    const command = editor.command.trim();
    if (!name || !command) {
      setError("Name and command are required.");
      return;
    }
    const input: McpServerInput = {
      name,
      command,
      args: lines(editor.args),
      cwd: editor.cwd.trim() || undefined,
      envKeys: lines(editor.envKeys),
      timeoutMs: Number(editor.timeoutMs) || 30_000,
      enabled: editor.enabled,
    };
    const key = editor.serverId ? `save:${editor.serverId}` : "create";
    try {
      await run(key, () =>
        editor.serverId
          ? onUpdateMcpServer(editor.serverId, input)
          : onCreateMcpServer(input),
      );
      setEditor(null);
    } catch {
      // The inline error keeps the form open for correction.
    }
  }

  async function restart(serverId: string) {
    try {
      await run(`restart:${serverId}`, () => onRestartMcpServer(serverId));
    } catch {
      // The server row keeps its last known state and exposes the error below.
    }
  }

  async function remove(view: McpServerView) {
    if (!window.confirm(`Delete MCP server "${view.server.name}"?`)) return;
    try {
      await run(`delete:${view.server.serverId}`, () =>
        onDeleteMcpServer(view.server.serverId),
      );
      if (editor?.serverId === view.server.serverId) setEditor(null);
    } catch {
      // The inline error provides the recovery path.
    }
  }

  return (
    <div className="extensions-view">
      <div className="extensions-toolbar">
        <div className="diff-summary-row">
          <span>{mcpServers.length} available</span>
          <span>{enabledCount} enabled</span>
        </div>
        <button
          className="icon-button"
          type="button"
          title="Add MCP server"
          aria-label="Add MCP server"
          onClick={() => {
            setError(null);
            setEditor(emptyMcpEditor());
          }}
        >
          <Plus size={15} />
        </button>
      </div>

      {editor && (
        <form className="mcp-editor" onSubmit={submitEditor}>
          <div className="mcp-editor-header">
            <strong>
              {editor.serverId ? "Edit MCP server" : "New MCP server"}
            </strong>
            <button
              className="icon-button"
              type="button"
              title="Close editor"
              aria-label="Close MCP editor"
              onClick={() => setEditor(null)}
            >
              <X size={15} />
            </button>
          </div>
          <div className="mcp-editor-grid">
            <label>
              <span>Name</span>
              <input
                autoFocus
                value={editor.name}
                onChange={(event) =>
                  setEditor({ ...editor, name: event.target.value })
                }
              />
            </label>
            <label>
              <span>Command</span>
              <input
                value={editor.command}
                onChange={(event) =>
                  setEditor({ ...editor, command: event.target.value })
                }
              />
            </label>
            <label>
              <span>Arguments</span>
              <textarea
                rows={3}
                title="One argument per line"
                value={editor.args}
                onChange={(event) =>
                  setEditor({ ...editor, args: event.target.value })
                }
              />
            </label>
            <label>
              <span>Environment keys</span>
              <textarea
                rows={3}
                title="One inherited environment variable name per line"
                value={editor.envKeys}
                onChange={(event) =>
                  setEditor({ ...editor, envKeys: event.target.value })
                }
              />
            </label>
            <label>
              <span>Working directory</span>
              <input
                value={editor.cwd}
                onChange={(event) =>
                  setEditor({ ...editor, cwd: event.target.value })
                }
              />
            </label>
            <label>
              <span>Timeout (ms)</span>
              <input
                type="number"
                min={1000}
                max={300000}
                step={1000}
                value={editor.timeoutMs}
                onChange={(event) =>
                  setEditor({ ...editor, timeoutMs: event.target.value })
                }
              />
            </label>
          </div>
          <div className="mcp-editor-actions">
            <label className="mcp-enabled-toggle">
              <input
                type="checkbox"
                checked={editor.enabled}
                onChange={(event) =>
                  setEditor({ ...editor, enabled: event.target.checked })
                }
              />
              <span>Enabled</span>
            </label>
            <button
              className="primary-button"
              type="submit"
              disabled={busyKey !== null}
            >
              <Save size={14} />
              {busyKey?.startsWith("save:") || busyKey === "create"
                ? "Saving"
                : "Save"}
            </button>
          </div>
        </form>
      )}

      {error && (
        <p className="workspace-error" role="alert">
          {error}
        </p>
      )}

      <div className="extension-list">
        {mcpServers.length ? (
          mcpServers.map((view) => {
            const serverId = view.server.serverId;
            const binding = bindings.get(serverId);
            const rowBusy = busyKey?.endsWith(serverId) ?? false;
            return (
              <div className="extension-row" key={serverId}>
                <input
                  type="checkbox"
                  aria-label={`Enable ${view.server.name} for this thread`}
                  checked={binding?.enabled ?? false}
                  disabled={!hasThread || !view.server.enabled || rowBusy}
                  onChange={(event) =>
                    onToggleThreadMcp(serverId, event.target.checked)
                  }
                />
                <div className="extension-main">
                  <span>{view.server.name}</span>
                  <small title={view.server.command}>
                    {view.server.command}
                  </small>
                </div>
                <em
                  className={`mcp-status is-${view.status.status}`}
                  title={view.status.message}
                >
                  {view.status.status}
                  {view.status.toolsCount ? ` · ${view.status.toolsCount}` : ""}
                </em>
                <div className="extension-actions">
                  <button
                    className="icon-button"
                    type="button"
                    title="Edit MCP server"
                    aria-label={`Edit ${view.server.name}`}
                    disabled={busyKey !== null}
                    onClick={() => {
                      setError(null);
                      setEditor(mcpEditorFor(view));
                    }}
                  >
                    <Pencil size={14} />
                  </button>
                  <button
                    className="icon-button"
                    type="button"
                    title="Restart MCP server"
                    aria-label={`Restart ${view.server.name}`}
                    disabled={busyKey !== null || !view.server.enabled}
                    onClick={() => void restart(serverId)}
                  >
                    <RefreshCw
                      className={
                        busyKey === `restart:${serverId}` ? "spinning" : ""
                      }
                      size={14}
                    />
                  </button>
                  <button
                    className="icon-button danger"
                    type="button"
                    title="Delete MCP server"
                    aria-label={`Delete ${view.server.name}`}
                    disabled={busyKey !== null}
                    onClick={() => void remove(view)}
                  >
                    <Trash2 size={14} />
                  </button>
                </div>
              </div>
            );
          })
        ) : (
          <div className="workbench-empty-state">
            No MCP servers configured.
          </div>
        )}
      </div>
    </div>
  );
}

function SandboxView({ sandbox }: { sandbox: SandboxDescriptor | null }) {
  if (!sandbox) {
    return <div className="workbench-empty-state">No sandbox loaded.</div>;
  }

  const workspaceRoot = formatPathForDisplay(sandbox.workspaceRoot);
  const readableRoots = sandbox.readableRoots.map(formatPathForDisplay);
  const writableRoots = sandbox.writableRoots.map(formatPathForDisplay);
  const protectedPaths = sandbox.protectedPaths.map(formatPathForDisplay);

  return (
    <div className="sandbox-view">
      <div className="sandbox-status">
        <span>{sandbox.kind}</span>
        <span>{sandbox.lifecycle}</span>
        <span>{sandbox.sandboxMode}</span>
        <span>{sandbox.enforced ? sandbox.mode : "not enforced"}</span>
        <span>
          {sandbox.network === "deny"
            ? "network denied"
            : `network ${sandbox.network}`}
        </span>
      </div>
      <dl className="sandbox-details">
        <div>
          <dt>Workspace</dt>
          <dd title={workspaceRoot}>{workspaceRoot}</dd>
        </div>
        <div>
          <dt>Sandbox ID</dt>
          <dd title={sandbox.id}>{sandbox.id}</dd>
        </div>
        <div>
          <dt>Backend</dt>
          <dd>{sandbox.backend ?? `${sandbox.platform} unavailable`}</dd>
        </div>
        <div>
          <dt>Permission profile</dt>
          <dd>{sandbox.permissionProfile}</dd>
        </div>
        <div>
          <dt>Readable roots</dt>
          <dd title={readableRoots.join("\n")}>
            {readableRoots.length
              ? readableRoots.join(", ")
              : sandbox.sandboxMode === "danger-full-access"
                ? "unrestricted"
                : "none"}
          </dd>
        </div>
        <div>
          <dt>Writable roots</dt>
          <dd title={writableRoots.join("\n")}>
            {writableRoots.length ? writableRoots.join(", ") : "none"}
          </dd>
        </div>
        <div>
          <dt>Protected metadata</dt>
          <dd title={protectedPaths.join("\n")}>
            {protectedPaths.length ? protectedPaths.join(", ") : "none"}
          </dd>
        </div>
        <div>
          <dt>Capabilities</dt>
          <dd>
            {sandbox.capabilities.length
              ? sandbox.capabilities.join(", ")
              : "none"}
          </dd>
        </div>
      </dl>
      <p>{sandbox.message}</p>
    </div>
  );
}

type TerminalRow = {
  id: string;
  kind: "info" | "command" | "output" | "error";
  label: string;
  time: string;
  body?: string;
  artifacts: ArtifactReference[];
  sortKey?: number;
};

type DiffHunk = {
  path: string;
  scope: "staged" | "unstaged";
  header: string;
  lines: string[];
  raw: string;
  patch?: string;
};

type ArtifactReference = {
  id: string;
  kind?: string;
  bytes?: number;
};

function ArtifactReferenceList({
  artifacts,
  threadId,
  onOpenArtifact,
}: {
  artifacts: ArtifactReference[];
  threadId: string;
  onOpenArtifact(threadId: string, artifactId: string): void;
}) {
  return (
    <div className="artifact-reference-list">
      {artifacts.map((artifact) => (
        <button
          className="artifact-reference-button"
          key={artifact.id}
          type="button"
          title={artifact.id}
          onClick={() => onOpenArtifact(threadId, artifact.id)}
        >
          <ExternalLink size={12} />
          <span>{artifact.kind ?? "artifact"}</span>
          {artifact.bytes !== undefined && (
            <small>{formatBytes(artifact.bytes)}</small>
          )}
        </button>
      ))}
    </div>
  );
}

function reviewHunksForSelection(
  workspaceDiff: WorkspaceDiff,
  selectedFile: ChangedFile | null,
): DiffHunk[] {
  const backendHunks = workspaceDiff.hunks ?? [];
  if (backendHunks.length) {
    return backendHunks
      .filter(
        (hunk) =>
          !selectedFile || sameWorkspacePath(hunk.path, selectedFile.path),
      )
      .map(normalizeWorkspaceHunk);
  }
  const fallbackDiff = selectedFile
    ? diffTextForPath(workspaceDiff, selectedFile.path)
    : workspaceDiff.diff;
  return parseDiffHunks(
    fallbackDiff,
    selectedFile?.path ?? "raw diff",
    "unstaged",
  );
}

function normalizeWorkspaceHunk(hunk: WorkspaceDiffHunk): DiffHunk {
  return {
    path: hunk.path,
    scope: hunk.scope,
    header: hunk.header,
    lines: hunk.lines,
    raw: hunk.raw,
    patch: hunk.patch,
  };
}

function parseDiffHunks(
  diffText: string,
  path: string,
  scope: "staged" | "unstaged",
): DiffHunk[] {
  const hunks: DiffHunk[] = [];
  const lines = diffText.split("\n");
  let currentHunk: DiffHunk | null = null;

  for (const line of lines) {
    if (/^@@ -\d+(,\d*)? \+\d+(,\d*)? @@/.test(line)) {
      if (currentHunk) hunks.push(currentHunk);
      currentHunk = { path, scope, header: line, lines: [], raw: line };
    } else if (currentHunk) {
      currentHunk.lines.push(line);
      currentHunk.raw = `${currentHunk.raw}\n${line}`;
    }
  }
  if (currentHunk) hunks.push(currentHunk);

  return hunks;
}

function diffHunkActionKey(
  hunk: Pick<DiffHunk, "path" | "scope" | "header">,
  action: WorkspaceDiffHunkAction,
): string {
  return `${action}:${hunk.scope}:${hunk.path}:${hunk.header}`;
}

function buildCombinedTerminalRows(
  events: AgentEvent[],
  terminalEvents: TerminalEvent[],
): TerminalRow[] {
  const agentTimes = new Map(
    events.map((event) => [event.id, Date.parse(event.createdAt)]),
  );
  const agentRows = buildTerminalRows(events).map((row) => ({
    ...row,
    sortKey: agentTimes.get(row.id) ?? 0,
  }));
  const terminalRows = buildTerminalEventRows(terminalEvents);
  return [...agentRows, ...terminalRows].sort(
    (left, right) => (left.sortKey ?? 0) - (right.sortKey ?? 0),
  );
}

function buildTerminalEventRows(events: TerminalEvent[]): TerminalRow[] {
  return events.map((event) => {
    const time = formatTime(event.createdAt);
    const sortKey = Date.parse(event.createdAt);
    const base = {
      id: event.id,
      time,
      sortKey,
      artifacts: [],
    };

    switch (event.type) {
      case "started":
        return {
          ...base,
          kind: "command",
          label: `$ ${event.command ?? "terminal command"}`,
          body: event.cwd ? `cwd: ${event.cwd}` : undefined,
        };
      case "stdout":
        return {
          ...base,
          kind: "output",
          label: "terminal stdout",
          body: truncateTerminalOutput(event.data ?? ""),
        };
      case "stderr":
        return {
          ...base,
          kind: "error",
          label: "terminal stderr",
          body: truncateTerminalOutput(event.data ?? ""),
        };
      case "finished":
        return {
          ...base,
          kind: event.success ? "info" : "error",
          label: event.success ? "terminal finished" : "terminal exited",
          body: terminalExitBody(event),
        };
      case "cancelled":
        return {
          ...base,
          kind: "error",
          label: "terminal cancelled",
          body: event.message ?? "command cancelled",
        };
      case "error":
        return {
          ...base,
          kind: "error",
          label: "terminal error",
          body: event.message ?? "terminal error",
        };
    }
  });
}

function terminalExitBody(event: TerminalEvent): string | undefined {
  const parts = [
    event.success === undefined || event.success === null
      ? undefined
      : event.success
        ? "成功"
        : "失败",
    event.message ?? undefined,
  ].filter(Boolean);
  return parts.length ? parts.join("\n") : undefined;
}

function buildTerminalRows(events: AgentEvent[]): TerminalRow[] {
  return events
    .filter((event) => event.payload.type !== "model_delta")
    .map((event) => {
      const time = formatTime(event.createdAt);
      switch (event.payload.type) {
        case "turn_started":
          return {
            id: event.id,
            kind: "info",
            label: "turn started",
            time,
            body: event.payload.user_message_id,
            artifacts: [],
          };
        case "tool_call_started":
          return {
            id: event.id,
            kind: "command",
            label: `$ ${event.payload.call.name}`,
            time,
            body: formatUnknown(event.payload.call.input),
            artifacts: [],
          };
        case "tool_call_finished":
          return {
            id: event.id,
            kind: "output",
            label: "tool output",
            time,
            body: truncateTerminalOutput(event.payload.result.output),
            artifacts: collectArtifactReferences(
              event.payload.result.metadata,
              event.payload.result.output,
            ),
          };
        case "plan_updated":
          return {
            id: event.id,
            kind: "info",
            label: "task plan updated",
            time,
            body: event.payload.plan.steps
              .map(
                (item) =>
                  `[${item.status}] ${item.title || item.step || item.id}`,
              )
              .join("\n"),
            artifacts: [],
          };
        case "assistant_message":
          return {
            id: event.id,
            kind: "info",
            label: "assistant message",
            time,
            artifacts: collectMessageArtifactReferences(event.payload.message),
          };
        case "file_changed":
          return {
            id: event.id,
            kind: "info",
            label: `file changed: ${event.payload.path}`,
            time,
            body: event.payload.summary,
            artifacts: [],
          };
        case "approval_requested":
          return {
            id: event.id,
            kind: "command",
            label: "approval requested",
            time,
            body: `${event.payload.action}\n\n${event.payload.reason}`,
            artifacts: [],
          };
        case "context_compacted":
          return {
            id: event.id,
            kind: "info",
            label: "context compacted",
            time,
            body: event.payload.summary.summary,
            artifacts: [],
          };
        case "context_projection_built":
          return {
            id: event.id,
            kind: "info",
            label: "context projection built",
            time,
            body: `${event.payload.projection.checkpointTokens} checkpoint tokens, ${event.payload.projection.recentTailTokens} recent-tail tokens`,
            artifacts: [],
          };
        case "provider_context_state_updated":
          return {
            id: event.id,
            kind: "info",
            label: "provider context updated",
            time,
            body: `${event.payload.state_kind.replaceAll("_", " ")}, ${event.payload.response_item_count} items`,
            artifacts: [],
          };
        case "provider_context_state_invalidated":
          return {
            id: event.id,
            kind: "info",
            label: "provider context rebuilt",
            time,
            body: event.payload.reason,
            artifacts: [],
          };
        case "turn_finished":
          return {
            id: event.id,
            kind: "info",
            label: "turn finished",
            time,
            body: event.payload.summary,
            artifacts: [],
          };
        case "error":
          return {
            id: event.id,
            kind: "error",
            label: "agent error",
            time,
            body: event.payload.message,
            artifacts: [],
          };
      }
    })
    .filter((row): row is TerminalRow => row !== undefined);
}

function splitWorkspacePath(path: string): string[] {
  if (!path || path === ".") return [];
  return path.split(/[\\/]/).filter(Boolean);
}

function parentPath(path: string): string {
  const parts = splitWorkspacePath(path);
  return parts.slice(0, -1).join("/");
}

function toWorkspaceAbsolutePath(
  workspaceRoot: string,
  targetPath: string,
): string {
  if (!targetPath) return workspaceRoot;
  if (/^[a-zA-Z]:[\\/]/.test(targetPath) || targetPath.startsWith("\\\\")) {
    return targetPath;
  }
  const separator = workspaceRoot.includes("\\") ? "\\" : "/";
  const root = workspaceRoot.replace(/[\\/]+$/, "");
  const child = targetPath.replace(/^[\\/]+/, "").replace(/[\\/]+/g, separator);
  return child ? `${root}${separator}${child}` : root;
}

function formatBytes(value?: number | null): string {
  if (value === undefined || value === null) return "";
  if (value < 1024) return `${value} B`;
  const units = ["KB", "MB", "GB"];
  let amount = value / 1024;
  let unitIndex = 0;
  while (amount >= 1024 && unitIndex < units.length - 1) {
    amount /= 1024;
    unitIndex += 1;
  }
  return `${amount.toFixed(amount >= 10 ? 0 : 1)} ${units[unitIndex]}`;
}

function formatNumber(value: number): string {
  return new Intl.NumberFormat().format(value);
}

function extractDiffForPath(rawDiff: string, path: string): string {
  if (!rawDiff.trim()) return "";
  const normalizedPath = path.replace(/\\/g, "/");
  const chunks = rawDiff.split(/\n(?=diff --git )/);
  const match = chunks.find((chunk) => {
    const normalizedChunk = chunk.replace(/\\/g, "/");
    return (
      normalizedChunk.includes(` a/${normalizedPath}`) ||
      normalizedChunk.includes(` b/${normalizedPath}`) ||
      normalizedChunk.includes(`--- ${normalizedPath}`) ||
      normalizedChunk.includes(`+++ ${normalizedPath}`) ||
      normalizedChunk.includes(normalizedPath)
    );
  });
  return match ?? rawDiff;
}

// Turn snapshots and `git status` disagree on separators and leading "./", so
// review requests are matched on a normalized form.
function sameDiffPath(
  left: string | null | undefined,
  right: string | null | undefined,
) {
  if (!left || !right) return false;
  const normalize = (value: string) =>
    value.replace(/\\/g, "/").replace(/^\.\//, "");
  return normalize(left) === normalize(right);
}

function diffTextForPath(workspaceDiff: WorkspaceDiff, path: string): string {
  const sources = [
    workspaceDiff.stagedDiff ?? "",
    workspaceDiff.unstagedDiff ?? "",
    workspaceDiff.diff,
  ].filter((source) => source.trim().length > 0);
  const matches = sources
    .map((source) => extractDiffForPath(source, path))
    .filter((source) => source.trim().length > 0);
  return uniqueStrings(matches).join("\n\n");
}

function uniqueStrings(values: string[]): string[] {
  return Array.from(new Set(values));
}

function changedFileBadges(file: ChangedFile): Array<{
  label: string;
  title: string;
  className: string;
}> {
  if (isUntrackedFile(file)) {
    return [{ label: "UN", title: "Untracked", className: "added" }];
  }

  const badges: Array<{ label: string; title: string; className: string }> = [];
  if (file.stagedStatus) {
    badges.push({
      label: `S:${shortStatus(file.stagedStatus)}`,
      title: `Staged ${file.stagedStatus}`,
      className: file.stagedStatus,
    });
  }
  if (file.unstagedStatus) {
    badges.push({
      label: `W:${shortStatus(file.unstagedStatus)}`,
      title: `Unstaged ${file.unstagedStatus}`,
      className: file.unstagedStatus,
    });
  }
  if (!badges.length) {
    badges.push({
      label: file.status || "?",
      title: file.status || "Unknown status",
      className: file.status || "modified",
    });
  }
  return badges;
}

function changedFileStatusTitle(file: ChangedFile): string {
  return changedFileBadges(file)
    .map((badge) => badge.title)
    .join(", ");
}

function shortStatus(status: string): string {
  switch (status.toLocaleLowerCase()) {
    case "modified":
      return "M";
    case "added":
      return "A";
    case "deleted":
      return "D";
    case "renamed":
      return "R";
    case "copied":
      return "C";
    case "unmerged":
      return "U";
    default:
      return status.slice(0, 2).toLocaleUpperCase();
  }
}

function hasStagedChange(file: ChangedFile): boolean {
  return Boolean(file.stagedStatus);
}

function hasUnstagedChange(file: ChangedFile): boolean {
  return Boolean(file.unstagedStatus) || isUntrackedFile(file);
}

function isUntrackedFile(file: ChangedFile): boolean {
  return Boolean(file.isUntracked || file.status === "??");
}

function isRenamedFile(file: ChangedFile): boolean {
  return Boolean(
    file.isRenamed ||
    file.originalPath ||
    file.status.toLocaleUpperCase().includes("R") ||
    file.stagedStatus === "renamed" ||
    file.unstagedStatus === "renamed",
  );
}

function restoreDisabledReason(file: ChangedFile): string | null {
  if (isUntrackedFile(file)) {
    return "Untracked files are not removed by this safe restore action.";
  }
  if (isRenamedFile(file)) {
    return "Renamed paths need manual review before restore.";
  }
  if (hasStagedChange(file)) {
    return "Files with staged changes must be handled manually before worktree restore.";
  }
  if (
    file.unstagedStatus === "modified" ||
    file.unstagedStatus === "deleted" ||
    isTrackedRevertCandidate(file.status)
  ) {
    return null;
  }
  return "Only unstaged modified or deleted tracked files can be restored here.";
}

function sameWorkspacePath(left: string, right: string): boolean {
  return left.replace(/\\/g, "/") === right.replace(/\\/g, "/");
}

function statusClass(status: string): string {
  const value = status.toLocaleLowerCase();
  if (value.includes("a") || value.includes("new")) return "added";
  if (value.includes("d") || value.includes("delete")) return "deleted";
  if (value.includes("r") || value.includes("rename")) return "renamed";
  return "modified";
}

function isTrackedRevertCandidate(status: string): boolean {
  const value = status.trim().toLocaleUpperCase();
  if (!value || value.includes("??") || value.includes("A")) return false;
  return value.includes("M") || value.includes("D");
}

function collectMessageArtifactReferences(
  message: Message,
): ArtifactReference[] {
  const refs: ArtifactReference[] = [];
  for (const part of message.parts) {
    if (part.type === "text") {
      refs.push(...artifactReferencesFromText(part.text));
    } else if (part.type === "tool_result") {
      refs.push(
        ...collectArtifactReferences(part.result.metadata, part.result.output),
      );
    }
  }
  return uniqueArtifactReferences(refs);
}

function collectArtifactReferences(
  metadata: unknown,
  output?: string,
): ArtifactReference[] {
  return uniqueArtifactReferences([
    ...artifactReferencesFromMetadata(metadata),
    ...artifactReferencesFromText(output ?? ""),
  ]);
}

function artifactReferencesFromMetadata(
  metadata: unknown,
): ArtifactReference[] {
  if (!isRecord(metadata)) return [];
  const refs: ArtifactReference[] = [];
  const artifactId = readString(metadata.artifactId);
  if (artifactId) {
    refs.push({
      id: artifactId,
      kind: readString(metadata.artifactKind),
      bytes: readNumber(metadata.artifactBytes),
    });
  }
  if (isRecord(metadata.artifact)) {
    const nestedId = readString(metadata.artifact.id);
    if (nestedId) {
      refs.push({
        id: nestedId,
        kind: readString(metadata.artifact.kind),
        bytes: readNumber(metadata.artifact.bytes),
      });
    }
  }
  return refs;
}

function artifactReferencesFromText(text: string): ArtifactReference[] {
  const refs: ArtifactReference[] = [];
  const pattern =
    /\[Artifact:\s*([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})\]/g;
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(text)) !== null) {
    refs.push({ id: match[1] });
  }
  return refs;
}

function uniqueArtifactReferences(
  refs: ArtifactReference[],
): ArtifactReference[] {
  const byId = new Map<string, ArtifactReference>();
  for (const ref of refs) {
    byId.set(ref.id, { ...byId.get(ref.id), ...ref });
  }
  return [...byId.values()];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function readString(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value : undefined;
}

function readNumber(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value)
    ? value
    : undefined;
}

function formatUnknown(value: unknown): string {
  if (value === undefined || value === null) return "";
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function truncateTerminalOutput(output: string): string {
  const limit = 12000;
  if (output.length <= limit) return output;
  return `${output.slice(0, limit)}\n\n[output truncated in UI]`;
}

function formatTime(value: string): string {
  return new Date(value).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}
