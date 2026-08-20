/* eslint-disable */
// Generated from the Rust DTO schema. Run `pnpm contracts:generate`; do not edit.

export type AgentEventPayload =
  | {
      snapshot: ThreadContextSnapshot;
      type: "thread_context_snapshot";
      [k: string]: unknown;
    }
  | {
      snapshot: TurnContextSnapshot;
      type: "turn_context_snapshot";
      [k: string]: unknown;
    }
  | {
      type: "turn_started";
      user_message_id: string;
      [k: string]: unknown;
    }
  | {
      context_hash: string;
      dynamic_tail_hash?: string | null;
      items?: ModelContextItem[];
      purpose?: ModelCallPurpose & string;
      request_id?: string;
      round: number;
      stable_prefix_hash?: string | null;
      token_breakdown?: TokenEstimateBreakdown | null;
      token_estimate: number;
      type: "model_context_built";
      [k: string]: unknown;
    }
  | {
      request?: unknown;
      request_id?: string;
      round: number;
      type: "model_request";
      [k: string]: unknown;
    }
  | {
      adapter: string;
      attempt: number;
      body?: unknown;
      endpoint: string;
      method: string;
      request_id: string;
      round: number;
      type: "provider_request_sent";
      [k: string]: unknown;
    }
  | {
      attempt: number;
      body?: unknown;
      reason: string;
      request_id: string;
      retry_index?: number | null;
      retry_kind?: ProviderRetryKind & string;
      retry_limit?: number | null;
      round: number;
      type: "provider_request_retried";
      [k: string]: unknown;
    }
  | {
      request_id: string;
      type: "provider_first_token_received";
      [k: string]: unknown;
    }
  | {
      attempt: number;
      body?: unknown;
      request_id: string;
      response_id?: string | null;
      round: number;
      status?: number | null;
      type: "provider_response_received";
      [k: string]: unknown;
    }
  | {
      text: string;
      type: "model_delta";
      [k: string]: unknown;
    }
  | {
      text: string;
      type: "reasoning_delta";
      [k: string]: unknown;
    }
  | {
      call: ToolCall;
      type: "tool_call_started";
      [k: string]: unknown;
    }
  | {
      result: ToolResult;
      type: "tool_call_finished";
      [k: string]: unknown;
    }
  | {
      form: WorkForm;
      type: "work_form_updated";
      [k: string]: unknown;
    }
  | {
      snapshot: GoalSnapshot;
      type: "goal_updated";
      [k: string]: unknown;
    }
  | {
      request: UserInputRequest;
      type: "user_input_requested";
      [k: string]: unknown;
    }
  | {
      message: Message;
      type: "assistant_message";
      [k: string]: unknown;
    }
  | {
      path: string;
      summary: string;
      type: "file_changed";
      [k: string]: unknown;
    }
  | {
      change_set: TurnChangeSet;
      type: "turn_changes_recorded";
      [k: string]: unknown;
    }
  | {
      files_changed: number;
      target_turn_id: string;
      type: "turn_undo_completed";
      [k: string]: unknown;
    }
  | {
      action: string;
      approval_id: string;
      reason: string;
      type: "approval_requested";
      [k: string]: unknown;
    }
  | {
      action: unknown;
      review_id: string;
      target_item_id: string;
      type: "automatic_approval_review_started";
      [k: string]: unknown;
    }
  | {
      action: unknown;
      attempts?: number;
      decision_source?: GuardianDecisionSource & string;
      failure_kind?: GuardianReviewFailureKind | null;
      rationale: string;
      review_id: string;
      risk_level?: GuardianRiskLevel | null;
      status: GuardianReviewStatus;
      target_item_id: string;
      tool_rounds?: number;
      type: "automatic_approval_review_completed";
      usage?: ModelUsage;
      user_authorization?: GuardianUserAuthorization | null;
      [k: string]: unknown;
    }
  | {
      message: string;
      type: "auto_review_interruption_warning";
      [k: string]: unknown;
    }
  | {
      details?: ContextCompactionDetails | null;
      summary: ContextSummary;
      type: "context_compacted";
      [k: string]: unknown;
    }
  | {
      projection: ContextProjection;
      type: "context_projection_built";
      [k: string]: unknown;
    }
  | {
      compaction_item_count: number;
      model: string;
      provider_id: string;
      response_item_count: number;
      state_kind: string;
      type: "provider_context_state_updated";
      [k: string]: unknown;
    }
  | {
      model?: string | null;
      provider_id?: string | null;
      reason: string;
      type: "provider_context_state_invalidated";
      [k: string]: unknown;
    }
  | {
      message: string;
      stage: string;
      type: "context_warning";
      [k: string]: unknown;
    }
  | {
      cache_write_tokens?: number | null;
      cached_input_tokens?: number | null;
      input_breakdown?: TokenEstimateBreakdown | null;
      input_tokens: number;
      local_input_estimate?: number | null;
      output_tokens: number;
      purpose?: ModelCallPurpose & string;
      reasoning_tokens?: number | null;
      request_id?: string | null;
      round?: number | null;
      total_tokens: number;
      type: "token_usage";
      [k: string]: unknown;
    }
  | {
      summary: string;
      type: "turn_finished";
      [k: string]: unknown;
    }
  | {
      approval_id: string;
      reason: string;
      type: "turn_suspended";
      [k: string]: unknown;
    }
  | {
      action: string;
      reason: string;
      type: "browser_handoff_required";
      url?: string | null;
      [k: string]: unknown;
    }
  | {
      prior_turn_id: string;
      type: "browser_handoff_completed";
      [k: string]: unknown;
    }
  | {
      request_id: string;
      type: "turn_awaiting_input";
      [k: string]: unknown;
    }
  | {
      reason: string;
      type: "turn_cancelled";
      [k: string]: unknown;
    }
  | {
      message: string;
      type: "error";
      [k: string]: unknown;
    };
