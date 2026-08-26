export type WorkspaceEntryKind = "file" | "directory" | "symlink" | "other";

export type WorkspaceEntry = {
  name: string;
  path: string;
  kind: WorkspaceEntryKind;
  size?: number | null;
  modifiedAt?: string | null;
};

export type WorkspaceTree = {
  root: string;
  path: string;
  entries: WorkspaceEntry[];
};

export type WorkspaceFilePreview = {
  path: string;
  content: string;
  bytes: number;
  truncated: boolean;
  readonly: boolean;
};

export type ChangedFile = {
  path: string;
  status: string;
  stagedStatus?: string | null;
  unstagedStatus?: string | null;
  originalPath?: string | null;
  isUntracked?: boolean;
  isRenamed?: boolean;
};

export type WorkspaceDiffScope = "staged" | "unstaged";

export type WorkspaceDiffHunk = {
  path: string;
  scope: WorkspaceDiffScope;
  header: string;
  lines: string[];
  raw: string;
  patch?: string;
  oldStart?: number | null;
  oldLines?: number | null;
  newStart?: number | null;
  newLines?: number | null;
};

export type WorkspaceDiffHunkAction = "stage" | "unstage" | "discard";

export type WorkspaceDiff = {
  command: string;
  branch?: string | null;
  remoteUrl?: string | null;
  files: ChangedFile[];
  diff: string;
  stagedDiff?: string;
  unstagedDiff?: string;
  hunks?: WorkspaceDiffHunk[];
  truncated: boolean;
  stagedTruncated?: boolean;
  unstagedTruncated?: boolean;
};

/**
 * A file handed to the review panel, e.g. by clicking a row on a turn change
 * card. `nonce` increments on every request so selecting the same path twice
 * still reaches the panel as a fresh request.
 */
export type ReviewFileRequest = {
  path: string;
  nonce: number;
};

export type TurnChangeSetStatus = "capturing" | "ready" | "empty" | "failed";

export type TurnFileChangeKind = "added" | "modified" | "deleted" | "renamed";

export type TurnFileChange = {
  kind: TurnFileChangeKind;
  oldPath?: string | null;
  newPath?: string | null;
  beforeOid?: string | null;
  afterOid?: string | null;
  beforeMode?: string | null;
  afterMode?: string | null;
  additions?: number | null;
  deletions?: number | null;
  binary: boolean;
};

export type TurnChangeSet = {
  turnId: string;
  threadId: string;
  workspaceRoot: string;
  repoRoot?: string | null;
  workspacePrefix?: string | null;
  beforeTree?: string | null;
  afterTree?: string | null;
  status: TurnChangeSetStatus;
  files: TurnFileChange[];
  additions: number;
  deletions: number;
  error?: string | null;
  createdAt: string;
  finalizedAt?: string | null;
  revertedAt?: string | null;
};

export type TurnFileDiffPreview = {
  turnId: string;
  path: string;
  oldPath?: string | null;
  newPath?: string | null;
  binary: boolean;
  diff: string;
  offset: number;
  nextOffset?: number | null;
  totalBytes: number;
};

export type TurnUndoConflictKind =
  | "unavailable"
  | "already_reverted"
  | "workspace_changed"
  | "merge_conflict"
  | "binary_changed"
  | "path_conflict"
  | "unsupported_file_type"
  | "too_large";

export type TurnUndoConflict = {
  path?: string | null;
  kind: TurnUndoConflictKind;
  reason: string;
};

export type TurnUndoPreview = {
  turnId: string;
  canUndo: boolean;
  filesToChange: number;
  additions: number;
  deletions: number;
  conflicts: TurnUndoConflict[];
  changeSet: TurnChangeSet;
};

export type TurnUndoResult = {
  applied: boolean;
  filesChanged: number;
  preview: TurnUndoPreview;
  changeSet: TurnChangeSet;
};

export type TerminalEventType =
  "started" | "stdout" | "stderr" | "finished" | "cancelled" | "error";

export type TerminalEvent = {
  id: string;
  threadId: string;
  commandId: string;
  seq: number;
  createdAt: string;
  type: TerminalEventType;
  command?: string | null;
  cwd?: string | null;
  data?: string | null;
  exitCode?: number | null;
  success?: boolean | null;
  message?: string | null;
};

