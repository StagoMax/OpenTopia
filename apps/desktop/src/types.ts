export type PlatformInfo = {
  platform: "desktop" | "web";
  os?: string;
  arch?: string;
  versions?: Record<string, string>;
  backendUrl: string;
  apiToken: string;
  keyring?: KeyringMetadata;
  paths?: {
    userData?: string | null;
    logs?: string | null;
    crashLogs?: string | null;
  };
  protocol?: {
    scheme: string;
    registered: boolean;
  };
};

export type SystemNotificationOptions = {
  title: string;
  body: string;
  silent?: boolean;
};

export type RecentWorkspace = {
  workspaceRoot: string;
  name: string;
  lastOpenedAt: string;
};

export type WorkspacePickResult =
  | { canceled: true }
  | {
      canceled: false;
      workspaceRoot: string;
      workspace: RecentWorkspace;
      recentWorkspaces: RecentWorkspace[];
    };

export type ContextSourceFile = {
  path: string;
  name: string;
  extension: string;
  kind: "text" | "image" | "document";
  bytes: number;
};

export type ContextSourcePickResult =
  | { canceled: true; files: [] }
  | { canceled: false; files: ContextSourceFile[] };

export type PluginDirectoryPickResult =
  { canceled: true } | { canceled: false; path: string };

export type BrowserContent =
  | { type: "text"; text: string; truncated: boolean }
  | { type: "json"; value: unknown }
  | { type: "image"; mime_type: string; bytes: number[] }
  | {
      type: "file";
      path: string;
      mime_type?: string | null;
      bytes: number;
    };

export type BrowserOutput = {
  url?: string | null;
  contents: BrowserContent[];
  metadata: unknown;
};

export type BrowserRect = {
  x: number;
  y: number;
  width: number;
  height: number;
};

export type BrowserNode = {
  nodeRef: string;
  role: string;
  name: string;
  tagName: string;
  bounds: BrowserRect;
  href?: string | null;
  formAction?: string | null;
  editable: boolean;
};

export type BrowserObservation = {
  observationId: string;
  url: string;
  title: string;
  text: string;
  textTruncated: boolean;
  nodes: BrowserNode[];
};

export type ScreenRect = {
  x: number;
  y: number;
  width: number;
  height: number;
};

export type ComputerWindowTarget = {
  windowId: string;
  processId: number;
  title: string;
  executable?: string | null;
  bounds: ScreenRect;
  isForeground: boolean;
};

export type ComputerScreenshot = {
  mimeType: string;
  bytes: number[];
};

export type ComputerObservation = {
  observationId: string;
  sessionId: string;
  target: ComputerWindowTarget;
  captureRect: ScreenRect;
  imageWidth: number;
  imageHeight: number;
  screenshot?: ComputerScreenshot | null;
  accessibilityTree?: unknown | null;
  unstable: boolean;
  capturedAt: string;
};

export type ExperienceMode = "work" | "code";
export type CollaborationMode = "default" | "plan" | "goal";

export type Thread = {
  id: string;
  title: string;
  workspaceRoot: string;
  projectId: string | null;
  experienceMode: ExperienceMode;
  archivedAt: string | null;
  createdAt: string;
  updatedAt: string;
};

export type Project = {
  id: string;
  name: string;
  workspaceRoot: string | null;
  pinned: boolean;
  sortOrder: number;
  createdAt: string;
  updatedAt: string;
};

export type PermissionMode =
  "chat" | "read_only" | "auto" | "approve" | "full_access";

export type ProviderKind =
  | "mock"
  | "openai_compatible"
  | "openai_responses"
  | "codex_app_server";

export type AppSettings = {
  providers: ProviderSettings[];
  activeProviderId: string;
  permissionMode: PermissionMode;
  defaultWorkspaceRoot?: string | null;
  sandbox: SandboxSettings;
  updatedAt: string;
};

export type SandboxSettings = {
  sandboxMode: "read-only" | "workspace-write" | "danger-full-access";
  enforcement: "disabled" | "best-effort" | "enforce";
  network: "inherit" | "allow" | "deny";
  writableRoots: string[];
  readPaths: string[];
};

