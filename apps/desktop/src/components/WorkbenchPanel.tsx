import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
  Puzzle,
  RefreshCw,
  RotateCcw,
  ShieldAlert,
  Square,
  TerminalSquare,
  XCircle,
} from "lucide-react";
import type {
  AgentEvent,
  ArtifactContent,
  ArtifactDescriptor,
  ChangedFile,
  ContextStatus,
  McpServerView,
  Message,
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
import { ArtifactGallery } from "./ArtifactGallery";
import { detectLanguage, MonacoEditor } from "./MonacoEditor";
import type { XtermTerminalHandle } from "./XtermTerminal";
import { XtermTerminal } from "./XtermTerminal";

export type WorkbenchTab =
  "files" | "diff" | "terminal" | "extensions" | "sandbox";

type WorkbenchPanelProps = {
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
  onDecideApproval(approvalId: string, approved: boolean): void;
  onRefreshWorkbench(): void;
  onOpenWorkspacePath(path?: string): void;
  onOpenWorkspaceEntry(entry: WorkspaceEntry): void;
  onToggleThreadMcp(serverId: string, enabled: boolean): void;
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
  { id: "extensions", label: "Extensions", icon: Puzzle },
  { id: "sandbox", label: "Sandbox", icon: Box },
];

export function WorkbenchPanel({
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
  onDecideApproval,
  onRefreshWorkbench,
  onOpenWorkspacePath,
  onOpenWorkspaceEntry,
  onToggleThreadMcp,
  onOpenPath,
  onEnsureTerminalSession,
  onWriteTerminalSession,
  onResizeTerminalSession,
  onCloseTerminalSession,
  onCompactContext,
  onOpenArtifact,
  onRevertDiffFile,
  onApplyDiffHunk,
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
          workspaceDiff={workspaceDiff}
          revertingDiffPath={revertingDiffPath}
          hunkActionKey={hunkActionKey}
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
          mcpServers={mcpServers}
          threadMcpServers={threadMcpServers}
          onToggleThreadMcp={onToggleThreadMcp}
        />
      )}
      {activeTab === "sandbox" && <SandboxView sandbox={sandbox} />}
    </>
  );

  if (mode === "stage") {
    return (
      <section className="workbench-stage-panel">
        {workbenchError && <p className="workspace-error">{workbenchError}</p>}
        <div className="workbench-tab-panel stage">{tabContent}</div>
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
              {latestApprovalPayload.action.startsWith("browser:domain:")
                ? "Allow Domain"
                : latestApprovalPayload.action === "Continue agent execution"
                  ? "Continue"
                  : "Allow Once"}
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
      {latestSummary ? (
        <details className="context-summary">
          <summary>
            Summary through event {latestSummary.coveredThroughSeq}
            <ChevronDown size={12} />
          </summary>
          <p>{latestSummary.summary}</p>
        </details>
      ) : (
        <p>No context summary yet.</p>
      )}
      <button
        className="secondary-button compact"
        type="button"
        disabled={disabled}
        onClick={onCompactContext}
      >
        <RefreshCw size={13} className={isCompacting ? "spin" : ""} />
        {isCompacting ? "Compacting" : "Compact"}
      </button>
    </section>
  );
}