/**
 * Semantic authority carried by a context item before a provider adapter maps it to the provider's supported message roles.
 *
 * This is harness metadata. Providers never receive this enum as a prompt tag. `Data` means the item is asserted as context or state, but does not mint new instructions merely because its transport role may be `developer`.
 */
export type ContextAuthority = "system" | "developer" | "user" | "assistant" | "tool" | "data";
export type ContextCacheScope = "stable" | "thread" | "turn" | "round" | "none";
/**
 * A typed unit of model input or tool output.
 *
 * Text is the portable baseline for providers and tools, while the other variants retain information that would otherwise be flattened into a prompt string. `Image` stores the original bytes so provider adapters can choose their native multimodal representation at the last possible point.
 */
export type ModelContentPart =
  | {
      text: string;
      type: "text";
      [k: string]: unknown;
    }
  | {
      type: "json";
      value: unknown;
      [k: string]: unknown;
    }
  | {
      content_type: string;
      data: number[];
      type: "image";
      [k: string]: unknown;
    }
  | {
      content_type?: string | null;
      name?: string | null;
      type: "resource";
      uri: string;
      [k: string]: unknown;
    };
export type ContextItemKind =
  | (
      | "base_instructions"
      | "developer_instructions"
      | "repository_instructions"
      | "environment"
      | "world_state"
      | "capability_catalog"
      | "skill_instructions"
      | "summary"
      | "checkpoint"
      | "conversation"
      | "user"
      | "tool_call"
      | "tool_result"
    )
  | "skill";
/**
 * Semantic lifetime used by the harness for auditing and invalidation.
 *
 * This is intentionally independent from `ContextCacheScope`: a Skill chosen for one Turn may still be placed in a reusable provider prefix, while a durable checkpoint belongs to an Epoch even when transported in that same prefix. Providers never receive this enum as a prompt tag.
 */
export type ContextLifecycle = "build" | "thread" | "epoch" | "turn" | "round";
export type ContextRole = "system" | "developer" | "user" | "assistant" | "tool";
export type ContextSensitivity = "public" | "workspace" | "sensitive";
export type ModelCallPurpose =
  "agent_round" | "context_compaction" | "guardian_review" | "title_generation" | "other";