export type ProviderSettings = {
  id: string;
  kind: ProviderKind;
  baseUrl: string;
  model: string;
  temperature: number;
  maxOutputTokens?: number | null;
  contextWindowTokens: number;
  reasoningEffort?:
    "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | null;
  storeResponses: boolean;
  parallelToolCalls: boolean;
  promptCacheKey?: string | null;
  promptCachePolicy?: "explicit_30m" | "legacy_in_memory" | "legacy_24h" | null;
  responsesCompactionThresholdTokens?: number | null;
  rolloutBudget?: RolloutBudgetSettings | null;
  supportsVision: boolean;
  apiKeySource: string;
  apiKeyConfigured: boolean;
  healthStatus?: string | null;
};

export type RolloutBudgetSettings = {
  limitTokens: number;
  samplingTokenWeight: number;
  prefillTokenWeight: number;
};

export type ProviderHealth = {
  id: string;
  kind: ProviderKind;
  baseUrl: string;
  model: string;
  apiKeySource: string;
  apiKeyConfigured: boolean;
  usingMock: boolean;
  status: string;
};

export type ProviderHealthCheckResult = {
  reachable: boolean;
  latencyMs?: number | null;
  modelAvailable: boolean;
  error?: string | null;
};

export type LogFileInfo = {
  name: string;
  path: string;
  size: number;
  modifiedAt: string;
};

export type SecretSource = {
  id: string;
  providerId?: string;
  kind: "environment" | "keyring" | string;
  label: string;
  envName?: string;
  configured: boolean;
  readableByRenderer: false;
  storesValue: boolean;
  status: string;
  available?: boolean;
  storageBackend?: string | null;
  storagePath?: string;
  envTarget?: string;
};

export type KeyringMetadata = {
  providerId?: string;
  available: boolean;
  encryptionAvailable: boolean;
  storageBackend?: string | null;
  storagePath?: string;
  providerApiKeyConfigured: boolean;
  providerApiKeySourceId: string;
  envTarget: string;
  status: string;
};

export type SecretSources = {
  activeProviderKeySource: string | null;
  keyring?: KeyringMetadata;
  sources: SecretSource[];
  notes: string[];
};

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

export type GitWorkflowActionKind =
  | "status"
  | "list_branches"
  | "create_branch"
  | "switch_branch"
  | "commit"
  | "push"
  | "compare"
  | "create_worktree";

export type GitWorkflowAction =
  | { type: "status"; request: { includeUntracked: boolean } }
  | { type: "list_branches"; request: { includeRemote: boolean } }
  | {
      type: "create_branch";
      request: { branch: string; startPoint: string | null };
    }
  | { type: "switch_branch"; request: { branch: string } }
  | { type: "commit"; request: { message: string; allTracked: boolean } }
  | {
      type: "push";
      request: { remote: string; branch: string; setUpstream: boolean };
    }
  | {
      type: "compare";
      request: {
        base: string;
        head: string;
        mode: "direct" | "merge_base";
      };
    };

export type GitWorkflowResponse = {
  action: GitWorkflowActionKind;
  stdout: string;
  stderr: string;
  exitCode: number | null;
  success: boolean;
  truncated: boolean;
};

export type GitStatusSummary = {
  branch: string | null;
  upstream: string | null;
  detached: boolean;
  ahead: number;
  behind: number;
  changed: number;
  staged: number;
  unstaged: number;
  untracked: number;
  raw: string;
};

export type GitBranchInfo = {
  fullRef: string;
  name: string;
  current: boolean;
  remote: boolean;
  upstream: string | null;
  symbolicTarget: string | null;
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
};