function FilesView({
  workspaceRoot,
  workspaceTree,
  filePreview,
  isRefreshing,
  onRefresh,
  onOpenWorkspacePath,
  onOpenWorkspaceEntry,
  onOpenPath,
}: {
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

  return (
    <div className="files-view">
      <div className="workbench-section-header">
        <Breadcrumb path={currentPath} onOpenPath={onOpenWorkspacePath} />
        <button
          className="icon-button small"
          type="button"
          aria-label="Refresh files"
          disabled={isRefreshing}
          onClick={onRefresh}
        >
          <RefreshCw size={13} className={isRefreshing ? "spin" : ""} />
        </button>
      </div>

      <div className="workbench-file-list">
        {entries.length ? (
          entries.map((entry) => (
            <button
              className={`file-row workbench-file-row ${
                filePreview?.path === entry.path ? "active" : ""
              }`}
              key={entry.path}
              type="button"
              title={entry.path}
              onClick={() => onOpenWorkspaceEntry(entry)}
            >
              {entry.kind === "directory" ? (
                <Folder size={14} />
              ) : (
                <FileText size={14} />
              )}
              <span>{entry.name}</span>
              <small>
                {entry.kind === "directory" ? "dir" : formatBytes(entry.size)}
              </small>
              {entry.kind === "directory" && <ChevronRight size={13} />}
            </button>
          ))
        ) : (
          <span className="muted">No files loaded.</span>
        )}
      </div>
    </div>
  );
}

function Breadcrumb({
  path,
  onOpenPath,
}: {
  path: string;
  onOpenPath(path?: string): void;
}) {
  const parts = splitWorkspacePath(path);

  return (
    <nav className="path-breadcrumb" aria-label="Current folder">
      <button type="button" onClick={() => onOpenPath(undefined)}>
        root
      </button>
      {parts.map((part, index) => (
        <span key={`${part}-${index}`} className="breadcrumb-part">
          <ChevronRight size={12} />
          <button
            type="button"
            title={parts.slice(0, index + 1).join("/")}
            onClick={() => onOpenPath(parts.slice(0, index + 1).join("/"))}
          >
            {part}
          </button>
        </span>
      ))}
    </nav>
  );
}

type DiffSubTab = "diff" | "review";

function DiffView({
  workspaceDiff,
  revertingDiffPath,
  hunkActionKey,
  onRevertDiffFile,
  onApplyDiffHunk,
}: {
  workspaceDiff: WorkspaceDiff | null;
  revertingDiffPath: string | null;
  hunkActionKey: string | null;
  onRevertDiffFile(path: string): void;
  onApplyDiffHunk(
    hunk: WorkspaceDiffHunk,
    action: WorkspaceDiffHunkAction,
  ): void;
}) {
  const [filter, setFilter] = useState("");
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [diffSubTab, setDiffSubTab] = useState<DiffSubTab>("diff");

  useEffect(() => {
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
  }, [selectedPath, workspaceDiff]);

  const filteredFiles = useMemo(() => {
    const normalizedFilter = filter.trim().toLocaleLowerCase();
    if (!workspaceDiff) return [];
    if (!normalizedFilter) return workspaceDiff.files;
    return workspaceDiff.files.filter((file) =>
      `${file.status} ${file.stagedStatus ?? ""} ${file.unstagedStatus ?? ""} ${file.originalPath ?? ""} ${file.path}`
        .toLocaleLowerCase()
        .includes(normalizedFilter),
    );
  }, [filter, workspaceDiff]);
  const statusSummary = useMemo(
    () => buildDiffStatusSummary(workspaceDiff?.files ?? []),
    [workspaceDiff],
  );

  const selectedFile =
    selectedPath && workspaceDiff
      ? (workspaceDiff.files.find((file) => file.path === selectedPath) ?? null)
      : null;

  const previewText =
    workspaceDiff && selectedFile
      ? diffTextForPath(workspaceDiff, selectedFile.path)
      : (workspaceDiff?.diff ?? "");

  const parsedDiff =
    selectedFile && workspaceDiff
      ? parseDiffContent(
          diffTextForPath(workspaceDiff, selectedFile.path),
          selectedFile.path,
        )
      : null;

  if (!workspaceDiff) {
    return <div className="workbench-empty-state">No diff loaded.</div>;
  }

  return (
    <div className="diff-view">
      <div className="diff-summary-row">
        <span>{workspaceDiff.files.length} changed</span>
        <span>{statusSummary.staged} staged</span>
        <span>{statusSummary.unstaged} unstaged</span>
        <span>{statusSummary.untracked} untracked</span>
        <span>{statusSummary.renamed} renamed</span>
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

      <div className="diff-sub-tabs">
        <button
          className={`diff-sub-tab ${diffSubTab === "diff" ? "active" : ""}`}
          type="button"
          onClick={() => setDiffSubTab("diff")}
        >
          <FileCode2 size={13} />
          Diff
        </button>
        <button
          className={`diff-sub-tab ${diffSubTab === "review" ? "active" : ""}`}
          type="button"
          onClick={() => setDiffSubTab("review")}
        >
          <ShieldAlert size={13} />
          Review
        </button>
      </div>

      {diffSubTab === "diff" && (
        <>
          <label className="diff-filter">
            <span>Filter</span>
            <input
              value={filter}
              placeholder="Path or status"
              onChange={(event) => setFilter(event.target.value)}
            />
          </label>

          <div className="changed-file-list">
            <button
              className={`changed-file-row ${selectedPath === null ? "active" : ""}`}
              type="button"
              onClick={() => setSelectedPath(null)}
            >
              <span className="diff-status">all</span>
              <span>All files</span>
            </button>
            {filteredFiles.length ? (
              filteredFiles.map((file) => (
                <ChangedFileButton
                  key={`${file.status}-${file.path}`}
                  file={file}
                  selected={file.path === selectedPath}
                  onSelect={() => setSelectedPath(file.path)}
                />
              ))
            ) : (
              <span className="muted">No matching files.</span>
            )}
          </div>

          <div className="diff-preview-header">
            <span title={selectedFile?.path ?? "Raw diff"}>
              {selectedFile?.path ?? "Raw diff"}
            </span>
            {selectedFile && (
              <button
                className="secondary-button compact"
                type="button"
                onClick={() => setSelectedPath(null)}
              >
                Show raw
              </button>
            )}
          </div>

          {selectedFile && parsedDiff ? (
            <div className="diff-editor-panel">
              <div className="diff-editor-container">
                <div className="diff-editor-label">Original</div>
                <MonacoEditor
                  value={parsedDiff.original}
                  language={detectLanguage(selectedFile.path)}
                  readOnly
                />
              </div>
              <div className="diff-editor-container">
                <div className="diff-editor-label">Modified</div>
                <MonacoEditor
                  value={parsedDiff.modified}
                  language={detectLanguage(selectedFile.path)}
                  readOnly
                />
              </div>
            </div>
          ) : (
            <MonacoEditor
              value={previewText || "(no diff)"}
              language="diff"
              readOnly
            />
          )}
        </>
      )}

      {diffSubTab === "review" && (
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
      )}
    </div>
  );
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

function ChangedFileButton({
  file,
  selected,
  onSelect,
}: {
  file: ChangedFile;
  selected: boolean;
  onSelect(): void;
}) {
  return (
    <button
      className={`changed-file-row ${selected ? "active" : ""}`}
      type="button"
      title={file.path}
      onClick={onSelect}
    >
      <ChangedFileStatusBadges file={file} />
      <span>{file.path}</span>
    </button>
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
    xtermRef.current?.clear();
  }, [thread?.id]);

  useEffect(() => {
    if (
      terminalEvents.length > 0 &&
      writtenTerminalEventsRef.current.size === 0
    ) {
      xtermRef.current?.clear();
    }

    for (const event of terminalEvents) {
      if (writtenTerminalEventsRef.current.has(event.id)) continue;
      writeTerminalEventToXterm(event, xtermRef.current);
      writtenTerminalEventsRef.current.add(event.id);
    }
  }, [terminalEvents]);

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
      <div className="terminal-toolbar">
        <span>
          {terminalSession
            ? `${terminalSession.shell} · persistent`
            : "shell closed"}
        </span>
        {thread && terminalSession ? (
          <button
            className="secondary-button compact"
            type="button"
            onClick={() => onCloseSession(thread.id, terminalSession.sessionId)}
          >
            <Square size={12} />
            Close shell
          </button>
        ) : (
          <button
            className="secondary-button compact"
            type="button"
            disabled={!thread || isStartingSession}
            onClick={handleRestart}
          >
            <TerminalSquare size={12} />
            {isStartingSession ? "Starting" : "Start shell"}
          </button>
        )}
        <button
          className="secondary-button compact"
          type="button"
          onClick={() => xtermRef.current?.clear()}
        >
          Clear
        </button>
      </div>
      <div className="xterm-wrapper">
        <XtermTerminal
          ref={xtermRef}
          disabled={inputDisabled}
          onData={handleData}
          onResize={handleResize}
        />
      </div>
      <details className="terminal-history">
        <summary>
          History ({terminalRows.length} events)
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
            <span className="muted">No events yet.</span>
          )}
        </div>
      </details>
    </div>
  );
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