export type ProviderRetryKind = "network" | "state_recovery";
export type CompletionDisposition = "blocking" | "advisory";
export type WorkItemStatus =
  "pending" | "in_progress" | "completed" | "deferred" | "blocked" | "cancelled";
export type WorkScope =
  | {
      id: string;
      kind: "turn";
      [k: string]: unknown;
    }
  | {
      id: string;
      kind: "goal";
      [k: string]: unknown;
    };
export type WorkFormStatus = "active" | "completed" | "blocked" | "paused" | "cancelled";
export type MessagePart =
  | {
      text: string;
      type: "text";
      [k: string]: unknown;
    }
  | {
      contentType: string;
      data: number[];
      id?: string | null;
      name?: string | null;
      type: "image";
      [k: string]: unknown;
    }
  | {
      image_id: string;
      type: "image_ref";
      [k: string]: unknown;
    }
  | {
      call: ToolCall;
      type: "tool_call";
      [k: string]: unknown;
    }
  | {
      result: ToolResult;
      type: "tool_result";
      [k: string]: unknown;
    }
  | {
      path: string;
      type: "file_ref";
      [k: string]: unknown;
    }
  | {
      source: ContextSourceRef;
      type: "source_ref";
      [k: string]: unknown;
    }
  | {
      skill: SkillRef;
      type: "skill_ref";
      [k: string]: unknown;
    }
  | {
      collaboration_mode: CollaborationMode;
      goal_id?: string | null;
      /**
       * Optional per-Turn retrieval backend selected by the conversation UI. This is execution metadata only; model history projection omits the TurnContext part itself.
       */
      library_provider?: string | null;
      type: "turn_context";
      [k: string]: unknown;
    }
  | {
      message: string;
      type: "error";
      [k: string]: unknown;
    };
export type ContextSourceKind = "text" | "image" | "document";
export type CollaborationMode = "default" | "plan" | "goal";
export type MessageRole = "system" | "user" | "assistant" | "tool";
export type TurnFileChangeKind = "added" | "modified" | "deleted" | "renamed";
export type TurnChangeSetStatus = "capturing" | "ready" | "empty" | "failed";
export type GuardianDecisionSource = "guardian" | "runtime";
export type GuardianReviewFailureKind = "reviewer_unavailable" | "invalid_reviewer_response";
export type GuardianRiskLevel = "low" | "medium" | "high" | "critical";
export type GuardianReviewStatus =
  | "in_progress"
  | "approved"
  | "needs_user_approval"
  | "denied_by_policy"
  | "reviewer_unavailable"
  | "invalid_reviewer_response"
  | "aborted";
export type GuardianUserAuthorization = "unknown" | "low" | "medium" | "high";
export type ContextCheckpointMode =
  "legacy_text" | "manual" | "structured_local" | "native_provider";
export type ContextFactStatus = "active" | "resolved" | "superseded";
export type DesktopStreamKind = "agent_event" | "agent_activity" | "terminal_event";

