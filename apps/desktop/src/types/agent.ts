import type {
  Message,
  ModelContentPart,
  ToolCall,
  ToolResult,
} from "./messages";
import type {
  ContextCompactionDetails,
  ContextProjection,
  ContextSummary,
  TurnChangeSet,
} from "./workspace";

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

export type TokenEstimateDetail = {
  id: string;
  label: string;
  tokens: number;
  children?: TokenEstimateDetail[];
};

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
  outputSchema?: number;
  turnAssistantState?: number;
  providerState: number;
  other: number;
  total: number;
  details?: TokenEstimateDetail[];
};

export type ProviderCacheTraceSegmentKind =
  | "instructions"
  | "system_message"
  | "developer_message"
  | "user_message"
  | "assistant_message"
  | "tool_call"
  | "tool_result"
  | "tool_image"
  | "input_item"
  | "unknown";

export type ProviderCacheTraceSegment = {
  kind: ProviderCacheTraceSegmentKind;
  source: string;
  name?: string | null;
  contentHash: string;
  tokenEstimate: number;
};

export type ProviderCacheTrace = {
  schemaVersion: number;
  prefixHash: string;
  segments: ProviderCacheTraceSegment[];
  toolCatalogHash?: string | null;
  promptCacheKeyHash?: string | null;
  previousResponseIdPresent: boolean;
  configuration?: Array<{ name: string; valueHash: string }>;
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
      cache_trace?: ProviderCacheTrace | null;
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
      cache_trace?: ProviderCacheTrace | null;
      body?: unknown;
    }
  | {
      type: "provider_first_token_received";
      request_id: string;
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
  "idle" | "queued" | "running" | "needs_attention" | "archived";

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