export type TerminalStartResponse = {
  threadId: string;
  commandId: string;
  status: "started";
  historyUrl: string;
  streamUrl: string;
};

export type TerminalCancelResponse = {
  commandId?: string | null;
  cancelled: boolean;
  message: string;
};

export type TerminalSession = {
  sessionId: string;
  threadId: string;
  status: "running" | "closed";
  cwd: string;
  shell: string;
  processId?: number | null;
  startedAt: string;
};

export type DiffFileActionResult = {
  path: string;
  diff: WorkspaceDiff;
};

export type ContextBudget = {
  totalTokens: number;
  usedTokens: number;
  messageCount: number;
  estimatedUsage: number;
};

export type ContextSummary = {
  id: string;
  threadId: string;
  coveredThroughSeq: number;
  messageCount: number;
  summary: string;
  tokenEstimate?: number | null;
  createdAt: string;
  metadata: unknown;
  checkpoint?: ContextCheckpoint | null;
};

export type ContextCheckpointFactStatus = "active" | "resolved" | "superseded";

export type ContextCheckpointFact = {
  id: string;
  text: string;
  status: ContextCheckpointFactStatus;
  sourceSeqs: number[];
  confidence?: number | null;
};

export type ContextCheckpoint = {
  id: string;
  threadId: string;
  schemaVersion: number;
  mode: "legacy_text" | "manual" | "structured_local" | "native_provider";
  previousCheckpointId?: string | null;
  coverage: {
    throughSeq: number;
    throughMessageCount: number;
  };
  providerCompatibilityHash?: string | null;
  goal: string;
  userConstraints: ContextCheckpointFact[];
  decisions: ContextCheckpointFact[];
  workspaceState: {
    branch?: string | null;
    gitStatus?: string | null;
    filesChanged: Array<{
      path: string;
      status: string;
      summary: string;
      sourceSeqs: number[];
    }>;
  };
  commandsAndValidation: Array<{
    command: string;
    outcome: string;
    summary: string;
    sourceSeqs: number[];
  }>;
  openIssues: ContextCheckpointFact[];
  nextSteps: Array<{
    id: string;
    text: string;
    status: string;
    sourceSeqs: number[];
  }>;
  pendingInteractions: Array<{
    kind: string;
    summary: string;
    sourceSeqs: number[];
  }>;
  artifacts: Array<{
    id?: string | null;
    path?: string | null;
    kind: string;
    summary: string;
    sourceSeqs: number[];
  }>;
  createdAt: string;
};

export type ContextProjection = {
  checkpointId?: string | null;
  checkpointMode?: string | null;
  checkpointTokens: number;
  coveredThroughSeq: number;
  coveredMessageCount: number;
  unsummarizedMessageCount: number;
  unsummarizedEventCount: number;
  recentTailTokens: number;
  nativeCompactionSupported: boolean;
  providerStateAvailable: boolean;
  providerStateKind?: string | null;
  providerItemCount: number;
  nativeCompactionItemCount: number;
};

export type ContextCompactionDetails = {
  checkpointId?: string | null;
  mode: ContextCheckpoint["mode"];
  coverage: ContextCheckpoint["coverage"];
  providerStateCheckpointId?: string | null;
  metrics?: {
    source: string;
    inputTokens: number;
    checkpointTokens: number;
    tokenReductionPercent: number;
    latencyMs: number;
    factRetentionPercent: number;
    activeConstraintRetentionPercent: number;
  } | null;
};

export type ContextStatus = {
  budget: ContextBudget;
  latestSummary?: ContextSummary | null;
  usage: {
    modelRequests: number;
    agentModelRequests: number;
    compactionModelRequests: number;
    auxiliaryModelRequests: number;
    providerResponses: number;
    providerUsageCoverage?: number | null;
    inputTokens: number;
    outputTokens: number;
    totalTokens: number;
    uncachedInputTokens: number;
    cachedInputTokens: number;
    cacheWriteTokens: number;
    reasoningTokens: number;
    localInputEstimate: number;
    rawInputEstimate: number;
    estimateCalibrationFactor?: number | null;
    estimateErrorMean?: number | null;
    estimateErrorP95?: number | null;
    rawEstimateErrorMean?: number | null;
    rawEstimateErrorP95?: number | null;
    compactions: number;
    nativeCompactions: number;
    providerFallbacks: number;
    warnings: number;
    compactionInputTokens: number;
    checkpointTokens: number;
    compactionLatencyMs: number;
    lastFactRetentionPercent: number;
    lastActiveConstraintRetentionPercent: number;
  };
  projection?: ContextProjection;
};