export interface AgentEventEnvelopeV1 {
  apiVersion: number;
  data: AgentEvent;
  kind: DesktopStreamKind;
  seq: number;
  [k: string]: unknown;
}
export interface AgentEvent {
  createdAt: string;
  id: string;
  payload: AgentEventPayload;
  seq: number;
  threadId: string;
  turnId?: string | null;
  [k: string]: unknown;
}
export interface ThreadContextSnapshot {
  capturedAt: string;
  contextHash: string;
  cwd: string;
  experienceMode?: string;
  instructions?: InstructionSnapshotRef[];
  model: string;
  permissionMode: string;
  /**
   * Concrete protocol contract used by this thread snapshot.
   */
  providerAdapter?: string;
  providerId: string;
  /**
   * Deprecated connection preset retained for event compatibility.
   */
  providerKind: string;
  sandboxMode: string;
  toolCatalogHash: string;
  workspaceRoot: string;
  worldStateHash: string;
  [k: string]: unknown;
}
export interface InstructionSnapshotRef {
  bytes: number;
  contentHash: string;
  path: string;
  scope: string;
  truncated: boolean;
  [k: string]: unknown;
}
export interface TurnContextSnapshot {
  capturedAt: string;
  changedKeys?: string[];
  contextHash: string;
  cwd: string;
  experienceMode?: string;
  instructions?: InstructionSnapshotRef[];
  permissionMode: string;
  previousWorldStateHash?: string | null;
  sandboxMode: string;
  workspaceRoots?: string[];
  worldState: WorldStateSnapshot;
  worldStateHash: string;
  [k: string]: unknown;
}
export interface WorldStateSnapshot {
  currentDate: string;
  cwd: string;
  gitBranch?: string | null;
  gitStatus?: string | null;
  mcpToolCount: number;
  metadata?: {
    [k: string]: unknown;
  };
  platform: string;
  skillCatalog?: WorldStateSkill[];
  timezone: string;
  toolCatalogHash: string;
  toolCount: number;
  workspaceRoots?: string[];
  [k: string]: unknown;
}
export interface WorldStateSkill {
  contentHash: string;
  description: string;
  id: string;
  name: string;
  scope: string;
  [k: string]: unknown;
}
export interface ModelContextItem {
  authority: ContextAuthority;
  cacheScope: ContextCacheScope;
  content: ModelContentPart[];
  contentHash: string;
  id: string;
  kind: ContextItemKind;
  lifecycle: ContextLifecycle;
  metadata?: unknown;
  /**
   * Provider transport role. Semantic authority is tracked separately so contextual data can be carried in a developer-shaped envelope without being classified as a developer-authored rule.
   */
  role: ContextRole;
  sensitivity: ContextSensitivity;
  source: string;
  tokenEstimate: number;
  [k: string]: unknown;
}
/**
 * Provider-neutral estimate of the logical input carried by one model request.
 *
 * These values are intentionally kept separate from provider-reported usage: they explain which harness modules built the request, while provider usage is the billing/accounting authority after the request completes.
 */
export interface TokenEstimateBreakdown {
  baseInstructions: number;
  checkpoints: number;
  conversation: number;
  currentUser: number;
  /**
   * Names/descriptions visible before a deferred tool is selected.
   */
  deferredToolCatalog?: number;
  /**
   * Hierarchical local attribution. Omitted when replaying legacy logs.
   */
  details?: TokenEstimateDetail[];
  developerInstructions: number;
  /**
   * Full input schemas sent directly in the request's tool surface.
   */
  directToolSchemas?: number;
  /**
   * Schemas appended by a provider Tool Search continuation.
   */
  loadedToolSchemas?: number;
  other: number;
  /**
   * A structured-output schema is a request field, not a tool definition.
   */
  outputSchema?: number;
  providerState: number;
  repositoryInstructions: number;
  runtimeContext: number;
  skillInstructions: number;
  summaries: number;
  toolCalls: number;
  toolResults: number;
  /**
   * Sum of the three tool-surface buckets above. Counted in `total` once.
   */
  toolSchemas: number;
  total: number;
  /**
   * Provider-native assistant message items replayed inside the active turn. These are neither durable conversation history nor opaque continuation state, so they remain a separate, mutually exclusive bucket.
   */
  turnAssistantState?: number;
  [k: string]: unknown;
}
/**
 * One node in the local attribution tree for a logical model request.
 *
 * Providers report request-level usage totals. These nodes preserve the harness-owned structure used to assemble that request without presenting the individual values as provider-billed facts.
 */
