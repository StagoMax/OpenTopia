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

export type LibraryProviderId = "sag" | "graph-rag";

export type LibraryProviderServiceRuntimeStatus = {
  provider?: LibraryProviderId;
  state: "ready" | "unavailable";
  endpoint: string;
  managed: boolean;
  canStart: boolean;
  source: string | null;
  message?: string;
};

export type SagServiceRuntimeStatus = LibraryProviderServiceRuntimeStatus;

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

export type InlineImageAttachment = {
  id: string;
  contentType: string;
  data: number[];
  name?: string;
};

export type InlineMessageContentPart =
  { type: "text"; text: string } | { type: "image_ref"; imageId: string };

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
  targetRef?: string | null;
  frameRef?: string | null;
  href?: string | null;
  formAction?: string | null;
  editable: boolean;
};

export type BrowserTarget = {
  targetRef: string;
  url: string;
  title: string;
  active: boolean;
  opener?: string | null;
};

export type BrowserFrame = {
  frameRef: string;
  targetRef: string;
  parentFrameRef?: string | null;
  url: string;
  name: string;
};

export type BrowserAccessibilityNode = {
  axNodeId: string;
  parentAxNodeId?: string | null;
  role: string;
  name: string;
  value?: string | null;
  description?: string | null;
  ignored: boolean;
  targetRef: string;
  frameRef?: string | null;
  nodeRef?: string | null;
};

export type BrowserDialog = {
  dialogType: string;
  message: string;
  defaultPrompt?: string | null;
  handled: boolean;
  targetRef: string;
};