export type ContextStatus = {
  budget: ContextBudget;
  latestSummary?: ContextSummary | null;
  usage: {
    modelRequests: number;
    inputTokens: number;
    cachedInputTokens: number;
    cacheWriteTokens: number;
    reasoningTokens: number;
    compactions: number;
    warnings: number;
  };
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
  "text" | "code" | "image" | "pdf" | "spreadsheet" | "web" | "unsupported";

export type PreviewTarget =
  | { type: "workspace"; path: string }
  | { type: "artifact"; artifactId: string }
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

export type WebPreviewState = {
  sessionId: string;
  url: string;
  title?: string;
  loading: boolean;
  canGoBack: boolean;
  canGoForward: boolean;
  visible?: boolean;
  bounds?: WebPreviewBounds;
  error?: string | null;
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

export type McpServerConfig = {
  serverId: string;
  name: string;
  command: string;
  args: string[];
  cwd?: string | null;
  envKeys: string[];
  timeoutMs: number;
  enabled: boolean;
  pluginId?: string;
  pluginServerName?: string;
  createdAt: string;
  updatedAt: string;
};

export type McpServerInput = {
  name: string;
  command: string;
  args?: string[];
  cwd?: string;
  envKeys?: string[];
  timeoutMs?: number;
  enabled?: boolean;
};

export type McpServerStatus = {
  serverId: string;
  name: string;
  status: "not_started" | "starting" | "ready" | "error" | "disabled";
  message: string;
  toolsCount: number;
  updatedAt: string;
};

export type McpServerView = {
  server: McpServerConfig;
  status: McpServerStatus;
};

export type ThreadMcpServer = {
  threadId: string;
  serverId: string;
  enabled: boolean;
  updatedAt: string;
};

export type ThreadMcpServerView = {
  server: McpServerConfig;
  binding?: ThreadMcpServer | null;
  enabled: boolean;
};

export type McpCallResult = {
  serverId: string;
  publicName: string;
  toolName: string;
  output: string;
  content: unknown[];
  structuredContent?: unknown | null;
  isError: boolean;
  raw: unknown;
};

export type MessageRole = "system" | "user" | "assistant" | "tool";

export type Message = {
  id: string;
  threadId: string;
  role: MessageRole;
  parts: MessagePart[];
  createdAt: string;
};

export type MessagePart =
  | { type: "text"; text: string }
  | { type: "tool_call"; call: ToolCall }
  | { type: "tool_result"; result: ToolResult }
  | { type: "file_ref"; path: string }
  | { type: "source_ref"; source: ContextSourceRef }
  | { type: "skill_ref"; skill: SkillRef }
  | {
      type: "turn_context";
      collaboration_mode: CollaborationMode;
      goal_id?: string | null;
    }
  | { type: "error"; message: string };

export type ContextSourceRef = {
  id: string;
  path: string;
  name: string;
  kind: "text" | "image" | "document";
  contentType: string;
  bytes: number;
  truncated: boolean;
};

export type SkillScope = "workspace" | "user";

export type SkillDescriptor = {
  id: string;
  name: string;
  description: string;
  path: string;
  scope: SkillScope;
  pluginId?: string;
};

export type PluginDescriptor = {
  id: string;
  name: string;
  displayName: string;
  version: string;
  description: string;
  longDescription: string;
  author: string;
  category: string;
  path: string;
  manifestPath: string;
  scope: "workspace" | "user" | "codex";
  managed: boolean;
  skillRoot?: string;
  skillCount: number;
  mcpServerCount: number;
  supportedMcpServerCount: number;
  hasApps: boolean;
  capabilities: string[];
  brandColor?: string;
  websiteUrl?: string;
  issues: string[];
};

export type PluginView = {
  plugin: PluginDescriptor;
  skillIds: string[];
  mcpServers: McpServerView[];
  threadEnabled: boolean;
  compatible: boolean;
};

export type SkillRef = {
  id: string;
  name: string;
  description: string;
  path: string;
  truncated: boolean;
};

export type ToolCall = {
  id: string;
  name: string;
  input: unknown;
};

export type ToolResult = {
  callId: string;
  output: string;
  content?: ModelContentPart[];
  metadata: unknown;
};

export type TaskPlanStepStatus =
  | "pending"
  | "in_progress"
  | "completed"
  | "deferred"
  | "blocked"
  | "cancelled";

export type TaskPlanStep = {
  id: string;
  title: string;
  step?: string;
  status: TaskPlanStepStatus;
  statusReason?: string | null;
  dependencies: string[];
  acceptanceCriteria: string[];
  evidence: string[];
};

export type TaskPlan = {
  planRevision: number;
  goalId: string;
  changeReason?: string | null;
  explanation?: string | null;
  steps: TaskPlanStep[];
};

export type GoalStatus =
  | "draft"
  | "ready"
  | "active"
  | "paused"
  | "completed"
  | "blocked"
  | "cancelled"
  | "failed";

export type GoalTaskStatus =
  | "pending"
  | "running"
  | "succeeded"
  | "deferred"
  | "blocked"
  | "cancelled"
  | "failed";

export type GoalAttemptStatus =
  "running" | "succeeded" | "failed" | "interrupted";

export type GoalRecord = {
  id: string;
  threadId: string;
  objective: string;
  status: GoalStatus;
  planRevision: number;
  tokenBudget?: number | null;
  tokensUsed: number;
  timeUsedSeconds: number;
  version: number;
  createdAt: string;
  updatedAt: string;
  completedAt?: string | null;
};

export type GoalTask = {
  goalId: string;
  stepId: string;
  ordinal: number;
  title: string;
  status: GoalTaskStatus;
  statusReason?: string | null;
  dependencies: string[];
  acceptanceCriteria: string[];
  evidence: string[];
  attemptCount: number;
  maxAttempts: number;
  updatedAt: string;
};

export type GoalTaskAttempt = {
  id: string;
  goalId: string;
  stepId: string;
  turnId: string;
  attemptNo: number;
  status: GoalAttemptStatus;
  startedAt: string;
  finishedAt?: string | null;
  evidence: string[];
  error?: string | null;
};

export type GoalSnapshot = {
  goal: GoalRecord;
  tasks: GoalTask[];
  attempts: GoalTaskAttempt[];
};

export type ModelContentPart =
  | { type: "text"; text: string }
  | { type: "json"; value: unknown }
  | { type: "image"; content_type: string; data: number[] }
  | {
      type: "resource";
      uri: string;
      content_type?: string | null;
      name?: string | null;
    };

export type ModelRequestSnapshot = {
  systemPrompt: string;
  conversation: Array<{
    role: "system" | "user" | "assistant";
    content: string;
    contentParts?: ModelContentPart[];
  }>;
  userMessage: string;
  userContent?: ModelContentPart[];
  toolCandidates: Array<{
    name: string;
    description: string;
    inputSchema: unknown;
  }>;
  previousToolCalls: Array<{
    id: string;
    name: string;
    arguments: unknown;
  }>;
  toolResults: Array<{
    callId: string;
    name: string;
    output: string;
    content?: ModelContentPart[];
    isError: boolean;
    metadata: unknown;
  }>;
  contextItems?: ModelContextItem[];
  previousResponseItems?: unknown[];
  promptCacheKey?: string | null;
  finalOutputJsonSchema?: unknown | null;
};

export type ModelContextItem = {
  id: string;
  kind:
    | "base_instructions"
    | "developer_instructions"
    | "repository_instructions"
    | "environment"
    | "world_state"
    | "skill"
    | "summary"
    | "conversation"
    | "user"
    | "tool_call"
    | "tool_result";
  role: "system" | "developer" | "user" | "assistant" | "tool";
  source: string;
  content: ModelContentPart[];
  contentHash: string;
  tokenEstimate: number;
  cacheScope: "stable" | "thread" | "turn" | "round" | "none";
  sensitivity: "public" | "workspace" | "sensitive";
  metadata?: unknown;
};

export type ThreadContextSnapshot = {
  capturedAt: string;
  providerId: string;
  providerKind: string;
  model: string;
  workspaceRoot: string;
  cwd: string;
  experienceMode: string;
  permissionMode: string;
  sandboxMode: string;
  instructions: unknown[];
  toolCatalogHash: string;
  worldStateHash: string;
  contextHash: string;
};

export type TurnContextSnapshot = {
  capturedAt: string;
  cwd: string;
  workspaceRoots: string[];
  experienceMode: string;
  permissionMode: string;
  sandboxMode: string;
  instructions: unknown[];
  worldState: Record<string, unknown>;
  worldStateHash: string;
  previousWorldStateHash?: string | null;
  changedKeys: string[];
  contextHash: string;
};

export type AgentEvent = {
  id: string;
  threadId: string;
  turnId?: string | null;
  seq: number;
  createdAt: string;
  payload: AgentEventPayload;
};

export type UserInputOption = {
  id: string;
  label: string;
  description: string;
  recommended: boolean;
};

export type UserInputQuestion = {
  id: string;
  header: string;
  question: string;
  options: UserInputOption[];
  allowCustom: boolean;
};

export type UserInputRequest = {
  requestId: string;
  questions: UserInputQuestion[];
};

export type UserInputAnswer = {
  questionId: string;
  optionId?: string;
  customText?: string;
};

export type UserInputResponse = {
  answers: UserInputAnswer[];
};

export type UserInputRecord = {
  threadId: string;
  request: UserInputRequest;
  status: "pending" | "answered";
  response?: UserInputResponse | null;
  createdAt: string;
  answeredAt?: string | null;
};

export type AgentEventPayload =
  | { type: "thread_context_snapshot"; snapshot: ThreadContextSnapshot }
  | { type: "turn_context_snapshot"; snapshot: TurnContextSnapshot }
  | { type: "turn_started"; user_message_id: string }
  | {
      type: "model_context_built";
      request_id: string;
      round: number;
      context_hash: string;
      token_estimate: number;
      items: ModelContextItem[];
    }
  | {
      type: "model_request";
      request_id: string;
      round: number;
      request: ModelRequestSnapshot;
    }
  | {
      type: "provider_request_sent";
      request_id: string;
      round: number;
      attempt: number;
      adapter: string;
      method: string;
      endpoint: string;
      body: unknown;
    }
  | {
      type: "provider_request_retried";
      request_id: string;
      round: number;
      attempt: number;
      reason: string;
      body: unknown;
    }
  | {
      type: "provider_response_received";
      request_id: string;
      round: number;
      attempt: number;
      status?: number | null;
      response_id?: string | null;
      body: unknown;
    }
  | { type: "model_delta"; text: string }
  | { type: "reasoning_delta"; text: string }
  | { type: "tool_call_started"; call: ToolCall }
  | { type: "tool_call_finished"; result: ToolResult }
  | { type: "plan_updated"; plan: TaskPlan }
  | { type: "goal_updated"; snapshot: GoalSnapshot }
  | { type: "user_input_requested"; request: UserInputRequest }
  | { type: "assistant_message"; message: Message }
  | { type: "file_changed"; path: string; summary: string }
  | { type: "turn_changes_recorded"; change_set: TurnChangeSet }
  | {
      type: "turn_undo_completed";
      target_turn_id: string;
      files_changed: number;
    }
  | { type: "subagent_updated"; run: SubagentRun }
  | {
      type: "approval_requested";
      approval_id: string;
      reason: string;
      action: string;
    }
  | {
      type: "automatic_approval_review_started";
      review_id: string;
      target_item_id: string;
      action: unknown;
    }
  | {
      type: "automatic_approval_review_completed";
      review_id: string;
      target_item_id: string;
      status: "in_progress" | "approved" | "denied" | "timed_out" | "aborted";
      risk_level?: "low" | "medium" | "high" | "critical" | null;
      user_authorization?: "unknown" | "low" | "medium" | "high" | null;
      rationale: string;
      action: unknown;
    }
  | { type: "auto_review_interruption_warning"; message: string }
  | { type: "context_compacted"; summary: ContextSummary }
  | { type: "context_warning"; stage: string; message: string }
  | {
      type: "token_usage";
      input_tokens: number;
      output_tokens: number;
      total_tokens: number;
      cached_input_tokens?: number | null;
      cache_write_tokens?: number | null;
      reasoning_tokens?: number | null;
    }
  | { type: "turn_finished"; summary: string }
  | { type: "turn_suspended"; approval_id: string; reason: string }
  | { type: "turn_awaiting_input"; request_id: string }
  | { type: "turn_cancelled"; reason: string }
  | { type: "error"; message: string };

export type SubagentRunStatus =
  "queued" | "running" | "completed" | "failed" | "cancelled" | "timed_out";

export type SubagentRun = {
  id: string;
  parentThreadId: string;
  parentTurnId: string;
  agentPath: string;
  parentAgentPath: string;
  name: string;
  agentType: string;
  input: string;
  forkTurns: string;
  lastTaskMessage: string;
  depth: number;
  status: SubagentRunStatus;
  result?: string | null;
  error?: string | null;
  createdAt: string;
  startedAt?: string | null;
  completedAt?: string | null;
};

export type TurnStatus = {
  turnId: string;
  threadId: string;
  userMessageId: string;
  status:
    | "running"
    | "waiting_approval"
    | "cancelling"
    | "succeeded"
    | "failed"
    | "cancelled"
    | "interrupted";
  startedAt: string;
  updatedAt: string;
  completedAt?: string | null;
  error?: string | null;
};

export type TurnCancelResult = {
  turnId?: string | null;
  cancelled: boolean;
  message: string;
};

declare global {
  interface Window {
    opentopia?: {
      getPlatformInfo(): Promise<PlatformInfo>;
      openExternal(url: string): Promise<void>;
      openPath(targetPath: string): Promise<{ path: string }>;
      showSystemNotification(
        options: SystemNotificationOptions,
      ): Promise<boolean>;
      selectWorkspace(options?: {
        defaultPath?: string;
      }): Promise<WorkspacePickResult>;
      selectContextFiles(options?: {
        defaultPath?: string;
      }): Promise<ContextSourcePickResult>;
      selectPluginDirectory(options?: {
        defaultPath?: string;
      }): Promise<PluginDirectoryPickResult>;
      getRecentWorkspaces(): Promise<RecentWorkspace[]>;
      saveRecentWorkspace(workspaceRoot: string): Promise<RecentWorkspace[]>;
      removeRecentWorkspace(workspaceRoot: string): Promise<RecentWorkspace[]>;
      clearRecentWorkspaces(): Promise<RecentWorkspace[]>;
      listSecretSources(): Promise<SecretSources>;
      setSecret(key: string, value: string): Promise<void>;
      deleteSecret(key: string): Promise<void>;
      getProviderApiKeyMetadata(providerId: string): Promise<KeyringMetadata>;
      setProviderApiKey(
        providerId: string,
        value: string,
      ): Promise<KeyringMetadata>;
      deleteProviderApiKey(providerId: string): Promise<KeyringMetadata>;
      listLogFiles(): Promise<LogFileInfo[]>;
      readLogFile(
        path: string,
        offset?: number,
        limit?: number,
      ): Promise<{ lines: string[]; total: number }>;
      browserHost?: {
        createSession(input: {
          sessionId: string;
          url?: string;
          bounds?: WebPreviewBounds;
          visible?: boolean;
        }): Promise<WebPreviewState>;
        destroySession(sessionId: string): Promise<void>;
        getState(sessionId: string): Promise<WebPreviewState>;
        navigate(sessionId: string, url: string): Promise<unknown>;
        back(sessionId: string): Promise<unknown>;
        forward(sessionId: string): Promise<unknown>;
        reload(sessionId: string): Promise<unknown>;
        setBounds(
          sessionId: string,
          bounds: WebPreviewBounds,
        ): Promise<unknown>;
        setVisibility(sessionId: string, visible: boolean): Promise<unknown>;
        show(sessionId: string, bounds?: WebPreviewBounds): Promise<unknown>;
        hide(sessionId: string): Promise<unknown>;
        onStateChanged(
          listener: (state: WebPreviewState) => void,
        ): (() => void) | void;
      };
    };
  }
}
