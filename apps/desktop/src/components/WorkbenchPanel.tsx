import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Box,
  Check,
  FileCode2,
  Folder,
  FolderOpen,
  GitBranch,
  Puzzle,
  RefreshCw,
  RotateCcw,
  ShieldAlert,
  TerminalSquare,
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
import {
  DiffReviewPanel,
  type DiffReviewFileContent,
  type DiffReviewGitAction,
  type DiffReviewTurnScope,
} from "./DiffReviewPanel";
import { detectLanguage, MonacoEditor } from "./MonacoEditor";
import { ExtensionsView } from "./workbench/ExtensionsView";
import { ContextCard, FilesView } from "./workbench/FilesView";
import { TerminalView } from "./workbench/TerminalView";
import { formatTime } from "./workbench/workbenchFormat";

export { terminalShellName } from "./workbench/TerminalView";

export type WorkbenchTab =
  "files" | "diff" | "terminal" | "extensions" | "sandbox";

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
  onTogglePlugin(pluginId: string, enabled: boolean): Promise<void>;
  onUsePluginSkills(pluginId: string, enabled: boolean): Promise<void>;
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
  onTogglePlugin,
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
          onTogglePlugin={onTogglePlugin}
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

type DiffHunk = {
  path: string;
  scope: "staged" | "unstaged";
  header: string;
  lines: string[];
  raw: string;
  patch?: string;
};

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