export type ArtifactDescriptor = {
  id: string;
  threadId?: string;
  kind: string;
  contentType: string;
  bytes: number;
  createdAt: string;
  metadata?: unknown;
  storage?:
    | { type: "inline" }
    | { type: "path"; path: string }
    | Record<string, unknown>;
};

export type ArtifactContent = {
  id: string;
  content: string;
  filePath?: string | null;
  metadata?: unknown;
};

export type PreviewRenderer =
  | "text"
  | "code"
  | "image"
  | "pdf"
  | "document"
  | "spreadsheet"
  | "web"
  | "unsupported";

export type PreviewTarget =
  | { type: "workspace"; path: string }
  | { type: "local"; path: string }
  | { type: "artifact"; artifactId: string }
  | { type: "attachment"; attachmentId: string }
  | { type: "url"; url: string };

export type PreviewDescriptor = {
  id: string;
  threadId: string;
  target: PreviewTarget;
  renderer: PreviewRenderer;
  title: string;
  contentType: string;
  bytes?: number | null;
  revision: string;
  readonly: boolean;
  capabilities: {
    read: boolean;
    write: boolean;
    watch: boolean;
    rangeRead: boolean;
    openExternal: boolean;
  };
  handlerId?: string | null;
  truncated?: boolean;
  externalPath?: string | null;
};

export type SpreadsheetSheetPreview = {
  id: string;
  name: string;
  rowCount: number;
  columnCount: number;
  hidden?: boolean;
};

export type SpreadsheetPreview = {
  previewId: string;
  sheets: SpreadsheetSheetPreview[];
};

export type SpreadsheetPreviewCell = {
  row: number;
  column: number;
  value: string | number | boolean | null;
  formatted?: string | null;
  formula?: string | null;
};

export type SpreadsheetPreviewRange = {
  previewId: string;
  sheetId: string;
  rowStart: number;
  columnStart: number;
  rowCount: number;
  columnCount: number;
  cells: SpreadsheetPreviewCell[];
};

export type WebPreviewBounds = {
  x: number;
  y: number;
  width: number;
  height: number;
};

export type BrowserNewTabRequest = {
  openerSessionId: string;
  url: string;
};

export type BrowserProfilePersistence = "persistent" | "ephemeral";

export type BrowserRuntimeRoute = "managed" | "chrome";

export type BrowserRuntimeStatus = {
  route: BrowserRuntimeRoute;
  chromeAvailable: boolean;
};

export type ChromeBridgeState = {
  sessionId: string;
  availability: "available" | "unavailable";
  status: "idle" | "waiting_for_extension" | "waiting_for_tab" | "attached";
  pairingCode?: string;
  pairingExpiresAt?: string;
  tabId?: number;
  targetId?: string;
  url: string;
  title: string;
  error?: string | null;
};

export type WebPreviewState = {
  sessionId: string;
  profileId: string;
  profilePersistence: BrowserProfilePersistence;
  url: string;
  title?: string;
  faviconUrl?: string | null;
  loading: boolean;
  canGoBack: boolean;
  canGoForward: boolean;
  visible?: boolean;
  bounds?: WebPreviewBounds;
  error?: string | null;
};

export type BrowserNavigationRequest = {
  id: string;
  url: string;
};

export type SandboxDescriptor = {
  id: string;
  threadId: string;
  kind: "local" | "docker" | "remote";
  lifecycle: "ready" | "starting" | "stopped" | "error";
  workspaceRoot: string;
  capabilities: string[];
  message: string;
  platform: "linux" | "macos" | "windows" | "unsupported";
  mode: "disabled" | "best_effort" | "enforce";
  network: "inherit" | "allow" | "deny";
  sandboxMode: "read-only" | "workspace-write" | "danger-full-access";
  readableRoots: string[];
  writableRoots: string[];
  protectedPaths: string[];
  backend?: string | null;
  permissionProfile: string;
  enforced: boolean;
  available: boolean;
};
