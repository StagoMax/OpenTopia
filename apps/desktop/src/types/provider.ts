export type ProviderModelSyncResult = {
  providerId: string;
  models: string[];
  /** Context windows advertised alongside model ids, when the endpoint exposes them. */
  modelContextWindows: Record<string, number>;
  /** Model capabilities returned by the endpoint's catalog. */
  modelCapabilities: Record<string, ProviderModelCapabilities>;
  /** A valid default selected from the models returned by the endpoint. */
  defaultModel: string;
  /** Whether the default model completed adapter capability negotiation. */
  defaultModelReady: boolean;
  /** Non-fatal negotiation failure; the model catalog was still persisted. */
  capabilityWarning?: string;
  syncedAt: string;
  /** Complete persisted connection; profiles may be pending when readiness is false. */
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
  | "omit"
  | "chat_reasoning_effort"
  | "chat_thinking_reasoning_effort"
  | "chat_thinking_high_max_no_tool_choice"
  | "responses_reasoning";

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
  "chat" | "read_only" | "auto" | "approve" | "full_access" | "unrestricted";

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