function ExtensionsView({
  mcpServers,
  threadMcpServers,
  onToggleThreadMcp,
}: {
  mcpServers: McpServerView[];
  threadMcpServers: ThreadMcpServerView[];
  onToggleThreadMcp(serverId: string, enabled: boolean): void;
}) {
  const statusByServer = new Map(
    mcpServers.map((item) => [item.server.serverId, item.status]),
  );

  return (
    <div className="extensions-view">
      <div className="diff-summary-row">
        <span>{threadMcpServers.length} available</span>
        <span>
          {threadMcpServers.filter((item) => item.enabled).length} enabled
        </span>
      </div>
      <div className="extension-list">
        {threadMcpServers.length ? (
          threadMcpServers.map((item) => {
            const status = statusByServer.get(item.server.serverId);
            return (
              <label className="extension-row" key={item.server.serverId}>
                <input
                  type="checkbox"
                  checked={item.enabled}
                  onChange={(event) =>
                    onToggleThreadMcp(
                      item.server.serverId,
                      event.target.checked,
                    )
                  }
                />
                <span>{item.server.name}</span>
                <small title={item.server.command}>{item.server.command}</small>
                <em>
                  {status?.status ??
                    (item.server.enabled ? "configured" : "disabled")}
                </em>
              </label>
            );
          })
        ) : (
          <span className="muted">
            {mcpServers.length
              ? "No thread extensions available."
              : "No MCP servers configured."}
          </span>
        )}
      </div>
    </div>
  );
}

