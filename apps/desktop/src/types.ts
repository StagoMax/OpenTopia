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

export type Thread = {
  id: string;
  title: string;
  workspaceRoot: string;
  projectId: string | null;
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

export type ProviderKind = "mock" | "openai_compatible";

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
  apiKeySource: string;
  apiKeyConfigured: boolean;
  healthStatus?: string | null;
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
  createdAt: string;
  updatedAt: string;
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

export type SkillDescriptor = {
  id: string;
  name: string;
  description: string;
  path: string;
  scope: "workspace" | "user";
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

export type AgentEvent = {
  id: string;
  threadId: string;
  turnId?: string | null;
  seq: number;
  createdAt: string;
  payload: AgentEventPayload;
};

export type AgentEventPayload =
  | { type: "turn_started"; user_message_id: string }
  | { type: "model_delta"; text: string }
  | { type: "tool_call_started"; call: ToolCall }
  | { type: "tool_call_finished"; result: ToolResult }
  | { type: "assistant_message"; message: Message }
  | { type: "file_changed"; path: string; summary: string }
  | { type: "subagent_updated"; run: SubagentRun }
  | {
      type: "approval_requested";
      approval_id: string;
      reason: string;
      action: string;
    }
  | { type: "context_compacted"; summary: ContextSummary }
  | {
      type: "token_usage";
      input_tokens: number;
      output_tokens: number;
      total_tokens: number;
    }
  | { type: "turn_finished"; summary: string }
  | { type: "turn_suspended"; approval_id: string; reason: string }
  | { type: "turn_cancelled"; reason: string }
  | { type: "error"; message: string };

export type SubagentRunStatus =
  "queued" | "running" | "completed" | "failed" | "cancelled" | "timed_out";

export type SubagentRun = {
  id: string;
  parentThreadId: string;
  parentTurnId: string;
  name: string;
  input: string;
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
  status: "running" | "cancelling";
  startedAt: string;
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
      selectWorkspace(options?: {
        defaultPath?: string;
      }): Promise<WorkspacePickResult>;
      selectContextFiles(options?: {
        defaultPath?: string;
      }): Promise<ContextSourcePickResult>;
      getRecentWorkspaces(): Promise<RecentWorkspace[]>;
      saveRecentWorkspace(workspaceRoot: string): Promise<RecentWorkspace[]>;
      removeRecentWorkspace(workspaceRoot: string): Promise<RecentWorkspace[]>;
      clearRecentWorkspaces(): Promise<RecentWorkspace[]>;
      listSecretSources(): Promise<SecretSources>;
      setSecret(key: string, value: string): Promise<void>;
      deleteSecret(key: string): Promise<void>;
      listLogFiles(): Promise<LogFileInfo[]>;
      readLogFile(
        path: string,
        offset?: number,
        limit?: number,
      ): Promise<{ lines: string[]; total: number }>;
    };
  }
}