export type BrowserObservation = {
  observationId: string;
  url: string;
  title: string;
  text: string;
  textTruncated: boolean;
  nodes: BrowserNode[];
  targets: BrowserTarget[];
  frames: BrowserFrame[];
  accessibilityTree: BrowserAccessibilityNode[];
  dialogs: BrowserDialog[];
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

export type ExperienceMode = "work" | "code" | "flow";
export type CollaborationMode = "default" | "plan" | "goal";

export type Thread = {
  id: string;
  title: string;
  workspaceRoot: string;
  projectId: string | null;
  experienceMode: ExperienceMode;
  /**
   * Model pinned to this conversation. Pinned at creation so a catalog refresh
   * never swaps the model mid-thread; `null` follows the active connection.
   */
  modelSelection: ThreadModelSelection | null;
  archivedAt: string | null;
  createdAt: string;
  updatedAt: string;
};

export type ThreadModelSelection = {
  connectionId: string;
  modelId: string;
  reasoningEffort: ReasoningEffort | null;
};

export type ProviderModelSyncResult = {
  providerId: string;
  models: string[];
  /** Context windows advertised alongside model ids, when the endpoint exposes them. */
  modelContextWindows: Record<string, number>;
  /** Model capabilities returned by the endpoint's catalog. */
  modelCapabilities: Record<string, ProviderModelCapabilities>;
  /** A valid default selected from the models returned by the endpoint. */
  defaultModel: string;
  syncedAt: string;
  /** Complete persisted connection, including negotiated per-model adapter profiles. */
  provider: ProviderSettings;
};

export type ProviderAdapterKind =
  | "open_ai_chat"
  | "open_ai_responses"
  | "anthropic_messages"
  | "codex_app_server"
  | "mock";

export type ProviderTransportKind = "http" | "codex_app_server" | "mock";

export type ProviderAuthKind =
  "bearer" | "x_api_key" | "codex_session" | "none";

export type ProviderInstructionEncoding =
  "native_roles" | "fold_developer_into_system" | "portable_chat_envelope";

export type ProviderReasoningProtocol =
  | "reasoning_effort"
  | "deep_seek_thinking"
  | "glm_thinking";

export type ProviderMessageProtocolCapabilities = {
  requiresReasoningContentForToolCalls: boolean;
};

export type ProviderOutputProtocolCapabilities = {
  jsonSchema: ProviderFeatureSupport;
};

export type ProviderToolProtocolCapabilities = {
  functionTools: ProviderFeatureSupport;
  strictFunctionTools: ProviderFeatureSupport;
  streamingTools: ProviderFeatureSupport;
  parallelToolCalls: ProviderFeatureSupport;
  freeformTools: ProviderFeatureSupport;
  hostedApplyPatch: ProviderFeatureSupport;
  assistantPhase: ProviderFeatureSupport;
  deferredToolLoading: ProviderFeatureSupport;
  namespaceTools: ProviderFeatureSupport;
  hostedToolSearch: ProviderFeatureSupport;
};

export type ProviderAdapterProfile = {
  profileVersion: number;
  baseUrl: string;
  model: string;
  adapter: ProviderAdapterKind;
  instructionEncoding: ProviderInstructionEncoding;
  reasoningProtocol: ProviderReasoningProtocol;
  messageProtocol: ProviderMessageProtocolCapabilities;
  outputProtocol: ProviderOutputProtocolCapabilities;
  toolProtocol: ProviderToolProtocolCapabilities;
  checkedAt: string;
};

export type ProviderModelCapabilities = {
  /** Omitted when the endpoint does not publish modality metadata. */
  supportsVision?: boolean;
};

export type ProviderModelSettings = {
  /** User overrides that apply only to this model. */
  supportsVision?: boolean;
  /** `undefined` inherits the connection default; `null` omits the parameter. */
  temperature?: number | null;
  maxOutputTokens?: number | null;
  contextWindowTokens?: number | null;
  reasoningEffort?: ReasoningEffort | null;
  preferredAdapter?: ProviderAdapterKind;
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

export type AgentRuntimeSettings = {
  personality: "focused" | "professional" | "warm";
  autonomy: "guided" | "balanced" | "proactive";
  multiAgent: "off" | "explicit" | "adaptive";
  progressUpdates: "milestones" | "balanced" | "frequent";
};

export type ProviderKind =
  | "mock"
  | "openai_compatible"
  | "openai_responses"
  | "anthropic"
  | "codex_app_server";

export type ProviderDriverDescriptor = {
  id: string;
  adapter: ProviderAdapterKind;
  transport: ProviderTransportKind;
  displayName: string;
  trust: "built_in" | "signed";
};

export type OpenAiProtocol = "chat_completions" | "responses";

export type ProviderFeatureSupport = "supported" | "unsupported" | "unknown";

export type OpenAiCompatibilityReport = {
  baseUrl: string;
  model: string;
  selectedProtocol: OpenAiProtocol;
  chatCompletions: ProviderFeatureSupport;
  chatFunctionTools: ProviderFeatureSupport;
  chatStrictFunctionTools: ProviderFeatureSupport;
  chatStreamingTools: ProviderFeatureSupport;
  chatParallelToolCalls: ProviderFeatureSupport;
  chatJsonSchemaOutput: ProviderFeatureSupport;
  chatMessageProtocol: ProviderMessageProtocolCapabilities;
  responses: ProviderFeatureSupport;
  responsesNativeTools: ProviderFeatureSupport;
  responsesFunctionTools: ProviderFeatureSupport;
  responsesStrictFunctionTools: ProviderFeatureSupport;
  responsesStreamingTools: ProviderFeatureSupport;
  responsesParallelToolCalls: ProviderFeatureSupport;
  responsesJsonSchemaOutput: ProviderFeatureSupport;
  responsesCustomTools: ProviderFeatureSupport;
  responsesApplyPatch: ProviderFeatureSupport;
  developerMessages: ProviderFeatureSupport;
  messageCompatibility: boolean;
  checkedAt: string;
  notes: string[];
};

export type AppSettings = {
  providers: ProviderSettings[];
  activeProviderId: string;
  permissionMode: PermissionMode;
  agentRuntime: AgentRuntimeSettings;
  defaultWorkspaceRoot?: string | null;
  sandbox: SandboxSettings;
  enterprise: {
    enabled: boolean;
  };
  parallelToolCallsMigrated: boolean;
  updatedAt: string;
};

export type SandboxSettings = {
  sandboxMode: "read-only" | "workspace-write" | "danger-full-access";
  enforcement: "disabled" | "best-effort" | "enforce";
  network: "inherit" | "allow" | "deny";
  writableRoots: string[];
  readPaths: string[];
  windowsBackend?: "auto" | "dedicated_user" | "elevated" | "unelevated";
};

export type WindowsSandboxSetupStatus = {
  supported: boolean;
  helperAvailable: boolean;
  state: "unavailable" | "not_configured" | "ready" | "degraded";
  backend: "dedicated_user";
  stateDir?: string | null;
  components: {
    credentials: boolean;
    offlineIdentity: boolean;
    onlineIdentity: boolean;
    offlineNetworkPolicy: boolean;
  };
  issues: string[];
};

export type SagLibraryStatus = {
  provider: "SAG";
  endpoint: string;
  status: {
    status: string;
    database?: string | null;
    indexVersion?: string | null;
    embeddingBackend?: string | null;
    embeddingDimensions?: number | null;
    stats: Record<string, number>;
    integrityCheck?: string | null;
    modelLoaded: boolean;
    deepseekConfigured: boolean;
    agentLoopIntegration: boolean;
    promptInjection: boolean;
  };
};

export type SagSource = {
  assetId: string;
  sourceKey: string;
  namespace: string;
  origin: string;
  versionId: string;
  versionNumber: number;
  sourceId: string;
  title: string;
  originalFilename: string;
  contentHash: string;
  storedPath: string;
  metadata: Record<string, unknown>;
  evidenceUnits: number;
  events: number;
  createdAt: string;
};

export type SagSearchRequest = {
  query: string;
  purpose?: string;
  topK?: number;
  maximumTokens?: number;
  useDeepseek?: boolean;
  subjectRefs?: string[];
  namespaces?: string[];
};

export type SagEvidenceNeed = {
  needId: string;
  description: string;
  query: string;
  facets: string[];
  subjectRefs: string[];
  timeMode?: string | null;
  required: boolean;
  weight: number;
};

export type SagNeedCoverage = {
  needId: string;
  required: boolean;
  status: "covered" | "uncovered" | string;
  selectedEventIds: string[];
  reason: string;
};

export type SagContextPackItem = {
  eventId: string;
  evidenceId: string;
  content: string;
  eventSummary: string;
  sourcePath: string;
  title: string;
  sectionPath: string[];
  anchors: string[];
  score: number;
  selectionReason: string;
  matchedNeedIds: string[];
  estimatedTokens: number;
};

export type SagSearchResponse = {
  pack: {
    packId?: string | null;
    status: "draft" | "approved" | "rejected" | string;
    purpose?: string | null;
    query?: string | null;
    plan: {
      requestId?: string | null;
      originalQuery?: string | null;
      purpose?: string | null;
      planner: string;
      needs: SagEvidenceNeed[];
      createdAt?: string | null;
    };
    coverage: SagNeedCoverage[];
    indexVersion?: string | null;
    retrievalEngine?: string | null;
    items: SagContextPackItem[];
    excludedItems: unknown[];
    estimatedTokens: number;
    maximumTokens: number;
    createdAt?: string | null;
  };
  diagnostics: {
    elapsedSeconds: number;
    routeCandidates: Record<string, number>;
    llmRequests: number;
    embeddingBackend?: string | null;
    deepseekEnabled: boolean;
    agentLoopIntegration: boolean;
    promptInjection: boolean;
  };
};

export type SagIngestionResult = {
  jobId: string;
  status: "published" | "unchanged" | string;
  assetId: string;
  versionId: string;
  previousVersionId?: string | null;
  versionNumber: number;
  sourceId: string;
  contentHash: string;
  namespace: string;
  title: string;
  storedPath: string;
  indexVersion: string;
  pipelineSignature: string;
  reusedProjection: boolean;
  evidenceUnits: number;
  events: number;
  entities: number;
  llmRequests: number;
  createdAt: string;
};

export type LibraryProviderDescriptor = {
  id: LibraryProviderId;
  name: string;
  title: string;
  description: string;
  capabilities: {
    graphPaths: boolean;
    temporalMemory: boolean;
    incrementalUpload: boolean;
    llmPlanning: boolean;
  };
};

export type GraphRagLibraryStatus = {
  provider: "Graph RAG";
  endpoint: string;
  status: {
    status: string;
    embeddingBackend?: string | null;
    embeddingDimensions?: number | null;
    rerankerBackend?: string | null;
    vectorBackend?: string | null;
    documents: number;
    chunks: number;
    relations: number;
    indexVersion?: string | null;
    graphEnabled: boolean;
    stats: Record<string, number>;
    agentLoopIntegration: boolean;
    promptInjection: boolean;
  };
};

export type LibraryProviderStatus = SagLibraryStatus | GraphRagLibraryStatus;

export type GraphRagSource = {
  documentId: string;
  title: string;
  owner: string;
  businessClass: string;
  sensitivity: string;
  version: string;
  sourceUri?: string | null;
};

export type LibrarySource = SagSource | GraphRagSource;

export type LibrarySourcePage = {
  items: LibrarySource[];
  total: number;
  authorizedTotal: number;
  indexTotal: number;
  offset: number;
  limit: number;
  hasMore: boolean;
};

export type LibrarySearchRequest = SagSearchRequest & {
  retrievalMode?: "auto" | "hybrid" | "graph";
};

export type GraphRagContextPackItem = {
  itemId: string;
  chunkId: string;
  documentId: string;
  title: string;
  content: string;
  anchor: string;
  sectionTitle?: string | null;
  score: number;
  lexicalScore: number;
  denseScore: number;
  retrievalMode: "hybrid" | "graph";
  graphPath: string[];
  graphRelations: string[];
  selectionReason: string;
  estimatedTokens: number;
};

export type GraphRagSearchResponse = {
  pack: {
    packId: string;
    status: "draft";
    query: string;
    route: string;
    routeReason: string;
    indexVersion: string;
    retrievalEngine: "graph_rag";
    items: GraphRagContextPackItem[];
    graphPaths: Array<{
      nodeIds: string[];
      relations: string[];
      confidence: number;
    }>;
    estimatedTokens: number;
    maximumTokens: number;
    createdAt: string;
  };
  diagnostics: {
    hitCount: number;
    graphUsed: boolean;
    graphPathCount: number;
    embeddingBackend: string;
    agentLoopIntegration: false;
    promptInjection: false;
  };
};

export type LibrarySearchResponse = SagSearchResponse | GraphRagSearchResponse;

export type GraphRagIngestionResult = {
  status: "indexed" | string;
  documentId: string;
  sourceKey: string;
  namespace: string;
  title: string;
  originalFilename: string;
  version: string;
  chunkCount: number;
  indexVersion: string;
  contentHash: string;
};

export type LibraryIngestionResult =
  SagIngestionResult | GraphRagIngestionResult;

export type ReasoningEffort =
  "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max";

export type ProviderSettings = {
  id: string;
  name: string;
  kind: ProviderKind;
  /** Independent connection transport. Missing values are migrated from `kind`. */
  transport?: ProviderTransportKind | null;
  /** Authentication is independent from the selected wire protocol. */
  auth?: ProviderAuthKind | null;
  /** Every protocol this connection may use. */
  allowedAdapters?: ProviderAdapterKind[];
  /** Optional connection-wide protocol preference; absent means automatic. */
  preferredAdapter?: ProviderAdapterKind | null;
  baseUrl: string;
  /**
   * Default model for this connection. Threads may pin a different model; this
   * is the fallback for new threads and internal calls like title generation.
   */
  model: string;
  /**
   * Model families the user allowed for this connection. Empty means "not
   * narrowed yet", which shows every synced family rather than none.
   */
  enabledFamilies: string[];
  /** Model ids from the last `/v1/models` sync, cached for offline use. */
  syncedModels: string[];
  /**
   * Context windows the endpoint reported per model id, captured on sync. This
   * is real capability detection and outranks the server's built-in table.
   */
  modelContextWindows?: Record<string, number>;
  /** Model capabilities automatically discovered from the provider's API. */
  modelCapabilities?: Record<string, ProviderModelCapabilities>;
  /** Negotiated runtime adapter contract per exact model id. */
  adapterProfiles?: Record<
    string,
    Partial<Record<ProviderAdapterKind, ProviderAdapterProfile>>
  >;
  /** Per-model user overrides, preserved across capability re-discovery. */
  modelSettings?: Record<string, ProviderModelSettings>;
  modelsSyncedAt?: string | null;
  /** `null` = don't send temperature, let the model use its vendor default. */
  temperature?: number | null;
  maxOutputTokens?: number | null;
  /**
   * Optional user override. When unset, the server resolves a known model
   * capability and otherwise uses its conservative unknown-model fallback.
   */
  contextWindowTokens: number | null;
  reasoningEffort?: ReasoningEffort | null;
  storeResponses: boolean;
  parallelToolCalls: boolean;
  promptCacheKey?: string | null;
  promptCachePolicy?: "explicit_30m" | "legacy_in_memory" | "legacy_24h" | null;
  responsesCompactionThresholdTokens?: number | null;
  rolloutBudget?: RolloutBudgetSettings | null;
  openaiCompatibility?: OpenAiCompatibilityReport | null;
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
  transport: ProviderTransportKind;
  auth: ProviderAuthKind;
  adapter: ProviderAdapterKind;
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
  openaiCompatibility?: OpenAiCompatibilityReport | null;
};

export type CodexAccountStatus = {
  loggedIn: boolean;
  authMode?: string | null;
  planType?: string | null;
  email?: string | null;
  accountId?: string | null;
  loginPending: boolean;
  loginId?: string | null;
  loginType?: string | null;
  authUrl?: string | null;
  verificationUrl?: string | null;
  userCode?: string | null;
  rateLimits?: unknown;
  usage?: unknown;
};

export type CodexLoginStart = {
  loginId: string;
  loginType: string;
  authUrl?: string | null;
  verificationUrl?: string | null;
  userCode?: string | null;
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

/**
 * Whether the local backend came back after a credential change. Reported
 * separately from the credential write because the secret is already durable
 * when this is produced — a failed restart is recoverable, not a lost key.
 */
export type BackendRestartOutcome = {
  restarted: boolean;
  error: string | null;
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
  backendRestart?: BackendRestartOutcome;
};

/**
 * Result of writing or clearing a provider credential. `stored: true` means the
 * key is persisted, even when `backendRestart.restarted` is false; callers must
 * not discard user input on a restart failure.
 */
export type ProviderSecretOutcome =
  | { stored: true; metadata: KeyringMetadata }
  | { stored: false; error: string };

export type SecretSources = {
  activeProviderKeySource: string | null;
  keyring?: KeyringMetadata;
  sources: SecretSource[];
  notes: string[];
  backendRestart?: BackendRestartOutcome;
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

export type LocalGitOperation =
  | { type: "status"; request: { includeUntracked: boolean } }
  | { type: "branches"; request: { includeRemote: boolean } }
  | { type: "remotes" }
  | { type: "stage"; request: { paths: string[] } }
  | { type: "unstage"; request: { paths: string[] } }
  | { type: "discard"; request: { paths: string[]; confirm: boolean } }
  | {
      type: "create_branch";
      request: { branch: string; startPoint: string | null };
    }
  | { type: "switch_branch"; request: { branch: string } }
  | {
      type: "commit";
      request: { message: string; allTracked: boolean };
    }
  | {
      type: "push";
      request: { remote: string; branch: string; setUpstream: boolean };
    }
  | { type: "fetch"; request: { remote: string | null } }
  | {
      type: "pull";
      request: { remote: string | null; branch: string | null };
    }
  | {
      type: "compare";
      request: { base: string; head: string; mode: "direct" | "merge_base" };
    }
  | {
      type: "create_worktree";
      request: {
        path: string;
        target:
          | { type: "existing_branch"; branch: string }
          | {
              type: "new_branch";
              branch: string;
              startPoint: string | null;
            };
      };
    }
  | { type: "list_worktrees" }
  | { type: "remove_worktree"; request: { path: string; confirm: boolean } };

export type NormalizedGitRemoteUrl = {
  normalized: string;
  scheme: string | null;
  host: string | null;
  port: number | null;
  repositoryPath: string;
};

export type LocalGitRemote = {
  name: string;
  fetchUrls: NormalizedGitRemoteUrl[];
  pushUrls: NormalizedGitRemoteUrl[];
};

export type LocalGitStatus = {
  branch: string | null;
  aheadBehind: { ahead: number; behind: number } | null;
  porcelainV2: string;
};

export type LocalGitWorktree = {
  path: string;
  head: string | null;
  branch: string | null;
  detached: boolean;
  bare: boolean;
  locked: boolean;
  lockReason: string | null;
  prunable: boolean;
  prunableReason: string | null;
};

export type LocalGitOutput =
  | { type: "status"; value: LocalGitStatus }
  | { type: "branches"; value: GitBranchInfo[] }
  | { type: "remotes"; value: LocalGitRemote[] }
  | { type: "worktrees"; value: LocalGitWorktree[] }
  | { type: "compare"; value: number[] }
  | { type: "mutation"; value: number[] };

export type LocalGitResponse = {
  apiVersion: "localGit.v1" | string;
  operation:
    | "status"
    | "list_branches"
    | "list_remotes"
    | "stage"
    | "unstage"
    | "discard"
    | "create_branch"
    | "switch_branch"
    | "commit"
    | "push"
    | "fetch"
    | "pull"
    | "compare"
    | "create_worktree"
    | "list_worktrees"
    | "remove_worktree";
  command: {
    exitCode: number | null;
    success: boolean;
    truncated: boolean;
    stderr: number[];
  };
  output: LocalGitOutput;
};

export type ScmConnectorCapability =
  | "change_requests"
  | "issues"
  | "automation"
  | "reviews"
  | "releases"
  | "repository_identity";

export type ScmHostMatcher =
  | { type: "exact"; value: string }
  | { type: "suffix"; value: string }
  | { type: "any" };

export type ScmPathMatcher =
  | { type: "exact"; value: string }
  | { type: "prefix"; value: string }
  | { type: "any" };

export type ScmConnectorDescriptor = {
  pluginId: string;
  connectorId: string;
  displayName: string;
  capabilities: ScmConnectorCapability[];
  remoteMatchers: Array<{
    matcherId: string;
    schemes: string[];
    host: ScmHostMatcher;
    path: ScmPathMatcher;
  }>;
};

export type ScmRemoteBinding = {
  workspaceKey: string;
  remoteName: string;
  connectorPluginId: string;
  connectorId: string;
  accountBindingId: string | null;
};

export type ScmConnectorCandidate = {
  pluginId: string;
  connectorId: string;
  matcherId: string;
  specificity: { host: number; path: number; scheme: number };
};

export type ScmConnectorSelection =
  | { status: "unmatched" }
  | {
      status: "selected";
      candidate: ScmConnectorCandidate;
      source: "best_match" | "remote_binding";
      accountBindingId: string | null;
    }
  | {
      status: "conflict";
      candidates: ScmConnectorCandidate[];
      bindingIssue:
        | "wrong_workspace_or_remote"
        | "connector_unavailable"
        | "connector_not_best_match"
        | null;
    };

export type ScmRemoteConnectorResponse = {
  remote: LocalGitRemote;
  connectors: ScmConnectorDescriptor[];
  binding: ScmRemoteBinding | null;
  selection: ScmConnectorSelection;
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

export type McpToolDescriptor = {
  publicName: string;
  serverId: string;
  toolName: string;
  description?: string | null;
  inputSchema: unknown;
  annotations: unknown;
  permissionLabels: string[];
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
  | ({ type: "image" } & Omit<InlineImageAttachment, "id"> & { id?: string })
  | { type: "image_ref"; image_id: string }
  | { type: "tool_call"; call: ToolCall }
  | { type: "tool_result"; result: ToolResult }
  | { type: "file_ref"; path: string }
  | { type: "source_ref"; source: ContextSourceRef }
  | { type: "skill_ref"; skill: SkillRef }
  | {
      type: "turn_context";
      collaboration_mode: CollaborationMode;
      goal_id?: string | null;
      library_provider?: LibraryProviderId | null;
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
  source: "workspace" | "user" | "codex" | "bundled";
  managed: boolean;
  trust: "standard" | "official" | "privileged" | "trusted_driver";
  defaultEnabled: boolean;
  nativeCapabilities: string[];
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

export type PluginControlScopeType = "global" | "workspace" | "thread";

export type PluginControlScope = {
  scopeType: PluginControlScopeType;
  scopeId?: string;
};

export type PluginActivationRecord = {
  pluginId: string;
  scope: PluginControlScope;
  enabled: boolean;
  updatedAt: string;
};

export type PluginSettingsRecord = {
  pluginId: string;
  scope: PluginControlScope;
  settings: unknown;
  updatedAt: string;
};

export type PluginSecretBindingRecord = {
  pluginId: string;
  scope: PluginControlScope;
  settingKey: string;
  bindingId: string;
  metadata: unknown;
  updatedAt: string;
};

export type PluginPermissionGrantStatus = "granted" | "revoked";

export type PluginPermissionGrantRecord = {
  pluginId: string;
  scope: PluginControlScope;
  permission: string;
  constraint: unknown;
  status: PluginPermissionGrantStatus;
  grantedAt?: string;
  updatedAt: string;
};

export type PluginPermissionRequest = {
  category: string;
  value: string;
  permission: string;
};

export type PluginContributionRecord = {
  pluginId: string;
  contributionId: string;
  kind: string;
  localId: string;
  descriptor: unknown;
  updatedAt: string;
};

export type PluginRuntimeHealthStatus =
  "unknown" | "ready" | "degraded" | "error" | "stopped";

export type PluginRuntimeHealthRecord = {
  pluginId: string;
  contributionId: string;
  status: PluginRuntimeHealthStatus;
  lastError?: string;
  lastCheckedAt: string;
  restartCount: number;
};

export type PluginControlManifest = {
  apiVersion?: string;
  hostCapabilities: string[];
  permissionRequests: PluginPermissionRequest[];
  configurationSchema?: unknown;
  secretSettingKeys: string[];
  requiredSecretSettingKeys: string[];
  contributions: PluginContributionRecord[];
};

export type PluginDetail = {
  plugin: PluginDescriptor;
  manifest: PluginControlManifest;
  activations: PluginActivationRecord[];
  effectiveEnabled: boolean;
  contributions: PluginContributionRecord[];
  health: PluginRuntimeHealthRecord[];
};

export type PluginActivationResponse = {
  activation: PluginActivationRecord;
  effectiveEnabled: boolean;
};

export type PluginSettingsResponse = {
  schema?: unknown;
  settings: PluginSettingsRecord;
  secretBindings: PluginSecretBindingRecord[];
};

export type PluginPermissionsResponse = {
  requests: PluginPermissionRequest[];
  grants: PluginPermissionGrantRecord[];
};

export type PluginContributionKind =
  | "skill"
  | "mcp_server"
  | "native_tool"
  | "previewer"
  | "context_loader"
  | "agent_profile"
  | "scm_connector"
  | "app";

export type PluginCapabilityPermission = {
  kind: "filesystem" | "network" | "secret" | "desktop";
  value: string;
};

export type PluginCapabilityContribution = {
  id: string;
  pluginId: string;
  localId: string;
  kind: PluginContributionKind;
  origin: "codex_compatible" | "open_topia";
  apiVersion: string;
  requiredHostCapabilities: string[];
  permissions: PluginCapabilityPermission[];
  configurationSchema?: string | null;
  declaration: unknown;
};

export type ActivatedPluginContribution = {
  pluginName: string;
  source: PluginDescriptor["source"];
  trust: PluginDescriptor["trust"];
  contribution: PluginCapabilityContribution;
};

export type CapabilityUnavailableReason =
  | "disabled"
  | "host_trust_required"
  | "conflict"
  | { missing_host_capabilities: string[] }
  | { missing_permissions: PluginCapabilityPermission[] };

export type CapabilityActivationSnapshot = {
  scope: {
    workspaceId?: string | null;
    threadId?: string | null;
  };
  active: ActivatedPluginContribution[];
  unavailable: Array<{
    contribution: ActivatedPluginContribution;
    reason: CapabilityUnavailableReason;
  }>;
  conflicts: Array<{
    key: string;
    contributionIds: string[];
  }>;
};

export type ThreadPluginCapabilities = {
  pluginId: string;
  pluginName: string;
  enabled: boolean;
  contributions: PluginContributionRecord[];
  grantedPermissions: string[];
};

export type CapabilityProjection = {
  allowAllTools: boolean;
  tools: string[];
  allowAllSkills: boolean;
  skills: string[];
  allowAllPlugins: boolean;
  plugins: string[];
  allowAllMcpServers: boolean;
  mcpServers: string[];
  allowAllWorkspaceRoots: boolean;
  workspaceRoots: string[];
};

export type DataClassification =
  "public" | "internal" | "confidential" | "restricted";

export type ExecutionResourceGrant = {
  bindingId: string;
  kind: "file" | "network" | "database";
  resource: string;
  canRead: boolean;
  canWrite: boolean;
  maxDataClassification: DataClassification;
};

export type AgentModelBinding = {
  providerId: string;
  modelId: string;
};

export type AgentModelPolicy = {
  allowAllModels: boolean;
  allowedModels: AgentModelBinding[];
};

export type AgentTemplateSpec = {
  description: string;
  instructions: string;
  capabilities: CapabilityProjection;
  resourceGrants: ExecutionResourceGrant[];
  modelPolicy: AgentModelPolicy;
  stateSchema: unknown;
  outputSchema: unknown;
  allowAllDelegates: boolean;
  delegateTemplateIds: string[];
  budget: {
    maxTurns: number;
    maxToolCalls: number;
    maxDurationSeconds: number;
  };
  riskClass: "low" | "medium" | "high" | "critical";
};

export type AgentTemplateVersion = {
  schemaVersion: number;
  templateId: string;
  version: number;
  name: string;
  owner: string;
  spec: AgentTemplateSpec;
  status: "draft" | "published";
  contentHash: string;
  createdAt: string;
  publishedAt: string | null;
  publishedBy: string | null;
};

export type AgentCapabilityChange = {
  scope: string;
  value: string;
  kind: "added" | "removed" | "expanded" | "reduced";
};

export type AgentTemplateVersionView = {
  template: AgentTemplateVersion;
  diff: {
    fromVersion: number | null;
    toVersion: number;
    changes: AgentCapabilityChange[];
    widensCapabilities: boolean;
  };
};

export type AgentInstanceStatus =
  "active" | "suspended" | "completed" | "revoked";

export type EnterpriseExecutionContext = {
  schemaVersion: number;
  agentId: string;
  threadId: string;
  mode: ExperienceMode;
  templateId: string;
  templateVersion: number;
  parentAgentId: string | null;
  delegationChain: string[];
  capabilities: CapabilityProjection;
  resourceGrants: ExecutionResourceGrant[];
  modelPolicy: AgentModelPolicy;
};

export type AgentInstance = {
  schemaVersion: number;
  id: string;
  templateId: string;
  templateVersion: number;
  threadId: string;
  parentInstanceId: string | null;
  delegationDepth: number;
  executionContext: EnterpriseExecutionContext;
  state: unknown;
  stateRevision: number;
  status: AgentInstanceStatus;
  createdAt: string;
  updatedAt: string;
};

export type FlowSource =
  | { kind: "natural_language"; description: string }
  | { kind: "run_trace"; runId: string; traceHash: string };

export type FlowNodeKind =
  | "agent"
  | "skill"
  | "tool"
  | "condition"
  | "validator"
  | "approval"
  | "join"
  | "loop"
  | "output";

export type FlowGraphNode = {
  id: string;
  label: string;
  kind: FlowNodeKind;
  config: Record<string, unknown>;
  inputSchema: Record<string, unknown>;
  outputSchema: Record<string, unknown>;
};

export type FlowGraphEdge = {
  from: string;
  to: string;
  condition: string | null;
  allowedFields: string[];
  dataClassification: DataClassification;
  onError: string | null;
  loopPolicy: {
    maxIterations: number;
    continueCondition: string;
    onExhausted: "require_human" | "return_partial" | "fail";
  } | null;
};

export type FlowSpec = {
  flowId: string;
  name: string;
  description: string;
  owner: string;
  categories: string[];
  source: FlowSource;
  inputSchema: Record<string, unknown>;
  outputSchema: Record<string, unknown>;
  graph: {
    schemaVersion: number;
    entryNodeId: string;
    nodes: FlowGraphNode[];
    edges: FlowGraphEdge[];
  };
  requestedCapabilities: CapabilityProjection;
  budget: {
    maxNodeExecutions: number;
    maxToolCalls: number;
    maxDurationSeconds: number;
    maxLoopIterations: number;
  };
  riskClass: "low" | "medium" | "high" | "critical";
  pendingDecisions: string[];
};

export type FlowValidationIssue = {
  severity: "error" | "warning";
  code: string;
  message: string;
  nodeId: string | null;
  edgeIndex: number | null;
  remediation: string;
};

export type FlowValidationReport = {
  valid: boolean;
  issues: FlowValidationIssue[];
  validatedAt: string;
};

export type FlowDraft = {
  schemaVersion: number;
  id: string;
  threadId: string;
  revision: number;
  status:
    "drafting" | "reviewing" | "validating" | "ready_to_publish" | "published";
  spec: FlowSpec;
  effectiveCapabilities: CapabilityProjection;
  contentHash: string;
  lastValidation: FlowValidationReport | null;
  createdAt: string;
  updatedAt: string;
};

export type FlowTrial = {
  schemaVersion: number;
  id: string;
  draftId: string;
  draftRevision: number;
  status: "passed" | "failed";
  input: unknown;
  steps: Array<{
    order: number;
    nodeId: string;
    harnessTarget: string;
    boundedBy: number | null;
  }>;
  report: FlowValidationReport;
  createdAt: string;
};

export type FlowDraftView = {
  draft: FlowDraft;
  trials: FlowTrial[];
};

export type FlowDefinition = {
  schemaVersion: number;
  id: string;
  flowId: string;
  name: string;
  version: number;
  owner: string;
  description: string;
  categories: string[];
  source: FlowSource;
  graph: FlowSpec["graph"];
  inputSchema: Record<string, unknown>;
  outputSchema: Record<string, unknown>;
  capabilities: CapabilityProjection;
  budget: FlowSpec["budget"];
  riskClass: FlowSpec["riskClass"];
  contentHash: string;
  publishedAt: string;
  publishedBy: string;
};

export type FlowRunStatus =
  | "queued"
  | "running"
  | "pause_requested"
  | "paused"
  | "waiting_approval"
  | "succeeded"
  | "failed"
  | "cancel_requested"
  | "cancelled";

export type FlowNodeRun = {
  id: string;
  nodeId: string;
  attempt: number;
  status: "running" | "waiting_approval" | "succeeded" | "failed" | "cancelled";
  input: unknown;
  output: unknown | null;
  error: string | null;
  toolCalls: number;
  transcript: FlowTranscriptEntry[];
  startedAt: string;
  completedAt: string | null;
};

export type FlowTranscriptEntry = {
  id: string;
  kind: "input" | "tool_call" | "tool_result" | "output" | "approval" | "error";
  title: string;
  content: unknown;
  toolName?: string | null;
  callId?: string | null;
  isError: boolean;
  createdAt: string;
};

export type FlowRun = {
  schemaVersion: number;
  id: string;
  threadId: string;
  flowId: string;
  flowVersion: number;
  definitionId: string;
  definitionContentHash: string;
  revision: number;
  status: FlowRunStatus;
  input: unknown;
  output: unknown | null;
  graph: FlowSpec["graph"];
  effectiveCapabilities: CapabilityProjection;
  budget: FlowSpec["budget"];
  readyNodes: string[];
  nodeRuns: FlowNodeRun[];
  nodeOutputs: Record<string, unknown>;
  loopCounts: Record<string, number>;
  nodeExecutions: number;
  toolCalls: number;
  waitingNodeId: string | null;
  error: string | null;
  createdAt: string;
  startedAt: string | null;
  completedAt: string | null;
  updatedAt: string;
};

export type ThreadCapabilities = {
  threadId: string;
  experienceMode: ExperienceMode;
  promptProfileId: string;
  capabilityProjection: CapabilityProjection;
  workspaceRoot: string;
  generatedAt: string;
  snapshot: CapabilityActivationSnapshot;
  plugins: ThreadPluginCapabilities[];
};

export type MediaHandlerDescriptor = {
  contributionId: string;
  pluginId: string;
  localId: string;
  kind: "previewer" | "context_loader";
  extensions: string[];
  mediaTypes: string[];
  priority: number;
  runtime: string;
};

export type MediaHandlerSelection =
  | { status: "none" }
  | { status: "selected"; handler: MediaHandlerDescriptor }
  | { status: "conflict"; contributionIds: string[] };

export type MediaHandlerOperation = "preview" | "load_context";

export type MediaHandlerRuntime =
  | { type: "mcp_v1"; server: string; tool: string }
  | { type: "builtin"; adapter: string };

export type MediaHandlerResult = {
  apiVersion: "opentopia.mediaHandlerResult.v1" | string;
  kind: MediaHandlerOperation;
  payload: unknown;
};

export type MediaHandlerInvocationResponse = {
  contributionId: string;
  pluginId: string;
  runtime: MediaHandlerRuntime;
  bytesRead: number;
  output: MediaHandlerResult;
};

export type AppViewDescriptor = {
  contributionId: string;
  pluginId: string;
  localId: string;
  title: string;
  entry: string;
  allowedChannels: string[];
  sandbox: {
    nodeIntegration: false;
    allowPopups: false;
    allowTopNavigation: false;
    allowedHostApis: string[];
  };
};

export type AppViewSession = {
  sessionId: string;
  threadId: string;
  descriptor: AppViewDescriptor;
  status: "ready" | "stopped";
  startedAt: string;
  stoppedAt?: string | null;
};

export type AppViewSessionResponse = AppViewSession & {
  contentPath: string;
};

export type AppViewMessage = {
  sessionId: string;
  channel: string;
  payload: unknown;
  sentAt: string;
};

export type PluginAgentProfile = {
  name: string;
  description: string;
  developer_instructions: string;
  nickname_candidates: string[];
  model?: string | null;
  model_reasoning_effort?: string | null;
  sandbox_mode?: "read-only" | "workspace-write" | "danger-full-access" | null;
  allowed_tools?: string[] | null;
  denied_tools: string[];
  source_plugin_id?: string | null;
  source_contribution_id?: string | null;
};

export type ContributionHostSnapshot = {
  previewers: MediaHandlerDescriptor[];
  contextLoaders: MediaHandlerDescriptor[];
  apps: AppViewDescriptor[];
  agentProfiles: PluginAgentProfile[];
  issues: string[];
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

export type WorkFormStatus =
  "active" | "completed" | "blocked" | "paused" | "cancelled";

export type WorkItemStatus =
  | "pending"
  | "in_progress"
  | "completed"
  | "deferred"
  | "blocked"
  | "cancelled";

export type CompletionDisposition = "blocking" | "advisory";

export type WorkScope =
  { kind: "turn"; id: string } | { kind: "goal"; id: string };

export type WorkItem = {
  id: string;
  title: string;
  status: WorkItemStatus;
  completionDisposition: CompletionDisposition;
  dependsOn: string[];
  note?: string | null;
  acceptance: string[];
  evidenceRefs: string[];
};

export type WorkForm = {
  id: string;
  threadId: string;
  scope: WorkScope;
  objective: string;
  constraints: string[];
  acceptance: string[];
  status: WorkFormStatus;
  revision: number;
  changeReason?: string | null;
  items: WorkItem[];
  createdAt: string;
  updatedAt: string;
};

export type GoalStatus = WorkFormStatus;

export type GoalRecord = {
  id: string;
  threadId: string;
  objective: string;
  tokenBudget?: number | null;
  tokensUsed: number;
  timeUsedSeconds: number;
  version: number;
  createdAt: string;
  updatedAt: string;
};

export type GoalSnapshot = {
  goal: GoalRecord;
  workForm: WorkForm;
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
  previousResponseId?: string | null;
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
    | "capability_catalog"
    | "skill_instructions"
    | "skill"
    | "summary"
    | "checkpoint"
    | "conversation"
    | "user"
    | "tool_call"
    | "tool_result";
  role: "system" | "developer" | "user" | "assistant" | "tool";
  /** Harness semantic authority; never emitted as a Provider prompt role. */
  authority: "system" | "developer" | "user" | "assistant" | "tool" | "data";
  /** Harness lifetime metadata; never emitted as a Provider prompt tag. */
  lifecycle: "build" | "thread" | "epoch" | "turn" | "round";
  source: string;
  content: ModelContentPart[];
  contentHash: string;
  tokenEstimate: number;
  /** Internal cache/placement segment, not a Provider-supported lifecycle. */
  cacheScope: "stable" | "thread" | "turn" | "round" | "none";
  sensitivity: "public" | "workspace" | "sensitive";
  metadata?: unknown;
};

export type ModelCallPurpose =
  | "agent_round"
  | "context_compaction"
  | "guardian_review"
  | "title_generation"
  | "other";

export type TokenEstimateBreakdown = {
  baseInstructions: number;
  developerInstructions: number;
  repositoryInstructions: number;
  runtimeContext: number;
  skillInstructions: number;
  summaries: number;
  checkpoints: number;
  conversation: number;
  currentUser: number;
  toolCalls: number;
  toolResults: number;
  directToolSchemas?: number;
  deferredToolCatalog?: number;
  loadedToolSchemas?: number;
  toolSchemas: number;
  providerState: number;
  other: number;
  total: number;
};

export type ThreadContextSnapshot = {
  capturedAt: string;
  providerId: string;
  providerKind: string;
  providerAdapter?: string;
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
  skipped?: boolean;
  cancelled?: boolean;
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
      purpose?: ModelCallPurpose;
      token_breakdown?: TokenEstimateBreakdown | null;
      items?: ModelContextItem[];
    }
  | {
      type: "model_request";
      request_id: string;
      round: number;
      request?: ModelRequestSnapshot | null;
    }
  | {
      type: "provider_request_sent";
      request_id: string;
      round: number;
      attempt: number;
      adapter: string;
      method: string;
      endpoint: string;
      body?: unknown;
    }
  | {
      type: "provider_request_retried";
      request_id: string;
      round: number;
      attempt: number;
      retry_kind?: "network" | "state_recovery";
      retry_index?: number | null;
      retry_limit?: number | null;
      reason: string;
      body?: unknown;
    }
  | {
      type: "provider_response_received";
      request_id: string;
      round: number;
      attempt: number;
      status?: number | null;
      response_id?: string | null;
      body?: unknown;
    }
  | { type: "model_delta"; text: string }
  | { type: "reasoning_delta"; text: string }
  | { type: "tool_call_started"; call: ToolCall }
  | { type: "tool_call_finished"; result: ToolResult }
  | { type: "work_form_updated"; form: WorkForm }
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
  | {
      type: "approval_requested";
      approval_id: string;
      reason: string;
      action: string;
    }
  | {
      type: "browser_handoff_required";
      action: string;
      reason: string;
      url?: string | null;
    }
  | { type: "browser_handoff_completed"; prior_turn_id: string }
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
      status:
        | "in_progress"
        | "approved"
        | "needs_user_approval"
        | "denied_by_policy"
        | "reviewer_unavailable"
        | "invalid_reviewer_response"
        | "aborted";
      risk_level?: "low" | "medium" | "high" | "critical" | null;
      user_authorization?: "unknown" | "low" | "medium" | "high" | null;
      rationale: string;
      action: unknown;
      usage: {
        inputTokens: number;
        outputTokens: number;
        totalTokens: number;
        cachedInputTokens?: number | null;
        cacheWriteTokens?: number | null;
        reasoningTokens?: number | null;
      };
      attempts: number;
      tool_rounds: number;
      decision_source: "guardian" | "runtime";
      failure_kind?:
        "reviewer_unavailable" | "invalid_reviewer_response" | null;
    }
  | { type: "auto_review_interruption_warning"; message: string }
  | {
      type: "context_compacted";
      summary: ContextSummary;
      details?: ContextCompactionDetails | null;
    }
  | { type: "context_projection_built"; projection: ContextProjection }
  | {
      type: "provider_context_state_updated";
      provider_id: string;
      model: string;
      state_kind: string;
      response_item_count: number;
      compaction_item_count: number;
    }
  | {
      type: "provider_context_state_invalidated";
      provider_id?: string | null;
      model?: string | null;
      reason: string;
    }
  | { type: "context_warning"; stage: string; message: string }
  | {
      type: "token_usage";
      request_id?: string | null;
      round?: number | null;
      purpose?: ModelCallPurpose;
      input_tokens: number;
      output_tokens: number;
      total_tokens: number;
      cached_input_tokens?: number | null;
      cache_write_tokens?: number | null;
      reasoning_tokens?: number | null;
      local_input_estimate?: number | null;
      input_breakdown?: TokenEstimateBreakdown | null;
    }
  | { type: "turn_finished"; summary: string }
  | { type: "turn_suspended"; approval_id: string; reason: string }
  | { type: "turn_awaiting_input"; request_id: string }
  | { type: "turn_cancelled"; reason: string }
  | { type: "error"; message: string };

export type AgentTurnStatus =
  | "queued"
  | "running"
  | "waiting_approval"
  | "waiting_input"
  | "waiting_action"
  | "completed"
  | "failed"
  | "cancelled"
  | "interrupted";

export type AgentAvailability =
  | "idle"
  | "queued"
  | "running"
  | "needs_attention"
  | "archived";

export type AgentSpawnPolicy = {
  allowChildSpawns: boolean;
  maxDepth: number;
  maxDirectChildren: number;
};

export type AgentThread = {
  id: string;
  sessionId: string;
  parentAgentThreadId?: string | null;
  path: string;
  taskName: string;
  agentType: string;
  runtimeSnapshotId: string;
  spawnPolicy: AgentSpawnPolicy;
  createdAt: string;
  archivedAt?: string | null;
};

export type AgentTurn = {
  id: string;
  sessionId: string;
  agentThreadId: string;
  requestedByAgentThreadId?: string | null;
  requestedByTurnId?: string | null;
  sequence: number;
  taskMessage: string;
  status: AgentTurnStatus;
  invocationId: number;
  outcomeRef?: string | null;
  createdAt: string;
  startedAt?: string | null;
  completedAt?: string | null;
};

export type AgentActivityEventDetails =
  | { type: "model_round"; round: number }
  | {
      type: "tool_call_started";
      invocation_id: string;
      tool_name: string;
      input_preview: unknown;
    }
  | {
      type: "tool_call_finished";
      invocation_id: string;
      tool_name?: string | null;
    }
  | { type: "waiting"; reason: string }
  | { type: "error"; message: string };

export type AgentActivityEvent = {
  seq: number;
  kind: string;
  createdAt: string;
  details?: AgentActivityEventDetails | null;
};

export type AgentToolResultProjection = {
  invocationId: string;
  toolName?: string | null;
  kind: "text" | "json" | "resource" | "binary" | "mixed";
  preview: unknown;
  truncated: boolean;
  resultRef: string;
};

export type AgentActivityWindow = {
  agentThreadId: string;
  agentTurnId: string;
  turnStatus: AgentTurnStatus;
  modelRound?: number | null;
  cursor: number;
  reasoningTail?: string | null;
  recentEvents: AgentActivityEvent[];
  recentToolResults: AgentToolResultProjection[];
};

export type AgentActivityNotification = {
  seq: number;
  agentThreadId: string;
};

export type AgentListItem = {
  agent: AgentThread;
  latestTurn?: AgentTurn | null;
  availability: AgentAvailability;
  activity?: AgentActivityWindow | null;
};

export type TurnStatus = {
  turnId: string;
  threadId: string;
  userMessageId: string;
  status:
    | "running"
    | "waiting_approval"
    | "waiting_user_input"
    | "waiting_user_action"
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

export type PlatformOpenRequest = {
  id: string;
  source: string;
  kind: string;
  action?: string;
  threadId?: string;
  workspaceRoot?: string;
  path?: string;
  receivedAt: string;
};

export type FileLinkAction =
  "open-default" | "open-vscode" | "open-with" | "save-as" | "reveal";

export type FileLinkActionRequest = {
  action: FileLinkAction;
  path: string;
  line?: number | null;
};

export type FileLinkActionResult = {
  canceled?: boolean;
  path?: string;
};

declare global {
  interface Window {
    opentopia?: {
      newWindow(): Promise<boolean>;
      closeWindow(): Promise<boolean>;
      quit(): Promise<boolean>;
      getPlatformInfo(): Promise<PlatformInfo>;
      ensureSagLibraryService(): Promise<SagServiceRuntimeStatus>;
      ensureLibraryProviderService(
        provider: LibraryProviderId,
      ): Promise<LibraryProviderServiceRuntimeStatus>;
      getOpenRequests(): Promise<PlatformOpenRequest[]>;
      onOpenRequest(
        listener: (request: PlatformOpenRequest) => void,
      ): () => void;
      /** Repaints the OS-drawn window chrome for the resolved theme. */
      setTheme(theme: "light" | "dark"): Promise<boolean>;
      openExternal(url: string): Promise<void>;
      openPath(targetPath: string): Promise<{ path: string }>;
      performFileLinkAction(
        request: FileLinkActionRequest,
      ): Promise<FileLinkActionResult>;
      showSystemNotification(
        options: SystemNotificationOptions,
      ): Promise<boolean>;
      writeClipboardImage(bytes: Uint8Array): Promise<boolean>;
      selectWorkspace(options?: {
        defaultPath?: string;
      }): Promise<WorkspacePickResult>;
      selectContextFiles(options?: {
        defaultPath?: string;
      }): Promise<ContextSourcePickResult>;
      getDroppedContextFiles(files: File[]): Promise<ContextSourcePickResult>;
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
      recordConversationRenderTrace?(trace: {
        stage: "received" | "committed" | "painted";
        channel: "assistant" | "commentary" | "reasoning" | "status";
        threadId: string;
        turnId?: string | null;
        eventId?: string;
        messageId?: string;
        seq?: number;
        sourceCreatedAt?: string;
        rendererAt: string;
        rendererClockMs: number;
        latencyMs?: number;
        change: "append" | "replace";
        text: string;
        textLength: number;
        visible: boolean;
      }): void;
      browserHost?: {
        createSession(input: {
          sessionId: string;
          profileId?: string;
          profilePersistence?: BrowserProfilePersistence;
          url?: string;
          bounds?: WebPreviewBounds;
          visible?: boolean;
        }): Promise<WebPreviewState>;
        destroySession(sessionId: string): Promise<void>;
        getState(sessionId: string): Promise<WebPreviewState>;
        navigate(sessionId: string, url: string): Promise<unknown>;
        navigateFromAddressBar(
          sessionId: string,
          url: string,
        ): Promise<unknown>;
        beginUserControl(sessionId: string): Promise<unknown>;
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
      chromeBridge?: {
        startPairing(sessionId: string): Promise<ChromeBridgeState>;
        getStatus(sessionId: string): Promise<ChromeBridgeState>;
        disconnect(sessionId: string): Promise<ChromeBridgeState>;
        runAction(
          sessionId: string,
          action: "navigate" | "back" | "forward" | "reload",
          value?: string,
        ): Promise<ChromeBridgeState>;
        onStateChanged(
          listener: (state: ChromeBridgeState) => void,
        ): (() => void) | void;
      };
    };
  }
}