function SandboxView({ sandbox }: { sandbox: SandboxDescriptor | null }) {
  if (!sandbox) {
    return <div className="workbench-empty-state">No sandbox loaded.</div>;
  }

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
          <dd title={sandbox.workspaceRoot}>{sandbox.workspaceRoot}</dd>
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
          <dt>Readable roots</dt>
          <dd title={sandbox.readableRoots.join("\n")}>
            {sandbox.readableRoots.length
              ? sandbox.readableRoots.join(", ")
              : sandbox.sandboxMode === "danger-full-access"
                ? "unrestricted"
                : "none"}
          </dd>
        </div>
        <div>
          <dt>Writable roots</dt>
          <dd title={sandbox.writableRoots.join("\n")}>
            {sandbox.writableRoots.length
              ? sandbox.writableRoots.join(", ")
              : "none"}
          </dd>
        </div>
        <div>
          <dt>Protected metadata</dt>
          <dd title={sandbox.protectedPaths.join("\n")}>
            {sandbox.protectedPaths.length
              ? sandbox.protectedPaths.join(", ")
              : "none"}
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

function parseDiffContent(
  diffText: string,
  filePath: string,
): { original: string; modified: string } | null {
  const normalizedPath = filePath.replace(/\\/g, "/");
  const chunks = diffText.split(/\n(?=diff --git )/);
  const match = chunks.find((chunk) => {
    const nc = chunk.replace(/\\/g, "/");
    return (
      nc.includes(` a/${normalizedPath}`) || nc.includes(` b/${normalizedPath}`)
    );
  });

  if (!match) return null;

  const originalLines: string[] = [];
  const modifiedLines: string[] = [];
  const lines = match.split("\n");
  let inHunk = false;

  for (const line of lines) {
    if (/^@@ -\d+(,\d*)? \+\d+(,\d*)? @@/.test(line)) {
      inHunk = true;
      continue;
    }
    if (!inHunk) continue;
    if (line.startsWith("-")) {
      originalLines.push(line.slice(1));
    } else if (line.startsWith("+")) {
      modifiedLines.push(line.slice(1));
    } else if (line.startsWith(" ")) {
      const content = line.slice(1);
      originalLines.push(content);
      modifiedLines.push(content);
    }
  }

  return {
    original: originalLines.join("\n"),
    modified: modifiedLines.join("\n"),
  };
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
    event.exitCode === undefined || event.exitCode === null
      ? undefined
      : `exit code: ${event.exitCode}`,
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
              .map((item) => `[${item.status}] ${item.step}`)
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

function buildDiffStatusSummary(files: ChangedFile[]): {
  staged: number;
  unstaged: number;
  untracked: number;
  renamed: number;
} {
  return files.reduce(
    (summary, file) => ({
      staged: summary.staged + (hasStagedChange(file) ? 1 : 0),
      unstaged: summary.unstaged + (hasUnstagedChange(file) ? 1 : 0),
      untracked: summary.untracked + (isUntrackedFile(file) ? 1 : 0),
      renamed: summary.renamed + (isRenamedFile(file) ? 1 : 0),
    }),
    { staged: 0, unstaged: 0, untracked: 0, renamed: 0 },
  );
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