export interface TokenEstimateDetail {
  children?: TokenEstimateDetail[];
  id: string;
  label: string;
  tokens: number;
  [k: string]: unknown;
}
export interface ToolCall {
  id: string;
  input: unknown;
  name: string;
  [k: string]: unknown;
}
export interface ToolResult {
  callId: string;
  content?: ModelContentPart[];
  /**
   * Tool-specific metadata is also the forward-compatible place for context and artifact hints, such as truncated/originalBytes/maxResults.
   */
  metadata: {
    [k: string]: unknown;
  };
  /**
   * Legacy text output. New tools should populate `content`; consumers can use `content_or_legacy_text` while callers migrate.
   */
  output: string;
  [k: string]: unknown;
}
export interface WorkForm {
  acceptance?: string[];
  changeReason?: string | null;
  constraints?: string[];
  createdAt: string;
  id: string;
  items?: WorkItem[];
  objective: string;
  revision: number;
  scope: WorkScope;
  status: WorkFormStatus;
  threadId: string;
  updatedAt: string;
  [k: string]: unknown;
}
export interface WorkItem {
  acceptance?: string[];
  completionDisposition?: CompletionDisposition & string;
  dependsOn?: string[];
  evidenceRefs?: string[];
  id: string;
  note?: string | null;
  status: WorkItemStatus;
  title: string;
  [k: string]: unknown;
}
export interface GoalSnapshot {
  goal: GoalRecord;
  workForm: WorkForm;
  [k: string]: unknown;
}
export interface GoalRecord {
  createdAt: string;
  id: string;
  objective: string;
  threadId: string;
  timeUsedSeconds: number;
  tokenBudget?: number | null;
  tokensUsed: number;
  updatedAt: string;
  version: number;
  [k: string]: unknown;
}
export interface UserInputRequest {
  questions: UserInputQuestion[];
  requestId: string;
  [k: string]: unknown;
}
export interface UserInputQuestion {
  allowCustom?: boolean;
  header: string;
  id: string;
  options: UserInputOption[];
  question: string;
  [k: string]: unknown;
}
export interface UserInputOption {
  description: string;
  id: string;
  label: string;
  recommended?: boolean;
  [k: string]: unknown;
}
export interface Message {
  createdAt: string;
  id: string;
  parts: MessagePart[];
  role: MessageRole;
  threadId: string;
  [k: string]: unknown;
}
export interface ContextSourceRef {
  bytes: number;
  contentType: string;
  id: string;
  kind: ContextSourceKind;
  name: string;
  path: string;
  truncated: boolean;
  [k: string]: unknown;
}
export interface SkillRef {
  description: string;
  id: string;
  name: string;
  path: string;
  truncated: boolean;
  [k: string]: unknown;
}
export interface TurnChangeSet {
  additions: number;
  afterTree?: string | null;
  beforeTree?: string | null;
  createdAt: string;
  deletions: number;
  error?: string | null;
  files: TurnFileChange[];
  finalizedAt?: string | null;
  repoRoot?: string | null;
  revertedAt?: string | null;
  status: TurnChangeSetStatus;
  threadId: string;
  turnId: string;
  workspacePrefix?: string | null;
  workspaceRoot: string;
  [k: string]: unknown;
}
export interface TurnFileChange {
  additions?: number | null;
  afterMode?: string | null;
  afterOid?: string | null;
  beforeMode?: string | null;
  beforeOid?: string | null;
  binary: boolean;
  deletions?: number | null;
  kind: TurnFileChangeKind;
  newPath?: string | null;
  oldPath?: string | null;
  [k: string]: unknown;
}
export interface ModelUsage {
  cacheWriteTokens?: number | null;
  cachedInputTokens?: number | null;
  inputTokens: number;
  outputTokens: number;
  reasoningTokens?: number | null;
  totalTokens: number;
  [k: string]: unknown;
}
export interface ContextCompactionDetails {
  checkpointId?: string | null;
  coverage: ContextCheckpointCoverage;
  metrics?: ContextCompactionMetrics | null;
  mode: ContextCheckpointMode;
  providerStateCheckpointId?: string | null;
  [k: string]: unknown;
}
export interface ContextCheckpointCoverage {
  throughMessageCount: number;
  throughSeq: number;
  [k: string]: unknown;
}
export interface ContextCompactionMetrics {
  activeConstraintRetentionPercent: number;
  cacheHitPercent?: number;
  cachedInputTokens?: number;
  checkpointTokens: number;
  factRetentionPercent: number;
  /**
   * Exact logical agent request before local compaction.
   */
  inputTokens: number;
  latencyMs: number;
  /**
   * Exact logical agent request after the checkpoint starts a new epoch.
   */
  postCompactionTokens?: number;
  /**
   * Provider-reported usage for the auxiliary compaction model call.
   */
  providerInputTokens?: number;
  providerOutputTokens?: number;
  /**
   * Percentage of the original request remaining after compaction.
   */
  remainingPercent?: number;
  source: string;
  tokenReductionPercent: number;
  tokensRemoved?: number;
  [k: string]: unknown;
}
export interface ContextSummary {
  checkpoint?: ContextCheckpoint | null;
  coveredThroughSeq: number;
  createdAt: string;
  id: string;
  messageCount: number;
  metadata: unknown;
  summary: string;
  threadId: string;
  tokenEstimate?: number | null;
  [k: string]: unknown;
}
export interface ContextCheckpoint {
  artifacts?: ContextCheckpointArtifact[];
  commandsAndValidation?: ContextCheckpointCommand[];
  coverage: ContextCheckpointCoverage;
  createdAt: string;
  decisions?: ContextCheckpointFact[];
  goal: string;
  id: string;
  mode: ContextCheckpointMode;
  nextSteps?: ContextCheckpointStep[];
  openIssues?: ContextCheckpointFact[];
  pendingInteractions?: ContextCheckpointInteraction[];
  phases?: ContextCheckpointPhase[];
  previousCheckpointId?: string | null;
  providerCompatibilityHash?: string | null;
  schemaVersion: number;
  threadId: string;
  userConstraints?: ContextCheckpointFact[];
  workspaceState?: ContextCheckpointWorkspace;
  [k: string]: unknown;
}
export interface ContextCheckpointArtifact {
  id?: string | null;
  kind: string;
  path?: string | null;
  sourceSeqs?: number[];
  summary: string;
  [k: string]: unknown;
}
export interface ContextCheckpointCommand {
  command: string;
  outcome: string;
  sourceSeqs?: number[];
  summary: string;
  [k: string]: unknown;
}
export interface ContextCheckpointFact {
  confidence?: number | null;
  id: string;
  sourceSeqs?: number[];
  status?: ContextFactStatus & string;
  text: string;
  [k: string]: unknown;
}
export interface ContextCheckpointStep {
  id: string;
  sourceSeqs?: number[];
  status: string;
  text: string;
  [k: string]: unknown;
}
export interface ContextCheckpointInteraction {
  kind: string;
  sourceSeqs?: number[];
  summary: string;
  [k: string]: unknown;
}
/**
 * One evidence-backed stage in the task's durable history.
 *
 * `from_seq`/`through_seq` are the authoritative ordering keys. Timestamps are canonicalized from those events by the checkpoint service rather than trusted from model output.
 */
export interface ContextCheckpointPhase {
  endedAt?: string | null;
  fromSeq: number;
  id: string;
  metrics?: ContextCheckpointMetric[];
  objective: string;
  outcome?: string | null;
  problem?: string | null;
  remainingRisks?: string[];
  resolution?: string | null;
  rootCause?: string | null;
  sourceSeqs?: number[];
  startedAt?: string | null;
  status: string;
  throughSeq: number;
  title: string;
  [k: string]: unknown;
}
export interface ContextCheckpointMetric {
  name: string;
  sourceSeqs?: number[];
  unit?: string | null;
  value: string;
  [k: string]: unknown;
}
export interface ContextCheckpointWorkspace {
  branch?: string | null;
  filesChanged?: ContextCheckpointFile[];
  gitStatus?: string | null;
  [k: string]: unknown;
}
export interface ContextCheckpointFile {
  path: string;
  sourceSeqs?: number[];
  status: string;
  summary: string;
  [k: string]: unknown;
}
export interface ContextProjection {
  checkpointId?: string | null;
  checkpointMode?: string | null;
  checkpointTokens: number;
  coveredMessageCount: number;
  coveredThroughSeq: number;
  nativeCompactionItemCount: number;
  nativeCompactionSupported: boolean;
  providerItemCount: number;
  providerStateAvailable: boolean;
  providerStateKind?: string | null;
  recentTailTokens: number;
  unsummarizedEventCount: number;
  unsummarizedMessageCount: number;
  [k: string]: unknown;
}
