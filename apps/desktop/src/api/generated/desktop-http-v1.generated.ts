/* eslint-disable */
// Generated from the Rust DTO schema. Run `pnpm contracts:generate`; do not edit.

export type WorkspaceDiffScope = "staged" | "unstaged";
export type HumanTaskActionV1 =
  "approve" | "reject" | "retry" | "resume" | "submit" | "reconnect" | "acknowledge" | "cancel";
export type HumanTaskSourceKindV1 = "flow_run" | "delivery_receipt";
export type HumanTaskStatusV1 = "pending" | "completed" | "cancelled";
export type HumanTaskTypeV1 =
  | "approval"
  | "input_request"
  | "output_review"
  | "recovery"
  | "reconnect"
  | "data_correction"
  | "reconciliation"
  | "manual";
export type BrowserRuntimeRoute = "managed" | "chrome";
export type ExperienceMode = "work" | "code" | "flow";
export type ResourceKind = "file" | "network" | "database";
export type DataClassification = "public" | "internal" | "confidential" | "restricted";
export type AgentInstanceStatusV1 = "active" | "suspended" | "completed" | "revoked";
export type WorkflowInterruptKindV1 =
  "approval" | "input_request" | "external_action" | "effect_reconciliation" | "resume_retry";
export type FlowTranscriptEntryKindV1 =
  "input" | "tool_call" | "tool_result" | "output" | "approval" | "error";
export type FlowResumeSignalV1 =
  | {
      approval_id?: string | null;
      approved: boolean;
      kind: "approval";
      [k: string]: unknown;
    }
  | {
      kind: "user_input";
      request_id: string;
      response: UserInputResponse;
      [k: string]: unknown;
    }
  | {
      kind: "external_action";
      observation: string;
      [k: string]: unknown;
    };
export type WorkflowCheckpointStatusV1 = "running" | "committed" | "failed" | "cancelled";
/**
 * Frozen authority for external Connection operations.
 *
 * This discriminator is independent from collaboration snapshots because the same immutable authority is also owned by persisted Flow runs. In particular, `structured { operations: [] }` is an explicit empty grant and must never fall back to mutable legacy thread MCP bindings.
 */
export type RuntimeConnectionAuthorityV1 =
  | {
      mode: "deny_all";
      [k: string]: unknown;
    }
  | {
      mode: "legacy_mcp";
      [k: string]: unknown;
    }
  | {
      mode: "structured";
      operations?: ExecutionConnectionOperationV1[];
      [k: string]: unknown;
    };
export type AgentRiskClassV1 = "low" | "medium" | "high" | "critical";
export type LoopExhaustionActionV1 = "require_human" | "return_partial" | "fail";
export type GraphNodeKindV1 =
  "agent" | "skill" | "tool" | "condition" | "validator" | "approval" | "join" | "loop" | "output";
export type WorkflowOutputSpecV1 =
  | {
      kind: "inbox";
      [k: string]: unknown;
    }
  | {
      credential_ref?: string | null;
      endpoint: string;
      kind: "webhook";
      [k: string]: unknown;
    }
  | {
      kind: "connection_operation";
      operation: ExecutionConnectionOperationV1;
      [k: string]: unknown;
    }
  | {
      assigned_to?: string | null;
      description: string;
      kind: "human_task";
      title: string;
      [k: string]: unknown;
    };
export type WorkflowOutputReviewPolicyV1 = "explicit_nodes_only" | "always_review_output";
export type WorkflowTriggerSpecV1 =
  | {
      kind: "manual";
      [k: string]: unknown;
    }
  | {
      kind: "webhook";
      token_ref: string;
      trigger_id: string;
      [k: string]: unknown;
    }
  | {
      interval_seconds: number;
      kind: "schedule";
      next_fire_at: string;
      trigger_id: string;
      [k: string]: unknown;
    }
  | {
      event_type: string;
      kind: "event_subscription";
      source: string;
      trigger_id: string;
      [k: string]: unknown;
    };
export type FlowNodeRunStatusV1 =
  | "running"
  | "waiting_approval"
  | "waiting_human"
  | "resuming"
  | "succeeded"
  | "failed"
  | "cancelled";
export type FlowRunStatusV1 =
  | "queued"
  | "running"
  | "pause_requested"
  | "paused"
  | "waiting_approval"
  | "waiting_human"
  | "resuming"
  | "succeeded"
  | "failed"
  | "cancel_requested"
  | "cancelled";
export type ContextFactStatus = "active" | "resolved" | "superseded";
export type ContextCheckpointMode =
  "legacy_text" | "manual" | "structured_local" | "native_provider";
export type CapabilityChangeKindV1 = "added" | "removed" | "expanded" | "reduced";
export type AgentTemplateStatusV1 = "draft" | "published";
export type ConnectionAuthVerificationV1 =
  "not_required" | "unverified" | "legacy_unverified" | "verified";
export type ConnectionOwnerTypeV1 = "personal" | "org_shared" | "service_account";
export type ConnectionRuntimeBindingV1 = {
  kind: "mcp_server";
  serverId: string;
  [k: string]: unknown;
};
export type ConnectionStatusV1 =
  "configured" | "ready" | "degraded" | "reauth_required" | "disabled";
export type FlowValidationSeverityV1 = "error" | "warning";
export type FlowSourceV1 =
  | {
      description: string;
      kind: "natural_language";
      [k: string]: unknown;
    }
  | {
      kind: "run_trace";
      run_id: string;
      trace_hash: string;
      [k: string]: unknown;
    };
export type FlowDraftStatusV1 =
  "drafting" | "reviewing" | "validating" | "ready_to_publish" | "published";
export type FlowTrialStatusV1 = "passed" | "failed";
export type IntegrationAuthSchemeV1 = "none" | "api_key" | "oauth2" | "external";
export type CapabilityDiscoveryKindV1 = "mcp_tools_list" | "static";
export type IntegrationKindV1 = "mcp" | "oauth_api" | "database" | "local_app";
export type McpLifecycleStatus = "not_started" | "starting" | "ready" | "error" | "disabled";
export type WorkflowDeploymentStatusV1 = "active" | "disabled";
export type WorkflowIngressPolicyV1 = "immediate" | "require_review";
export type WorkflowReleaseStatusV1 = "active" | "disabled";
export type WorkflowTriggerInvocationStatusV1 = "accepted" | "started" | "failed";
export type GitWorkflowActionKind =
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
export type LocalGitV1Output =
  | {
      type: "status";
      value: LocalGitStatus;
      [k: string]: unknown;
    }
  | {
      type: "branches";
      value: GitBranchInfo[];
      [k: string]: unknown;
    }
  | {
      type: "remotes";
      value: LocalGitRemote[];
      [k: string]: unknown;
    }
  | {
      type: "worktrees";
      value: LocalGitWorktree[];
      [k: string]: unknown;
    }
  | {
      type: "compare";
      value: number[];
      [k: string]: unknown;
    }
  | {
      type: "mutation";
      value: number[];
      [k: string]: unknown;
    };
export type ConnectionAccessIssueSeverity = "error" | "warning";
export type ConnectionCapabilityKindV1 = "tool";
export type ConnectionAccessMode = "none" | "legacy" | "structured";
export type ArtifactStorage =
  | {
      content: string;
      type: "inline";
      [k: string]: unknown;
    }
  | {
      path: string;
      type: "path";
      [k: string]: unknown;
    };
export type CapabilityDiscoverySupportV1 = "supported" | "unsupported";
export type ConnectionCapabilitySourceV1 = "mcp_tools_list" | "static";
export type SandboxMode = "read-only" | "workspace-write" | "danger-full-access";
export type ContributionKind =
  | "skill"
  | "mcp_server"
  | "native_tool"
  | "previewer"
  | "context_loader"
  | "agent_profile"
  | "scm_connector"
  | "app";
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
export type LibraryProviderStatus = SagConnectionView | GraphRagConnectionView;
export type PluginControlScopeType = "global" | "workspace" | "thread";
export type PluginRuntimeHealthStatus = "unknown" | "ready" | "degraded" | "error" | "stopped";
export type ContributionOrigin = "codex_compatible" | "open_topia";
export type PluginPermissionKind = "filesystem" | "network" | "secret" | "desktop";
export type PluginScope = "workspace" | "user" | "codex";
export type PluginSource = "workspace" | "user" | "codex" | "bundled";
export type BundledPluginTrust = "standard" | "official" | "privileged" | "trusted_driver";
export type PluginPermissionGrantStatus = "granted" | "revoked";
/**
 * Wire protocol selected for one concrete endpoint/model pair. Connections are credentials and routing; adapters are protocol codecs, so one relay may legitimately select different adapters for different models.
 */
export type ProviderAdapterKind =
  "open_ai_chat" | "open_ai_responses" | "anthropic_messages" | "codex_app_server" | "mock";
export type ProviderAuthKind = "bearer" | "x_api_key" | "codex_session" | "none";
/**
 * Legacy provider preset identity. New runtime code resolves transport, authentication, and adapter independently; this enum remains serialized for one compatibility window so older desktop builds can still read settings.
 */
export type ProviderKind =
  ("mock" | "openai_compatible" | "openai_responses") | "codex_app_server" | "anthropic";
export type ProviderTransportKind = "http" | "codex_app_server" | "mock";
export type PreviewKind = "text" | "image" | "pdf" | "document" | "spreadsheet" | "unsupported";
export type PreviewSource = "workspace" | "local" | "artifact" | "attachment";
export type ExecutionEnvironmentKind = "local" | "docker" | "remote";
export type SandboxLifecycle = "ready" | "starting" | "stopped" | "error";
export type OsSandboxMode = "disabled" | "best_effort" | "enforce";
export type NetworkPolicy = "inherit" | "allow" | "deny";
export type OsSandboxPlatform = "linux" | "macos" | "windows" | "unsupported";
export type ScmConnectorCapability =
  "change_requests" | "issues" | "automation" | "reviews" | "releases" | "repository_identity";
export type ScmHostMatcher =
  | {
      type: "exact";
      value: string;
      [k: string]: unknown;
    }
  | {
      type: "suffix";
      value: string;
      [k: string]: unknown;
    }
  | {
      type: "any";
      [k: string]: unknown;
    };
export type ScmPathMatcher =
  | {
      type: "exact";
      value: string;
      [k: string]: unknown;
    }
  | {
      type: "prefix";
      value: string;
      [k: string]: unknown;
    }
  | {
      type: "any";
      [k: string]: unknown;
    };
export type ScmConnectorSelection =
  | {
      status: "unmatched";
      [k: string]: unknown;
    }
  | {
      account_binding_id?: string | null;
      candidate: ScmConnectorCandidate;
      source: ScmSelectionSource;
      status: "selected";
      [k: string]: unknown;
    }
  | {
      binding_issue?: ScmBindingIssue | null;
      candidates: ScmConnectorCandidate[];
      status: "conflict";
      [k: string]: unknown;
    };
export type ScmSelectionSource = "best_match" | "remote_binding";
export type ScmBindingIssue =
  "wrong_workspace_or_remote" | "connector_unavailable" | "connector_not_best_match";
export type AgentAutonomy = "guided" | "balanced" | "proactive";
export type MultiAgentMode = "off" | "explicit" | "adaptive";
export type AgentPersonality = "focused" | "professional" | "warm";
export type ProgressUpdateMode = "milestones" | "balanced" | "frequent";
export type PermissionMode =
  ("chat" | "read_only" | "auto" | "approve") | "full_access" | "unrestricted";
/**
 * Deterministic instruction lowering selected during capability negotiation. The adapter reads this value while encoding; it never probes or changes it.
 */
export type ProviderInstructionEncoding =
  "native_roles" | "fold_developer_into_system" | "portable_chat_envelope";
export type ProviderFeatureSupport = "supported" | "unsupported" | "unknown";
/**
 * Structural request envelope used to control model reasoning. Variant names describe wire behavior rather than vendors or model families. Capability negotiation owns this choice; request codecs never infer it from a model id.
 */
export type ProviderReasoningProtocol =
  | "omit"
  | "chat_reasoning_effort"
  | "chat_thinking_reasoning_effort"
  | "chat_thinking_high_max_no_tool_choice"
  | "responses_reasoning";
export type OpenAiProtocol = "chat_completions" | "responses";
export type PromptCachePolicy = "explicit30m" | "legacy_in_memory" | "legacy24h";
export type SandboxEnforcement = "disabled" | "best-effort" | "enforce";
export type WindowsSandboxBackend = "auto" | "dedicated_user" | "unelevated";
export type SheetKind = "worksheet" | "dialog_sheet" | "macro_sheet" | "chart_sheet" | "vba";
export type SheetVisibility = "visible" | "hidden" | "very_hidden";
export type SpreadsheetCellValue =
  | {
      type: "empty";
      [k: string]: unknown;
    }
  | {
      type: "string";
      value: string;
      [k: string]: unknown;
    }
  | {
      type: "integer";
      value: number;
      [k: string]: unknown;
    }
  | {
      type: "number";
      value: number;
      [k: string]: unknown;
    }
  | {
      type: "boolean";
      value: boolean;
      [k: string]: unknown;
    }
  | {
      type: "date_time";
      value: ExcelDateTimeValue;
      [k: string]: unknown;
    }
  | {
      type: "date_time_iso";
      value: string;
      [k: string]: unknown;
    }
  | {
      type: "duration_iso";
      value: string;
      [k: string]: unknown;
    }
  | {
      type: "error";
      value: string;
      [k: string]: unknown;
    };
export type CapabilityUnavailableReason =
  | ("disabled" | "host_trust_required" | "conflict")
  | {
      missing_host_capabilities: string[];
    }
  | {
      missing_permissions: PluginPermission[];
    };
export type TurnFileChangeKind = "added" | "modified" | "deleted" | "renamed";
export type TurnChangeSetStatus = "capturing" | "ready" | "empty" | "failed";
export type TurnStatus =
  | "running"
  | "waiting_approval"
  | "waiting_user_input"
  | "waiting_user_action"
  | "cancelling"
  | "succeeded"
  | "failed"
  | "cancelled"
  | "interrupted";
export type WindowsSandboxSetupState = "unavailable" | "not_configured" | "ready" | "degraded";
export type ManagedOfficeRuntimeStatus =
  "not_required" | "pending" | "downloading" | "ready" | "disabled" | "failed";
export type OfficeRuntimeSource = "configured" | "packaged" | "managed" | "legacy_override";
export type ManagedPowerShellStatus =
  "not_required" | "pending" | "downloading" | "ready" | "disabled" | "failed";
export type ShellDialect = "power_shell7" | "windows_power_shell51" | "posix_sh";
export type ShellRuntimeSource = "configured" | "managed" | "standard_install" | "path" | "system";
export type MediaHandlerOperation = "preview" | "load_context";
export type MediaHandlerRuntime =
  | {
      server: string;
      tool: string;
      type: "mcp_v1";
      [k: string]: unknown;
    }
  | {
      adapter: string;
      type: "builtin";
      [k: string]: unknown;
    };
export type ActivityEventDetails =
  | {
      round: number;
      type: "model_round";
      [k: string]: unknown;
    }
  | {
      input_preview: unknown;
      invocation_id: string;
      tool_name: string;
      type: "tool_call_started";
      [k: string]: unknown;
    }
  | {
      invocation_id: string;
      tool_name?: string | null;
      type: "tool_call_finished";
      [k: string]: unknown;
    }
  | {
      reason: string;
      type: "waiting";
      [k: string]: unknown;
    }
  | {
      message: string;
      type: "error";
      [k: string]: unknown;
    };
export type ToolResultKind = "text" | "json" | "resource" | "binary" | "mixed";
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
export type AgentAvailability = "idle" | "queued" | "running" | "needs_attention" | "archived";
export type ArtifactStorageMetadata =
  | {
      type: "inline";
      [k: string]: unknown;
    }
  | {
      path: string;
      type: "path";
      [k: string]: unknown;
    };
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
      cache_trace?: ProviderCacheTrace | null;
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
      cache_trace?: ProviderCacheTrace | null;
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
export type ProviderRetryKind = "network" | "state_recovery";
export type MessagePart =
  | {
      text: string;
      type: "text";
      [k: string]: unknown;
    }
  | {
      text: string;
      type: "proposed_plan";
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
export type ApprovalStatus = "pending" | "approved" | "denied";
export type UserInputStatus = "pending" | "answered";
export type ProviderDriverTrust = "built_in" | "signed";
export type SkillScope = "workspace" | "user";
export type TerminalEventKind =
  "started" | "stdout" | "stderr" | "finished" | "cancelled" | "error";
export type WorkflowDeliveryStatusV1 =
  "pending" | "delivered" | "failed" | "waiting_human" | "cancelled";
export type WorkspaceEntryKind = "file" | "directory" | "symlink" | "other";
export type TurnUndoConflictKind =
  | "unavailable"
  | "already_reverted"
  | "workspace_changed"
  | "merge_conflict"
  | "binary_changed"
  | "path_conflict"
  | "unsupported_file_type"
  | "too_large";
/**
 * Content that can later be passed straight into a multimodal tool-result/message contract.
 */
export type BrowserContent =
  | {
      text: string;
      truncated: boolean;
      type: "text";
      [k: string]: unknown;
    }
  | {
      type: "json";
      value: unknown;
      [k: string]: unknown;
    }
  | {
      bytes: number[];
      mime_type: string;
      type: "image";
      [k: string]: unknown;
    }
  | {
      bytes: number;
      mime_type?: string | null;
      path: string;
      type: "file";
      [k: string]: unknown;
    };
export type MediaHandlerSelection =
  | {
      status: "none";
      [k: string]: unknown;
    }
  | {
      handler: MediaHandlerDescriptor;
      status: "selected";
      [k: string]: unknown;
    }
  | {
      contribution_ids: string[];
      status: "conflict";
      [k: string]: unknown;
    };
export type AppViewSessionStatus = "ready" | "stopped";

export interface DesktopHttpResponsesV1 {
  applyWorkspaceDiffHunk: WorkspaceDiffActionResponse;
  archiveAgentTemplate: DeleteResponse;
  assignHumanTask: HumanTaskV1;
  bindBrowserRuntime: BrowserRuntimeStatus;
  bindThreadAgentInstance: AgentInstanceV1;
  callMcpTool: McpCallResult;
  cancelCodexLogin: DeleteResponse;
  cancelFlowRun: FlowRunV1;
  cancelTerminalCommand: TerminalCancelResponse;
  cancelTurn: TurnCancelResult;
  claimHumanTask: HumanTaskV1;
  closeComputerSession: DeleteResponse;
  closePreview: ResourceReleaseResponse;
  closeTerminalSession: TerminalSessionResponse;
  compactContext: ContextSummary;
  createAgentInstance: CreateAgentInstanceResponse;
  createAgentTemplateVersion: AgentTemplateVersionView;
  createConnection: ConnectionV1;
  createFlowDraft: FlowDraftView;
  createIntegrationDefinition: IntegrationDefinitionV1;
  createMcpServer: McpServerView;
  createProject: Project;
  createThread: Thread;
  createWorkflowDeployment: WorkflowDeploymentV1;
  createWorkflowEvaluation: WorkflowEvaluationV1;
  createWorkflowRelease: WorkflowReleaseV1;
  decideApproval: ApprovalDecisionResponse;
  deleteAgentTemplateVersion: DeleteResponse;
  deleteMcpServer: DeleteResponse;
  deleteProject: DeleteResponse;
  deleteThread: DeleteResponse;
  disableWorkflowDeployment: WorkflowDeploymentV1;
  disableWorkflowRelease: WorkflowReleaseV1;
  dispatchWorkflowEvent: WorkflowInvocationResult[];
  ensureTerminalSession: TerminalSessionResponse;
  executeLocalGit: LocalGitV1Response;
  generateThreadTitle: GenerateThreadTitleResponse;
  getAgentTemplateConnectionAccess: AgentTemplateConnectionAccessView;
  getArtifact: Artifact;
  getBoundThreadAgentInstance?: AgentInstanceV1 | null;
  getBrowserRuntime: BrowserRuntimeStatus;
  getCodexAccount: CodexAccountStatus;
  getConnection: ConnectionV1;
  getConnectionCapabilityRevision: ConnectionCapabilityRevisionV1;
  getContextStatus: ContextStatusResponse;
  getContributionHosts: ContributionHostSnapshot;
  getFlowRun: FlowRunV1;
  getGoal?: GoalSnapshot | null;
  getHumanTask: HumanTaskV1;
  getIntegrationDefinition: IntegrationDefinitionV1;
  getLibraryProviderStatus: LibraryProviderStatus;
  getPluginContributions: PluginContributionRecord[];
  getPluginDetail: PluginDetailResponse;
  getPluginHealth: PluginRuntimeHealthRecord[];
  getPluginPermissions: PluginPermissionsResponse;
  getPluginSettings: PluginSettingsResponse;
  getProviderHealth: ProviderHealth[];
  getResourceMetadata: PreviewDescriptor;
  getSandbox: SandboxDescriptor;
  getScmRemoteConnector: ScmRemoteConnectorResponse;
  getSettings: AppSettings;
  getSpreadsheetPreview: PreviewWorkbook;
  getSpreadsheetPreviewRange: PreviewRange;
  getTerminalSession?: TerminalSessionResponse | null;
  getThreadCapabilities: ThreadCapabilitiesResponse;
  getThreadFlowDraft?: FlowDraftView | null;
  getTurnChanges: TurnChangeSet;
  getTurnFileDiffPreview: TurnFileDiffPreview;
  getTurnStatus?: TurnRecord | null;
  getWindowsSandboxSetup: WindowsSandboxSetupStatus;
  getWorkflowDeployment: WorkflowDeploymentV1;
  getWorkflowEvaluationSummary: WorkflowEvaluationSummary;
  getWorkflowRelease: WorkflowReleaseV1;
  getWorkspaceDiff: WorkspaceDiff;
  health: HealthResponse;
  ingestSagText: LibraryIngestionResponseView;
  installPlugin: PluginView;
  interruptAgent: DeleteResponse;
  invokeMediaHandler: MediaHandlerInvocationResponse;
  invokeWorkflowRelease: WorkflowInvocationResult;
  listActivityStatuses: TurnRecord[];
  listAgentInstances: AgentInstanceV1[];
  listAgentTemplates: AgentTemplateVersionView[];
  listAgents: AgentListItem[];
  listAllFlowRuns: FlowRunV1[];
  listArtifacts: ArtifactMetadata[];
  listComputerWindows: WindowTarget[];
  listConnectionCapabilityRevisions: ConnectionCapabilityRevisionV1[];
  listConnections: ConnectionV1[];
  listConversationEvents: AgentEvent[];
  listEvents: AgentEvent[];
  listFlowDrafts: FlowDraftView[];
  listFlowRuns: FlowRunV1[];
  listHumanTasks: HumanTaskV1[];
  listIntegrationDefinitions: IntegrationDefinitionV1[];
  listLibraryProviders: LibraryProviderDescriptor[];
  listLibrarySources: LibrarySourcePageView;
  listMcpServers: McpServerView[];
  listMcpTools: McpToolDescriptor[];
  listMessages: Message[];
  listPendingApprovals: Approval[];
  listPendingUserInput: UserInputRecord[];
  listPlugins: PluginView[];
  listProjects: Project[];
  listProviderDrivers: ProviderDriverDescriptor[];
  listSkills: SkillDescriptor[];
  listTerminalHistory: TerminalEvent[];
  listThreadAgentInstances: AgentInstanceV1[];
  listThreadMcpServers: ThreadMcpServerView[];
  listThreads: Thread[];
  listWorkflowDeliveryReceipts: WorkflowDeliveryReceiptV1[];
  listWorkflowDeployments: WorkflowDeploymentV1[];
  listWorkflowEvaluations: WorkflowEvaluationV1[];
  listWorkflowReleases: WorkflowReleaseV1[];
  listWorkflowTriggerInvocations: WorkflowTriggerInvocationV1[];
  listWorkspaceTree: WorkspaceTree;
  logoutCodexAccount: DeleteResponse;
  observeComputerWindow: ComputerObservation;
  pauseFlowRun: FlowRunV1;
  postPluginAppMessage: AppViewMessage;
  previewTurnUndo: TurnUndoPreview;
  promoteWorkflowRelease: WorkflowReleaseV1;
  publishAgentTemplateVersion: AgentTemplateVersionView;
  publishFlowDraft: FlowDefinitionV1;
  readWorkspaceFile: WorkspaceFilePreview;
  refreshConnectionCapabilities: RefreshConnectionCapabilitiesResponse;
  removeWindowsSandbox: WindowsSandboxSetupStatus;
  resizeTerminalSession: TerminalSessionResponse;
  resolveHumanTask: ResolveHumanTaskResponse;
  resolvePreview: PreviewDescriptor;
  respondToUserInput: UserInputResponseAccepted;
  restartMcpServer: McpServerStatus;
  resumeExternalAction: ExternalActionResumeResponse;
  resumeFlowRun: FlowRunV1;
  retryManagedOfficeRuntime: OfficeRuntimeStatus;
  retryManagedPowerShell: ShellRuntimeStatus;
  retryWorkflowDelivery: WorkflowDeliveryReceiptV1;
  revertWorkspaceFile: WorkspaceDiffActionResponse;
  rollbackWorkflowRelease: WorkflowReleaseV1;
  runBrowserCommand: BrowserOutput;
  runGitWorkflow: GitWorkflowResponse;
  searchFlows: FlowDefinitionV1[];
  searchLibrary: LibrarySearchResponseView;
  selectContextLoader: MediaHandlerSelection;
  selectPreviewHandler: MediaHandlerSelection;
  sendMessage: Message;
  setPluginActivation: PluginActivationResponse;
  setPluginPermission: PluginPermissionGrantRecord;
  setScmRemoteConnector: ScmRemoteConnectorResponse;
  setThreadMcpServer: ThreadMcpServer;
  setThreadModel: Thread;
  setThreadPlugin: PluginView;
  setWorkflowReleaseCanary: WorkflowReleaseV1;
  setupWindowsSandbox: WindowsSandboxSetupStatus;
  simulateFlowDraft: FlowTrialV1;
  startCodexLogin: CodexLoginStart;
  startDeployedWorkflowRun: FlowRunV1;
  startFlowRun: FlowRunV1;
  startFlowTestRun: FlowRunV1;
  startPendingWorkflowInvocation: WorkflowInvocationResult;
  startPluginAppSession: AppViewSessionResponse;
  startTerminalCommand: TerminalStartResponse;
  stopPluginAppSession: AppViewSession;
  syncProviderModels: ProviderModelSyncResult;
  testConnection: TestConnectionResponse;
  testProviderConnection: ProviderHealthCheck;
  undoTurnChanges: TurnUndoResult;
  uninstallPlugin: DeleteResponse;
  updateAgentInstance: AgentInstanceV1;
  updateConnection: ConnectionV1;
  updateFlowDraft: FlowDraftView;
  updateGoal: GoalSnapshot;
  updateIntegrationDefinition: IntegrationDefinitionV1;
  updateMcpServer: McpServerView;
  updatePluginSettings: PluginSettingsResponse;
  updateProject: Project;
  updateSettings: AppSettings;
  updateThread: Thread;
  uploadLibrarySource: LibraryIngestionResponseView;
  validateFlowDraft: FlowDraftView;
  writeResourceContent: PreviewDescriptor;
  writeTerminalSession: TerminalSessionResponse;
}
export interface WorkspaceDiffActionResponse {
  diff: WorkspaceDiff;
  path: string;
  [k: string]: unknown;
}
export interface WorkspaceDiff {
  branch?: string | null;
  command: string;
  diff: string;
  files: ChangedFile[];
  hunks: WorkspaceDiffHunk[];
  remoteUrl?: string | null;
  stagedDiff: string;
  stagedTruncated: boolean;
  truncated: boolean;
  unstagedDiff: string;
  unstagedTruncated: boolean;
  [k: string]: unknown;
}
export interface ChangedFile {
  isRenamed: boolean;
  isUntracked: boolean;
  originalPath?: string | null;
  path: string;
  stagedStatus: string;
  status: string;
  unstagedStatus: string;
  [k: string]: unknown;
}
export interface WorkspaceDiffHunk {
  header: string;
  lines: string[];
  newLines?: number | null;
  newStart?: number | null;
  oldLines?: number | null;
  oldStart?: number | null;
  patch: string;
  path: string;
  raw: string;
  scope: WorkspaceDiffScope;
  [k: string]: unknown;
}
export interface DeleteResponse {
  deleted: boolean;
  [k: string]: unknown;
}
export interface HumanTaskV1 {
  actionSchema?: unknown;
  allowedActions: HumanTaskActionV1[];
  assignedTo?: string | null;
  checkpointId?: string | null;
  claimedAt?: string | null;
  claimedBy?: string | null;
  continuationId?: string | null;
  createdAt: string;
  description: string;
  dueAt?: string | null;
  id: string;
  payload?: {
    [k: string]: unknown;
  };
  resolution?: HumanTaskResolutionV1 | null;
  resolvedAt?: string | null;
  revision: number;
  schemaVersion: number;
  sourceId: string;
  sourceKind: HumanTaskSourceKindV1;
  sourceNodeId?: string | null;
  sourceNodeRunId?: string | null;
  status: HumanTaskStatusV1;
  taskType: HumanTaskTypeV1;
  threadId: string;
  title: string;
  updatedAt: string;
  [k: string]: unknown;
}
export interface HumanTaskResolutionV1 {
  action: HumanTaskActionV1;
  commandId?: string | null;
  idempotencyKey?: string | null;
  note?: string | null;
  resolvedAt: string;
  resolvedBy: string;
  response?: unknown;
  [k: string]: unknown;
}
export interface BrowserRuntimeStatus {
  chromeAvailable: boolean;
  route: BrowserRuntimeRoute;
  [k: string]: unknown;
}
export interface AgentInstanceV1 {
  createdAt: string;
  delegationDepth: number;
  executionContext: EnterpriseExecutionContextV1;
  id: string;
  parentInstanceId?: string | null;
  schemaVersion: number;
  state: unknown;
  stateRevision: number;
  status: AgentInstanceStatusV1;
  templateId: string;
  templateVersion: number;
  threadId: string;
  updatedAt: string;
  [k: string]: unknown;
}
export interface EnterpriseExecutionContextV1 {
  agentId: string;
  capabilities: CapabilityProjection;
  connectionBindings?: ConnectionBindingV1[];
  connectionOperations?: ExecutionConnectionOperationV1[];
  delegationChain: string[];
  knowledgeBinding?: SagKnowledgeBindingV1 | null;
  mode: ExperienceMode;
  modelPolicy: AgentModelPolicyV1;
  parentAgentId?: string | null;
  resourceGrants: ExecutionResourceGrantV1[];
  schemaVersion: number;
  templateId: string;
  templateVersion: number;
  threadId: string;
  [k: string]: unknown;
}
/**
 * A deterministic, fail-closed view of the capabilities available to one Agent execution. `allow_all_*` is explicit so a missing field never means unrestricted access when an ExecutionContext is deserialized.
 */
export interface CapabilityProjection {
  allowAllMcpServers?: boolean;
  allowAllPlugins?: boolean;
  allowAllSkills?: boolean;
  allowAllTools?: boolean;
  allowAllWorkspaceRoots?: boolean;
  mcpServers?: string[];
  plugins?: string[];
  skills?: string[];
  tools?: string[];
  workspaceRoots?: string[];
  [k: string]: unknown;
}
/**
 * Pins one account-level Connection and the immutable capability revision that was reviewed when an Agent template was published.
 */
export interface ConnectionBindingV1 {
  capabilityRevision: number;
  connectionId: string;
  operationGrants?: OperationGrantV1[];
}
/**
 * Grants one stable provider operation. The operation ID is deliberately not a runtime tool name; the server resolves it through the pinned capability revision before execution.
 */
export interface OperationGrantV1 {
  operationId: string;
}
/**
 * Credential-free operation route frozen into an Agent instance. The runtime must still revalidate the live Connection and fingerprint immediately before crossing the external-call boundary.
 */
export interface ExecutionConnectionOperationV1 {
  capabilityRevision: number;
  connectionId: string;
  mcpServerId: string;
  modelToolName: string;
  operationId: string;
  pinnedOperationFingerprint: string;
  providerToolName: string;
}
/**
 * Immutable, server-enforced SAG scope available to an Agent. Namespaces are intentionally absent from the model-facing tool schema so the model cannot widen the template's knowledge boundary at invocation time.
 */
export interface SagKnowledgeBindingV1 {
  namespaces: string[];
  [k: string]: unknown;
}
export interface AgentModelPolicyV1 {
  allowAllModels?: boolean;
  allowedModels?: AgentModelBindingV1[];
  [k: string]: unknown;
}
export interface AgentModelBindingV1 {
  modelId: string;
  providerId: string;
  [k: string]: unknown;
}
export interface ExecutionResourceGrantV1 {
  bindingId: string;
  canRead: boolean;
  canWrite: boolean;
  kind: ResourceKind;
  maxDataClassification: DataClassification;
  /**
   * A workspace-relative root, network origin, or logical database route.
   */
  resource: string;
  [k: string]: unknown;
}
export interface McpCallResult {
  content: unknown[];
  isError: boolean;
  output: string;
  publicName: string;
  raw: unknown;
  serverId: string;
  structuredContent?: unknown;
  toolName: string;
  [k: string]: unknown;
}
export interface FlowRunV1 {
  activeCheckpoint?: WorkflowCheckpointV1 | null;
  activeHumanTaskId?: string | null;
  budget: FlowBudgetV1;
  checkpointHistory?: WorkflowCheckpointSummaryV1[];
  completedAt?: string | null;
  /**
   * Immutable operation-level authority captured when the Run starts. `None` is reserved for persisted runs created before this field existed; those are restored through explicit legacy-projection inference.
   */
  connectionAuthority?: RuntimeConnectionAuthorityV1 | null;
  createdAt: string;
  definitionContentHash: string;
  definitionId: string;
  deploymentId?: string | null;
  deploymentSnapshot?: DeploymentSnapshotV1 | null;
  effectiveCapabilities: CapabilityProjection;
  error?: string | null;
  flowId: string;
  flowVersion: number;
  graph: GraphDefinitionV1;
  id: string;
  input: unknown;
  loopCounts?: {
    [k: string]: number;
  };
  nodeExecutions: number;
  nodeOutputs?: {
    [k: string]: unknown;
  };
  nodeRuns?: FlowNodeRunV1[];
  output?: unknown;
  /**
   * Production deployments pause after terminal output until a HumanTask records the review decision. Trial/manual compatibility runs may opt out.
   */
  outputReviewRequired?: boolean;
  outputReviewed?: boolean;
  readyNodes?: string[];
  revision: number;
  schemaVersion: number;
  startedAt?: string | null;
  /**
   * Reducer-owned shared state. Node outputs remain the immutable routing source; state channels are applied only at a committed superstep.
   */
  state?: {
    [k: string]: unknown;
  };
  status: FlowRunStatusV1;
  superstep?: number;
  testDraftId?: string | null;
  testDraftRevision?: number | null;
  threadId: string;
  toolCalls: number;
  updatedAt: string;
  waitingNodeId?: string | null;
  [k: string]: unknown;
}
export interface WorkflowCheckpointV1 {
  completedAt?: string | null;
  createdAt: string;
  id: string;
  nodes: WorkflowSuperstepNodeV1[];
  pendingWrites?: WorkflowPendingWriteV1[];
  status: WorkflowCheckpointStatusV1;
  superstep: number;
  [k: string]: unknown;
}
export interface WorkflowSuperstepNodeV1 {
  attempt: number;
  input: unknown;
  nodeId: string;
  nodeRunId: string;
  [k: string]: unknown;
}
export interface WorkflowPendingWriteV1 {
  completedAt: string;
  error?: string | null;
  interrupt?: WorkflowInterruptRequestV1 | null;
  nodeId: string;
  nodeRunId: string;
  result?: FlowNodeExecutionResultV1 | null;
  /**
   * The command is retained after execution as an immutable audit record. It only applies when its interrupt id/revision matches `interrupt`.
   */
  resumeCommand?: FlowResumeCommandV1 | null;
  [k: string]: unknown;
}
export interface WorkflowInterruptRequestV1 {
  checkpointId: string;
  continuation: AgentContinuationEnvelopeV1;
  createdAt: string;
  description: string;
  id: string;
  kind: WorkflowInterruptKindV1;
  nodeId: string;
  nodeRunId: string;
  payload?: {
    [k: string]: unknown;
  };
  revision: number;
  schemaVersion: number;
  superstep: number;
  title: string;
  toolCalls: number;
  transcript?: FlowTranscriptEntryV1[];
  [k: string]: unknown;
}
export interface AgentContinuationEnvelopeV1 {
  contentHash: string;
  id: string;
  payload: unknown;
  schemaVersion: number;
  [k: string]: unknown;
}
export interface FlowTranscriptEntryV1 {
  callId?: string | null;
  content: unknown;
  createdAt: string;
  id: string;
  isError?: boolean;
  kind: FlowTranscriptEntryKindV1;
  title: string;
  toolName?: string | null;
  [k: string]: unknown;
}
export interface FlowNodeExecutionResultV1 {
  output: unknown;
  toolCalls: number;
  transcript?: FlowTranscriptEntryV1[];
  [k: string]: unknown;
}
export interface FlowResumeCommandV1 {
  expectedInterruptRevision: number;
  id: string;
  idempotencyKey: string;
  interruptId: string;
  issuedAt: string;
  issuedBy: string;
  note?: string | null;
  schemaVersion: number;
  signal: FlowResumeSignalV1;
  [k: string]: unknown;
}
export interface UserInputResponse {
  answers: UserInputAnswer[];
  /**
   * Dismiss the decision boundary and end the waiting Turn without another model invocation. A later user message starts a new Turn normally.
   */
  cancelled?: boolean;
  /**
   * Skip the optional decision and let the same Turn continue with a reasonable assumption.
   */
  skipped?: boolean;
  [k: string]: unknown;
}
export interface UserInputAnswer {
  customText?: string | null;
  optionId?: string | null;
  questionId: string;
  [k: string]: unknown;
}
export interface FlowBudgetV1 {
  maxDurationSeconds: number;
  maxLoopIterations: number;
  maxNodeExecutions: number;
  maxToolCalls: number;
  [k: string]: unknown;
}
export interface WorkflowCheckpointSummaryV1 {
  completedAt: string;
  createdAt: string;
  id: string;
  nodeIds: string[];
  pendingWriteCount: number;
  status: WorkflowCheckpointStatusV1;
  superstep: number;
  [k: string]: unknown;
}
export interface DeploymentSnapshotV1 {
  compiledWorkflow: CompiledWorkflowV1;
  contentHash: string;
  createdAt: string;
  createdBy: string;
  id: string;
  output: WorkflowOutputSpecV1;
  outputReviewPolicy?: WorkflowOutputReviewPolicyV1 & string;
  schemaVersion: number;
  trigger: WorkflowTriggerSpecV1;
  [k: string]: unknown;
}
export interface CompiledWorkflowV1 {
  agentSpecs: {
    [k: string]: WorkflowAgentSpecV1;
  };
  budget: FlowBudgetV1;
  contentHash: string;
  definitionContentHash: string;
  definitionId: string;
  flowId: string;
  flowVersion: number;
  graph: GraphDefinitionV1;
  harnessCapabilities: CapabilityProjection;
  harnessConnectionAuthority: RuntimeConnectionAuthorityV1;
  inputSchema: unknown;
  outputSchema: unknown;
  rootCapabilities: CapabilityProjection;
  schemaVersion: number;
  [k: string]: unknown;
}
export interface WorkflowAgentSpecV1 {
  capabilities: CapabilityProjection;
  connectionAuthority: RuntimeConnectionAuthorityV1;
  connectionBindings: ConnectionBindingV1[];
  instructions: string;
  knowledgeBinding?: SagKnowledgeBindingV1 | null;
  modelPolicy: AgentModelPolicyV1;
  name: string;
  nodeId: string;
  outputSchema: unknown;
  owner: string;
  resourceGrants: ExecutionResourceGrantV1[];
  riskClass: AgentRiskClassV1;
  stateSchema: unknown;
  templateContentHash: string;
  templateId: string;
  templateVersion: number;
  [k: string]: unknown;
}
export interface GraphDefinitionV1 {
  edges: GraphEdgeV1[];
  entryNodeId: string;
  nodes: GraphNodeV1[];
  schemaVersion: number;
  [k: string]: unknown;
}
export interface GraphEdgeV1 {
  allowedFields?: string[];
  condition?: string | null;
  dataClassification?: DataClassification & string;
  from: string;
  loopPolicy?: GraphLoopPolicyV1 | null;
  onError?: string | null;
  to: string;
  [k: string]: unknown;
}
export interface GraphLoopPolicyV1 {
  continueCondition: string;
  maxIterations: number;
  onExhausted: LoopExhaustionActionV1;
  [k: string]: unknown;
}
export interface GraphNodeV1 {
  config?: {
    [k: string]: unknown;
  };
  id: string;
  inputSchema?: {
    [k: string]: unknown;
  };
  kind: GraphNodeKindV1;
  label: string;
  outputSchema?: {
    [k: string]: unknown;
  };
  [k: string]: unknown;
}
export interface FlowNodeRunV1 {
  attempt: number;
  completedAt?: string | null;
  error?: string | null;
  id: string;
  input: unknown;
  nodeId: string;
  output?: unknown;
  startedAt: string;
  status: FlowNodeRunStatusV1;
  toolCalls: number;
  transcript?: FlowTranscriptEntryV1[];
  [k: string]: unknown;
}
export interface TerminalCancelResponse {
  cancelled: boolean;
  commandId?: string | null;
  message: string;
  [k: string]: unknown;
}
export interface TurnCancelResult {
  cancelled: boolean;
  message: string;
  turnId?: string | null;
  [k: string]: unknown;
}
export interface ResourceReleaseResponse {
  released: boolean;
  [k: string]: unknown;
}
export interface TerminalSessionResponse {
  cwd: string;
  processId?: number | null;
  sessionId: string;
  shell: string;
  startedAt: string;
  status: string;
  threadId: string;
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
export interface ContextCheckpointCoverage {
  throughMessageCount: number;
  throughSeq: number;
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
export interface CreateAgentInstanceResponse {
  bound: boolean;
  instance: AgentInstanceV1;
  [k: string]: unknown;
}
export interface AgentTemplateVersionView {
  diff: AgentTemplateDiffV1;
  template: AgentTemplateVersionV1;
  [k: string]: unknown;
}
export interface AgentTemplateDiffV1 {
  changes: CapabilityChangeV1[];
  fromVersion?: number | null;
  toVersion: number;
  widensCapabilities: boolean;
  [k: string]: unknown;
}
export interface CapabilityChangeV1 {
  kind: CapabilityChangeKindV1;
  scope: string;
  value: string;
  [k: string]: unknown;
}
export interface AgentTemplateVersionV1 {
  contentHash: string;
  createdAt: string;
  name: string;
  owner: string;
  publishedAt?: string | null;
  publishedBy?: string | null;
  schemaVersion: number;
  spec: AgentTemplateSpecV1;
  status: AgentTemplateStatusV1;
  templateId: string;
  version: number;
  [k: string]: unknown;
}
export interface AgentTemplateSpecV1 {
  allowAllDelegates: boolean;
  budget: AgentBudgetV1;
  capabilities: CapabilityProjection;
  connectionBindings?: ConnectionBindingV1[];
  delegateTemplateIds: string[];
  description: string;
  instructions: string;
  knowledgeBinding?: SagKnowledgeBindingV1 | null;
  modelPolicy: AgentModelPolicyV1;
  outputSchema: unknown;
  resourceGrants: ExecutionResourceGrantV1[];
  riskClass: AgentRiskClassV1;
  stateSchema: unknown;
  [k: string]: unknown;
}
export interface AgentBudgetV1 {
  maxDurationSeconds: number;
  maxToolCalls: number;
  maxTurns: number;
  [k: string]: unknown;
}
export interface ConnectionV1 {
  activeCapabilityRevision?: number | null;
  authContext: ConnectionAuthContextV1;
  createdAt: string;
  enabled: boolean;
  environment: string;
  id: string;
  integrationDefinitionId: string;
  lastError?: string | null;
  lastTestedAt?: string | null;
  name: string;
  ownerType: ConnectionOwnerTypeV1;
  revision: number;
  runtimeBinding: ConnectionRuntimeBindingV1;
  schemaVersion: number;
  status: ConnectionStatusV1;
  updatedAt: string;
  [k: string]: unknown;
}
export interface ConnectionAuthContextV1 {
  account?: ConnectionAccountV1;
  /**
   * Opaque reference resolved by a credential provider. Never a token or password.
   */
  credentialRef?: string | null;
  expiresAt?: string | null;
  grantedScopes?: string[];
  verification: ConnectionAuthVerificationV1;
  [k: string]: unknown;
}
export interface ConnectionAccountV1 {
  displayName?: string | null;
  externalAccountId?: string | null;
  tenantId?: string | null;
  tenantName?: string | null;
  workspaceId?: string | null;
  workspaceName?: string | null;
  [k: string]: unknown;
}
export interface FlowDraftView {
  draft: FlowDraftV1;
  testRuns: FlowRunV1[];
  trials: FlowTrialV1[];
  [k: string]: unknown;
}
export interface FlowDraftV1 {
  contentHash: string;
  createdAt: string;
  effectiveCapabilities: CapabilityProjection;
  id: string;
  lastValidation?: FlowValidationReportV1 | null;
  revision: number;
  schemaVersion: number;
  spec: FlowSpecV1;
  status: FlowDraftStatusV1;
  threadId: string;
  updatedAt: string;
  [k: string]: unknown;
}
export interface FlowValidationReportV1 {
  issues: FlowValidationIssueV1[];
  valid: boolean;
  validatedAt: string;
  [k: string]: unknown;
}
export interface FlowValidationIssueV1 {
  code: string;
  edgeIndex?: number | null;
  message: string;
  nodeId?: string | null;
  remediation: string;
  severity: FlowValidationSeverityV1;
  [k: string]: unknown;
}
export interface FlowSpecV1 {
  budget?: FlowBudgetV1;
  categories?: string[];
  description: string;
  flowId: string;
  graph: GraphDefinitionV1;
  inputSchema?: {
    [k: string]: unknown;
  };
  name: string;
  outputSchema?: {
    [k: string]: unknown;
  };
  owner: string;
  pendingDecisions?: string[];
  requestedCapabilities?: CapabilityProjection;
  riskClass: AgentRiskClassV1;
  source: FlowSourceV1;
  [k: string]: unknown;
}
export interface FlowTrialV1 {
  createdAt: string;
  draftId: string;
  draftRevision: number;
  id: string;
  input: unknown;
  report: FlowValidationReportV1;
  schemaVersion: number;
  status: FlowTrialStatusV1;
  steps: FlowSimulationStepV1[];
  [k: string]: unknown;
}
export interface FlowSimulationStepV1 {
  boundedBy?: number | null;
  harnessTarget: string;
  nodeId: string;
  order: number;
  [k: string]: unknown;
}
export interface IntegrationDefinitionV1 {
  authScheme: IntegrationAuthSchemeV1;
  capabilityDiscovery: CapabilityDiscoveryKindV1;
  createdAt: string;
  description?: string | null;
  enabled: boolean;
  id: string;
  key: string;
  kind: IntegrationKindV1;
  name: string;
  revision: number;
  schemaVersion: number;
  updatedAt: string;
  [k: string]: unknown;
}
export interface McpServerView {
  server: McpServerConfig;
  status: McpServerStatus;
  [k: string]: unknown;
}
export interface McpServerConfig {
  args: string[];
  command: string;
  createdAt: string;
  cwd?: string | null;
  enabled: boolean;
  envKeys: string[];
  name: string;
  pluginId?: string | null;
  pluginServerName?: string | null;
  serverId: string;
  timeoutMs: number;
  updatedAt: string;
  [k: string]: unknown;
}
export interface McpServerStatus {
  message: string;
  name: string;
  serverId: string;
  status: McpLifecycleStatus;
  toolsCount: number;
  updatedAt: string;
  [k: string]: unknown;
}
export interface Project {
  createdAt: string;
  id: string;
  name: string;
  pinned: boolean;
  sortOrder: number;
  updatedAt: string;
  workspaceRoot?: string | null;
  [k: string]: unknown;
}
export interface Thread {
  archivedAt?: string | null;
  createdAt: string;
  experienceMode?: ExperienceMode & string;
  id: string;
  /**
   * Model chosen for this conversation. Pinned at creation so a catalog refresh never swaps the model mid-thread; `None` means "use the active connection's default", which keeps pre-existing threads working.
   */
  modelSelection?: ThreadModelSelection | null;
  projectId?: string | null;
  title: string;
  updatedAt: string;
  workspaceRoot: string;
  [k: string]: unknown;
}
/**
 * A concrete model to run a thread with. The connection supplies the endpoint and credentials; this only narrows which model and how hard it thinks.
 */
export interface ThreadModelSelection {
  connectionId: string;
  modelId: string;
  reasoningEffort?: string | null;
  [k: string]: unknown;
}
export interface WorkflowDeploymentV1 {
  createdAt: string;
  createdBy: string;
  environment: string;
  id: string;
  name: string;
  revision: number;
  schemaVersion: number;
  snapshot: DeploymentSnapshotV1;
  status: WorkflowDeploymentStatusV1;
  updatedAt: string;
  [k: string]: unknown;
}
export interface WorkflowEvaluationV1 {
  createdAt: string;
  deploymentId: string;
  evaluator: string;
  id: string;
  labels?: string[];
  note?: string | null;
  passed: boolean;
  runId: string;
  schemaVersion: number;
  score: number;
  [k: string]: unknown;
}
export interface WorkflowReleaseV1 {
  canaryDeploymentId?: string | null;
  canaryPercent: number;
  createdAt: string;
  createdBy: string;
  environment: string;
  id: string;
  ingressPolicy?: WorkflowIngressPolicyV1 & string;
  previousPrimaryDeploymentId?: string | null;
  primaryDeploymentId: string;
  releaseKey: string;
  revision: number;
  schemaVersion: number;
  status: WorkflowReleaseStatusV1;
  threadId: string;
  trigger: WorkflowTriggerSpecV1;
  updatedAt: string;
  [k: string]: unknown;
}
export interface ApprovalDecisionResponse {
  accepted: boolean;
  executed: boolean;
  [k: string]: unknown;
}
export interface WorkflowInvocationResult {
  invocation: WorkflowTriggerInvocationV1;
  reused: boolean;
  run?: FlowRunV1 | null;
  [k: string]: unknown;
}
export interface WorkflowTriggerInvocationV1 {
  createdAt: string;
  deploymentId: string;
  error?: string | null;
  flowRunId?: string | null;
  id: string;
  idempotencyKey: string;
  input?: {
    [k: string]: unknown;
  };
  inputHash: string;
  releaseId: string;
  schemaVersion: number;
  status: WorkflowTriggerInvocationStatusV1;
  triggerId: string;
  updatedAt: string;
  [k: string]: unknown;
}
export interface LocalGitV1Response {
  apiVersion: string;
  command: LocalGitCommandSummary;
  operation: GitWorkflowActionKind;
  output: LocalGitV1Output;
  [k: string]: unknown;
}
export interface LocalGitCommandSummary {
  exitCode?: number | null;
  stderr: number[];
  success: boolean;
  truncated: boolean;
  [k: string]: unknown;
}
export interface LocalGitStatus {
  aheadBehind?: AheadBehind | null;
  branch?: string | null;
  porcelainV2: string;
  [k: string]: unknown;
}
export interface AheadBehind {
  ahead: number;
  behind: number;
  [k: string]: unknown;
}
export interface GitBranchInfo {
  current: boolean;
  fullRef: string;
  name: string;
  remote: boolean;
  symbolicTarget?: string | null;
  upstream?: string | null;
  [k: string]: unknown;
}
export interface LocalGitRemote {
  fetchUrls: NormalizedGitRemoteUrl[];
  name: string;
  pushUrls: NormalizedGitRemoteUrl[];
  [k: string]: unknown;
}
export interface NormalizedGitRemoteUrl {
  host?: string | null;
  normalized: string;
  port?: number | null;
  repositoryPath: string;
  scheme?: string | null;
  [k: string]: unknown;
}
export interface LocalGitWorktree {
  bare: boolean;
  branch?: string | null;
  detached: boolean;
  head?: string | null;
  lockReason?: string | null;
  locked: boolean;
  path: string;
  prunable: boolean;
  prunableReason?: string | null;
  [k: string]: unknown;
}
export interface GenerateThreadTitleResponse {
  thread: Thread;
  updated: boolean;
  [k: string]: unknown;
}
export interface AgentTemplateConnectionAccessView {
  bindings: ConnectionAccessBindingView[];
  effectiveMcpServerIds: string[];
  effectiveModelToolNames: string[];
  issues: ConnectionAccessIssueView[];
  mode: ConnectionAccessMode;
  valid: boolean;
  [k: string]: unknown;
}
export interface ConnectionAccessBindingView {
  capabilityRevision: number;
  connectionId: string;
  connectionName?: string | null;
  issues: ConnectionAccessIssueView[];
  operations: ConnectionAccessOperationView[];
  status?: ConnectionStatusV1 | null;
  valid: boolean;
  [k: string]: unknown;
}
export interface ConnectionAccessIssueView {
  code: string;
  connectionId?: string | null;
  message: string;
  operationId?: string | null;
  severity: ConnectionAccessIssueSeverity;
  [k: string]: unknown;
}
export interface ConnectionAccessOperationView {
  displayName?: string | null;
  kind?: ConnectionCapabilityKindV1 | null;
  modelToolName?: string | null;
  name?: string | null;
  operationId: string;
  permissionLabels: string[];
  providerPublicName?: string | null;
  [k: string]: unknown;
}
export interface Artifact {
  bytes: number;
  contentType: string;
  createdAt: string;
  id: string;
  kind: string;
  metadata: unknown;
  storage: ArtifactStorage;
  threadId: string;
  [k: string]: unknown;
}
/**
 * Public account controls for the local Codex App Server.
 *
 * The server owns the child process and keeps authentication inside Codex. OpenTopia only exposes non-secret account metadata and the documented login instructions to its UI.
 */
export interface CodexAccountStatus {
  accountId?: string | null;
  authMode?: string | null;
  authUrl?: string | null;
  email?: string | null;
  loggedIn: boolean;
  loginId?: string | null;
  loginPending: boolean;
  loginType?: string | null;
  planType?: string | null;
  rateLimits?: unknown;
  usage?: unknown;
  userCode?: string | null;
  verificationUrl?: string | null;
  [k: string]: unknown;
}
export interface ConnectionCapabilityRevisionV1 {
  capabilities: ConnectionCapabilityV1[];
  connectionId: string;
  contentHash: string;
  discoveredAt: string;
  discoveryCoverage: ConnectionCapabilityDiscoveryCoverageV1;
  id: string;
  revision: number;
  schemaVersion: number;
  source: ConnectionCapabilitySourceV1;
  [k: string]: unknown;
}
export interface ConnectionCapabilityV1 {
  /**
   * Only standard, public MCP behavior hints are projected here.
   */
  annotations: {
    [k: string]: unknown;
  };
  capabilityId: string;
  description?: string | null;
  displayName: string;
  inputSchema: unknown;
  kind: ConnectionCapabilityKindV1;
  name: string;
  permissionLabels: string[];
  providerMetadata: ConnectionCapabilityProviderMetadataV1;
  [k: string]: unknown;
}
export interface ConnectionCapabilityProviderMetadataV1 {
  publicName: string;
  serverId: string;
  toolName: string;
  [k: string]: unknown;
}
export interface ConnectionCapabilityDiscoveryCoverageV1 {
  prompts: CapabilityDiscoverySupportV1;
  resources: CapabilityDiscoverySupportV1;
  tools: CapabilityDiscoverySupportV1;
  [k: string]: unknown;
}
export interface ContextStatusResponse {
  budget: ContextBudget;
  latestSummary?: ContextSummary | null;
  projection: ContextProjection;
  usage: ContextUsageMetrics;
  [k: string]: unknown;
}
export interface ContextBudget {
  estimatedUsage: number;
  messageCount: number;
  totalTokens: number;
  usedTokens: number;
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
export interface ContextUsageMetrics {
  agentModelRequests: number;
  auxiliaryModelRequests: number;
  cacheWriteTokens: number;
  cachedInputTokens: number;
  checkpointTokens: number;
  compactionInputTokens: number;
  compactionLatencyMs: number;
  compactionModelRequests: number;
  compactions: number;
  estimateCalibrationFactor?: number | null;
  estimateErrorMean?: number | null;
  estimateErrorP95?: number | null;
  inputTokens: number;
  lastActiveConstraintRetentionPercent: number;
  lastFactRetentionPercent: number;
  localInputEstimate: number;
  modelRequests: number;
  nativeCompactions: number;
  outputTokens: number;
  providerFallbacks: number;
  providerResponses: number;
  providerUsageCoverage?: number | null;
  rawEstimateErrorMean?: number | null;
  rawEstimateErrorP95?: number | null;
  rawInputEstimate: number;
  reasoningTokens: number;
  totalTokens: number;
  uncachedInputTokens: number;
  warnings: number;
  [k: string]: unknown;
}
export interface ContributionHostSnapshot {
  agentProfiles: AgentProfile[];
  apps: AppViewDescriptor[];
  contextLoaders: MediaHandlerDescriptor[];
  issues: string[];
  previewers: MediaHandlerDescriptor[];
  [k: string]: unknown;
}
export interface AgentProfile {
  allowed_tools?: string[] | null;
  denied_tools?: string[];
  description: string;
  developer_instructions: string;
  model?: string | null;
  model_reasoning_effort?: string | null;
  name: string;
  nickname_candidates?: string[];
  sandbox_mode?: SandboxMode | null;
  source_contribution_id?: string | null;
  source_plugin_id?: string | null;
  [k: string]: unknown;
}
export interface AppViewDescriptor {
  allowedChannels: string[];
  contributionId: string;
  entry: string;
  localId: string;
  pluginId: string;
  sandbox: AppViewSandbox;
  title: string;
  [k: string]: unknown;
}
export interface AppViewSandbox {
  allowPopups: boolean;
  allowTopNavigation: boolean;
  allowedHostApis: string[];
  nodeIntegration: boolean;
  [k: string]: unknown;
}
export interface MediaHandlerDescriptor {
  contributionId: string;
  extensions: string[];
  kind: ContributionKind;
  localId: string;
  mediaTypes: string[];
  pluginId: string;
  priority: number;
  runtime: string;
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
export interface SagConnectionView {
  endpoint: string;
  provider: string;
  status: SagStatus;
  [k: string]: unknown;
}
export interface SagStatus {
  agent_loop_integration?: boolean;
  database?: string | null;
  deepseek_configured?: boolean;
  embedding_backend?: string | null;
  embedding_dimensions?: number | null;
  index_version?: string | null;
  integrity_check?: string | null;
  model_loaded?: boolean;
  prompt_injection?: boolean;
  stats?: {
    [k: string]: number;
  };
  status: string;
  [k: string]: unknown;
}
export interface GraphRagConnectionView {
  endpoint: string;
  provider: string;
  status: GraphRagStatus;
  [k: string]: unknown;
}
export interface GraphRagStatus {
  agent_loop_integration?: boolean;
  chunks?: number;
  documents?: number;
  embedding_backend?: string | null;
  embedding_dimensions?: number | null;
  graph_enabled?: boolean;
  index_version?: string | null;
  prompt_injection?: boolean;
  relations?: number;
  reranker_backend?: string | null;
  stats?: {
    [k: string]: number;
  };
  status: string;
  vector_backend?: string | null;
  [k: string]: unknown;
}
export interface PluginContributionRecord {
  contributionId: string;
  descriptor?: {
    [k: string]: unknown;
  };
  kind: string;
  localId: string;
  pluginId: string;
  updatedAt: string;
  [k: string]: unknown;
}
export interface PluginDetailResponse {
  activations: PluginActivationRecord[];
  contributions: PluginContributionRecord[];
  effectiveEnabled: boolean;
  health: PluginRuntimeHealthRecord[];
  manifest: PluginControlManifest;
  plugin: PluginDescriptor;
  [k: string]: unknown;
}
export interface PluginActivationRecord {
  enabled: boolean;
  pluginId: string;
  scope: PluginControlScope;
  updatedAt: string;
  [k: string]: unknown;
}
export interface PluginControlScope {
  scopeId?: string | null;
  scopeType: PluginControlScopeType;
  [k: string]: unknown;
}
export interface PluginRuntimeHealthRecord {
  contributionId: string;
  lastCheckedAt: string;
  lastError?: string | null;
  pluginId: string;
  restartCount: number;
  status: PluginRuntimeHealthStatus;
  [k: string]: unknown;
}
export interface PluginControlManifest {
  apiVersion?: string | null;
  configurationSchema?: unknown;
  contributions?: PluginContributionRecord[];
  hostCapabilities?: string[];
  permissionRequests?: PluginPermissionRequest[];
  requiredSecretSettingKeys?: string[];
  secretSettingKeys?: string[];
  [k: string]: unknown;
}
export interface PluginPermissionRequest {
  category: string;
  permission: string;
  value: string;
  [k: string]: unknown;
}
export interface PluginDescriptor {
  author: string;
  brandColor?: string | null;
  capabilities: string[];
  capabilityManifest: PluginCapabilityManifest;
  category: string;
  defaultEnabled: boolean;
  description: string;
  displayName: string;
  hasApps: boolean;
  id: string;
  issues: string[];
  longDescription: string;
  managed: boolean;
  manifestPath: string;
  mcpServerCount: number;
  name: string;
  nativeCapabilities: string[];
  path: string;
  scope: PluginScope;
  skillCount: number;
  skillRoot?: string | null;
  source: PluginSource;
  supportedMcpServerCount: number;
  trust: BundledPluginTrust;
  version: string;
  websiteUrl?: string | null;
  [k: string]: unknown;
}
export interface PluginCapabilityManifest {
  apiVersion?: string | null;
  configurationSchema?: string | null;
  contributions: PluginContribution[];
  permissions: PluginPermissions;
  requiredHostCapabilities: string[];
  [k: string]: unknown;
}
export interface PluginContribution {
  apiVersion: string;
  configurationSchema?: string | null;
  declaration: unknown;
  id: string;
  kind: ContributionKind;
  localId: string;
  origin: ContributionOrigin;
  permissions: PluginPermission[];
  pluginId: string;
  requiredHostCapabilities: string[];
  [k: string]: unknown;
}
export interface PluginPermission {
  kind: PluginPermissionKind;
  value: string;
  [k: string]: unknown;
}
export interface PluginPermissions {
  desktop?: string[];
  filesystem?: string[];
  network?: string[];
  secrets?: string[];
  [k: string]: unknown;
}
export interface PluginPermissionsResponse {
  grants: PluginPermissionGrantRecord[];
  requests: PluginPermissionRequest[];
  [k: string]: unknown;
}
export interface PluginPermissionGrantRecord {
  constraint?: {
    [k: string]: unknown;
  };
  grantedAt?: string | null;
  permission: string;
  pluginId: string;
  scope: PluginControlScope;
  status: PluginPermissionGrantStatus;
  updatedAt: string;
  [k: string]: unknown;
}
export interface PluginSettingsResponse {
  schema?: unknown;
  secretBindings: PluginSecretBindingRecord[];
  settings: PluginSettingsRecord;
  [k: string]: unknown;
}
export interface PluginSecretBindingRecord {
  bindingId: string;
  metadata?: {
    [k: string]: unknown;
  };
  pluginId: string;
  scope: PluginControlScope;
  settingKey: string;
  updatedAt: string;
  [k: string]: unknown;
}
export interface PluginSettingsRecord {
  pluginId: string;
  scope: PluginControlScope;
  settings: unknown;
  updatedAt: string;
  [k: string]: unknown;
}
export interface ProviderHealth {
  adapter: ProviderAdapterKind;
  apiKeyConfigured: boolean;
  apiKeySource: string;
  auth: ProviderAuthKind;
  baseUrl: string;
  id: string;
  /**
   * Legacy preset identity for older desktop builds.
   */
  kind: ProviderKind;
  model: string;
  status: string;
  transport: ProviderTransportKind;
  usingMock: boolean;
  [k: string]: unknown;
}
export interface PreviewDescriptor {
  bytes: number;
  capabilities?: PreviewCapabilities;
  contentType: string;
  handlerId?: string | null;
  id: string;
  kind: PreviewKind;
  name: string;
  path?: string | null;
  readonly: boolean;
  revision: string;
  source: PreviewSource;
  [k: string]: unknown;
}
/**
 * Operations granted to a resolved resource. Renderers consume these capabilities instead of inferring authority from where a file came from.
 */
export interface PreviewCapabilities {
  openExternal: boolean;
  rangeRead: boolean;
  read: boolean;
  watch: boolean;
  write: boolean;
  [k: string]: unknown;
}
export interface SandboxDescriptor {
  available: boolean;
  backend?: string | null;
  capabilities: string[];
  enforced: boolean;
  id: string;
  kind: ExecutionEnvironmentKind;
  lifecycle: SandboxLifecycle;
  message: string;
  mode: OsSandboxMode;
  network: NetworkPolicy;
  permissionProfile: string;
  platform: OsSandboxPlatform;
  protectedPaths: string[];
  readableRoots: string[];
  sandboxMode: SandboxMode;
  threadId: string;
  workspaceRoot: string;
  writableRoots: string[];
  [k: string]: unknown;
}
export interface ScmRemoteConnectorResponse {
  binding?: ScmRemoteBinding | null;
  connectors: ScmConnectorDescriptor[];
  remote: LocalGitRemote;
  selection: ScmConnectorSelection;
  [k: string]: unknown;
}
export interface ScmRemoteBinding {
  accountBindingId?: string | null;
  connectorId: string;
  connectorPluginId: string;
  remoteName: string;
  workspaceKey: string;
  [k: string]: unknown;
}
export interface ScmConnectorDescriptor {
  capabilities?: ScmConnectorCapability[];
  connectorId: string;
  displayName: string;
  pluginId: string;
  remoteMatchers?: ScmRemoteUrlMatcher[];
  [k: string]: unknown;
}
export interface ScmRemoteUrlMatcher {
  host: ScmHostMatcher;
  matcherId: string;
  path: ScmPathMatcher;
  schemes?: string[];
  [k: string]: unknown;
}
export interface ScmConnectorCandidate {
  connectorId: string;
  matcherId: string;
  pluginId: string;
  specificity: ScmMatcherSpecificity;
  [k: string]: unknown;
}
export interface ScmMatcherSpecificity {
  host: number;
  path: number;
  scheme: number;
  [k: string]: unknown;
}
export interface AppSettings {
  activeProviderId?: string;
  agentRuntime?: AgentRuntimeSettings;
  defaultWorkspaceRoot?: string | null;
  enterprise?: EnterpriseSettings;
  /**
   * One-time settings migration marker. Older desktop builds persisted `parallelToolCalls: false` as their UI default, so absence means those values have not yet been upgraded to the runtime's default-on policy.
   */
  parallelToolCallsMigrated?: boolean;
  permissionMode: PermissionMode;
  providers?: ProviderSettings[];
  sandbox?: SandboxSettings;
  updatedAt: string;
  [k: string]: unknown;
}
export interface AgentRuntimeSettings {
  autonomy?: AgentAutonomy & string;
  multiAgent?: MultiAgentMode & string;
  personality?: AgentPersonality & string;
  progressUpdates?: ProgressUpdateMode & string;
  [k: string]: unknown;
}
export interface EnterpriseSettings {
  /**
   * Deployment-owned gate. It is intentionally not writable through the settings API, so a consumer session cannot enable enterprise surfaces.
   */
  enabled: boolean;
  [k: string]: unknown;
}
export interface ProviderSettings {
  /**
   * Negotiated wire contract per model. This is the sole runtime source for adapter selection and message lowering; probe reports are diagnostics.
   */
  adapterProfiles?: {
    [k: string]: {
      [k: string]: ProviderAdapterProfile;
    };
  };
  /**
   * Protocols this connection is permitted to use. Empty is a legacy value and is interpreted from `kind` until settings are next saved.
   */
  allowedAdapters?: ProviderAdapterKind[];
  apiKeyConfigured: boolean;
  apiKeySource: string;
  auth?: ProviderAuthKind | null;
  baseUrl: string;
  /**
   * Optional user override. When omitted, the server resolves a known model capability and falls back to a conservative default for custom models.
   */
  contextWindowTokens?: number | null;
  /**
   * Model families the user allowed for this connection. Empty means "not narrowed yet", which shows every synced family rather than none.
   */
  enabledFamilies?: string[];
  healthStatus?: string | null;
  id: string;
  /**
   * Deprecated preset identity retained only for serialized compatibility. Runtime dispatch must use `effective_transport`, `effective_auth`, and `resolved_adapter_for_model` instead.
   */
  kind: ProviderKind;
  maxOutputTokens?: number | null;
  /**
   * Default model for this connection. Threads may override it per conversation; this value is the fallback for new threads and for internal utility calls such as title generation.
   */
  model: string;
  /**
   * Capabilities reported for each model by the connection's catalog.
   */
  modelCapabilities?: {
    [k: string]: ProviderModelCapabilities;
  };
  /**
   * Context windows the connection reported for its own models. Populated on sync when the endpoint publishes them, which is the only real capability detection available; it outranks the built-in table.
   */
  modelContextWindows?: {
    [k: string]: number;
  };
  /**
   * Per-model user overrides. These are intentionally separate from the catalog so a subsequent sync never discards an explicit choice.
   */
  modelSettings?: {
    [k: string]: ProviderModelSettings;
  };
  modelsSyncedAt?: string | null;
  /**
   * User-facing label. Empty values from legacy settings fall back to `id`.
   */
  name?: string;
  /**
   * Last explicit compatibility probe for an OpenAI-compatible `/v1` connection. The endpoint and model are included so stale results are ignored after either setting changes.
   */
  openaiCompatibility?: OpenAiCompatibilityReport | null;
  parallelToolCalls?: boolean;
  /**
   * Connection-wide preference. `None` means use model preference, then the latest probe recommendation, then the legacy preset fallback.
   */
  preferredAdapter?: ProviderAdapterKind | null;
  promptCacheKey?: string | null;
  promptCachePolicy?: PromptCachePolicy | null;
  reasoningEffort?: string | null;
  responsesCompactionThresholdTokens?: number | null;
  rolloutBudget?: RolloutBudgetSettings | null;
  storeResponses?: boolean;
  /**
   * Model ids last returned by the connection's `/v1/models` endpoint. Cached so the picker works offline; refreshed on explicit sync.
   */
  syncedModels?: string[];
  /**
   * `None` means "don't send temperature — let the model use its default." This is important for reasoning models (o-series, GPT-5.x) that reject explicit temperature, and for users who want the vendor default.
   */
  temperature?: number | null;
  transport?: ProviderTransportKind | null;
  [k: string]: unknown;
}
/**
 * Normalized output of provider capability negotiation. Probe diagnostics may remain provider-specific, but the runtime consumes only this stable adapter contract and therefore never needs to reinterpret a probe response.
 */
export interface ProviderAdapterProfile {
  adapter: ProviderAdapterKind;
  baseUrl: string;
  checkedAt: string;
  instructionEncoding: ProviderInstructionEncoding;
  messageProtocol?: ProviderMessageProtocolCapabilities;
  model: string;
  outputProtocol?: ProviderOutputProtocolCapabilities;
  profileVersion: number;
  reasoningProtocol?: ProviderReasoningProtocol & string;
  toolProtocol?: ProviderToolProtocolCapabilities;
  [k: string]: unknown;
}
/**
 * Assistant-message constraints imposed by one concrete wire protocol. These are negotiated or supplied by a trusted built-in endpoint contract; request codecs consume the result without inspecting vendor or model names.
 */
export interface ProviderMessageProtocolCapabilities {
  /**
   * Every assistant message that contains tool calls must preserve the provider-issued `reasoning_content` field in subsequent requests.
   */
  requiresReasoningContentForToolCalls?: boolean;
  [k: string]: unknown;
}
/**
 * Structured final-output features exposed by one concrete wire protocol. These are negotiated during provider setup and never inferred by retrying a modified request after a live turn has already started.
 */
export interface ProviderOutputProtocolCapabilities {
  jsonSchema?: ProviderFeatureSupport & string;
  [k: string]: unknown;
}
/**
 * Capabilities of the selected API protocol as actually exposed by a connection. `Unknown` intentionally behaves like unsupported at selection time so compatible relays always retain the portable function-tool path.
 */
export interface ProviderToolProtocolCapabilities {
  assistantPhase?: ProviderFeatureSupport & string;
  /**
   * Function definitions may be advertised with `defer_loading`.
   */
  deferredToolLoading?: ProviderFeatureSupport & string;
  freeformTools?: ProviderFeatureSupport & string;
  functionTools?: ProviderFeatureSupport & string;
  hostedApplyPatch?: ProviderFeatureSupport & string;
  /**
   * The provider can execute the hosted `tool_search` tool.
   */
  hostedToolSearch?: ProviderFeatureSupport & string;
  /**
   * The endpoint/model can execute the Responses hosted `web_search` tool. Compatible relays must prove this independently from function tools.
   */
  hostedWebSearch?: ProviderFeatureSupport & string;
  /**
   * Deferred functions may be grouped under native namespaces.
   */
  namespaceTools?: ProviderFeatureSupport & string;
  /**
   * The protocol accepts the optional `parallel_tool_calls` request hint.
   */
  parallelToolCalls?: ProviderFeatureSupport & string;
  /**
   * The selected adapter/model/endpoint tuple has passed a production-codec round trip with tools enabled and streaming transport selected. Unknown is deliberately treated as unsupported when preparing a tool-capable request.
   */
  streamingTools?: ProviderFeatureSupport & string;
  strictFunctionTools?: ProviderFeatureSupport & string;
  [k: string]: unknown;
}
export interface ProviderModelCapabilities {
  /**
   * Image-input support reported by the provider's model catalog. `None` means the endpoint did not publish modality metadata for this model.
   */
  supportsVision?: boolean | null;
  [k: string]: unknown;
}
export interface ProviderModelSettings {
  contextWindowTokens?: number | null;
  maxOutputTokens?: number | null;
  /**
   * Optional model-level protocol preference. This is independent from the connection preset and may be overridden again by a thread selection.
   */
  preferredAdapter?: ProviderAdapterKind | null;
  reasoningEffort?: string | null;
  /**
   * An explicit user choice that takes precedence over catalog detection.
   */
  supportsVision?: boolean | null;
  /**
   * Outer `None` inherits the legacy connection setting. Inner `None` explicitly omits the request parameter for this model.
   */
  temperature?: number | null;
  [k: string]: unknown;
}
export interface OpenAiCompatibilityReport {
  baseUrl: string;
  chatCompletions: ProviderFeatureSupport;
  chatFunctionTools?: ProviderFeatureSupport & string;
  chatJsonSchemaOutput?: ProviderFeatureSupport & string;
  /**
   * Assistant-message replay requirements discovered for Chat Completions.
   */
  chatMessageProtocol?: ProviderMessageProtocolCapabilities;
  chatParallelToolCalls?: ProviderFeatureSupport & string;
  /**
   * Reasoning request envelope proven together with the Chat function-tool round trip. `None` is reserved for reports written before v7.
   */
  chatReasoningProtocol?: ProviderReasoningProtocol | null;
  /**
   * Chat Completions function calls remain structurally valid when tool arguments are delivered as a stream. This is negotiated separately from non-streaming function support because compatible relays frequently use different translation paths for the two transports.
   */
  chatStreamingTools?: ProviderFeatureSupport & string;
  /**
   * Chat Completions function tools with provider-enforced strict JSON Schema output. This is negotiated independently from ordinary function tools because many compatible relays accept `tools` but reject `strict`.
   */
  chatStrictFunctionTools?: ProviderFeatureSupport & string;
  checkedAt: string;
  developerMessages: ProviderFeatureSupport;
  messageCompatibility: boolean;
  model: string;
  notes?: string[];
  responses: ProviderFeatureSupport;
  /**
   * The named Responses `apply_patch` tool and its structured call/output item pair. This is negotiated per endpoint/model, never inferred from a vendor or model-name table.
   */
  responsesApplyPatch?: ProviderFeatureSupport & string;
  /**
   * Freeform/custom tool definitions and `custom_tool_call` output items.
   */
  responsesCustomTools?: ProviderFeatureSupport & string;
  /**
   * Function tools using the Responses wire shape. Kept separate from `responses_native_tools`: a relay may accept hosted web search while rejecting application-defined functions, or vice versa.
   */
  responsesFunctionTools?: ProviderFeatureSupport & string;
  responsesJsonSchemaOutput?: ProviderFeatureSupport & string;
  responsesNativeTools?: ProviderFeatureSupport & string;
  responsesParallelToolCalls?: ProviderFeatureSupport & string;
  /**
   * Reasoning request envelope proven together with the Responses function-tool round trip. `None` is reserved for legacy reports.
   */
  responsesReasoningProtocol?: ProviderReasoningProtocol | null;
  /**
   * Responses tool calls remain structurally valid over the streaming event protocol used by the runtime.
   */
  responsesStreamingTools?: ProviderFeatureSupport & string;
  /**
   * Responses function tools with provider-enforced strict JSON Schema output. Kept separate from the portable function-tool capability.
   */
  responsesStrictFunctionTools?: ProviderFeatureSupport & string;
  selectedProtocol: OpenAiProtocol;
  [k: string]: unknown;
}
export interface RolloutBudgetSettings {
  limitTokens: number;
  prefillTokenWeight?: number;
  samplingTokenWeight?: number;
  [k: string]: unknown;
}
export interface SandboxSettings {
  enforcement: SandboxEnforcement;
  network: NetworkPolicy;
  readPaths: string[];
  sandboxMode: SandboxMode;
  windowsBackend: WindowsSandboxBackend;
  writableRoots: string[];
  [k: string]: unknown;
}
export interface PreviewWorkbook {
  bytes: number;
  previewId: string;
  sheets: PreviewSheet[];
  [k: string]: unknown;
}
export interface PreviewSheet {
  columnCount: number;
  kind: SheetKind;
  name: string;
  rowCount: number;
  visibility: SheetVisibility;
  [k: string]: unknown;
}
export interface PreviewRange {
  previewId: string;
  range: CellRange;
  rows: SpreadsheetCell[][];
  sheet: string;
  [k: string]: unknown;
}
/**
 * An inclusive range using zero-based coordinates.
 */
export interface CellRange {
  end: CellAddress;
  start: CellAddress;
}
/**
 * Zero-based row and column coordinates.
 */
export interface CellAddress {
  column: number;
  row: number;
}
export interface SpreadsheetCell {
  formula?: string | null;
  value: SpreadsheetCellValue;
  [k: string]: unknown;
}
export interface ExcelDateTimeValue {
  isDuration: boolean;
  serial: number;
  [k: string]: unknown;
}
export interface ThreadCapabilitiesResponse {
  capabilityProjection: CapabilityProjection;
  experienceMode: ExperienceMode;
  generatedAt: string;
  plugins: ThreadPluginCapabilities[];
  promptProfileId: string;
  snapshot: CapabilityActivationSnapshot;
  threadId: string;
  workspaceRoot: string;
  [k: string]: unknown;
}
export interface ThreadPluginCapabilities {
  contributions: PluginContributionRecord[];
  enabled: boolean;
  grantedPermissions: string[];
  pluginId: string;
  pluginName: string;
  [k: string]: unknown;
}
export interface CapabilityActivationSnapshot {
  active: ActivatedContribution[];
  conflicts: CapabilityConflict[];
  scope: CapabilityActivationScope;
  unavailable: UnavailableContribution[];
  [k: string]: unknown;
}
export interface ActivatedContribution {
  contribution: PluginContribution;
  pluginName: string;
  source: PluginSource;
  trust: BundledPluginTrust;
  [k: string]: unknown;
}
export interface CapabilityConflict {
  contributionIds: string[];
  key: string;
  [k: string]: unknown;
}
export interface CapabilityActivationScope {
  threadId?: string | null;
  workspaceId?: string | null;
  [k: string]: unknown;
}
export interface UnavailableContribution {
  contribution: ActivatedContribution;
  reason: CapabilityUnavailableReason;
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
export interface TurnFileDiffPreview {
  binary: boolean;
  diff: string;
  newPath?: string | null;
  nextOffset?: number | null;
  offset: number;
  oldPath?: string | null;
  path: string;
  totalBytes: number;
  turnId: string;
  [k: string]: unknown;
}
export interface TurnRecord {
  completedAt?: string | null;
  error?: string | null;
  /**
   * Monotonic execution attempt inside this logical Turn. Interactive resumes increment this value without changing `turn_id`.
   */
  invocationId?: number;
  startedAt: string;
  status: TurnStatus;
  threadId: string;
  turnId: string;
  updatedAt: string;
  userMessageId: string;
  [k: string]: unknown;
}
export interface WindowsSandboxSetupStatus {
  backend: string;
  components: WindowsSandboxSetupComponents;
  helperAvailable: boolean;
  issues: string[];
  state: WindowsSandboxSetupState;
  stateDir?: string | null;
  supported: boolean;
  [k: string]: unknown;
}
export interface WindowsSandboxSetupComponents {
  credentials: boolean;
  offlineIdentity: boolean;
  offlineNetworkPolicy: boolean;
  onlineIdentity: boolean;
  [k: string]: unknown;
}
export interface WorkflowEvaluationSummary {
  averageScore?: number | null;
  deliveryStatusCounts: {
    [k: string]: number;
  };
  deploymentId: string;
  evaluationCount: number;
  failureClusters: WorkflowFailureCluster[];
  passRate?: number | null;
  runStatusCounts: {
    [k: string]: number;
  };
  totalRuns: number;
  [k: string]: unknown;
}
export interface WorkflowFailureCluster {
  count: number;
  key: string;
  sample: string;
  [k: string]: unknown;
}
export interface HealthResponse {
  apiVersion: number;
  officeRuntime: OfficeRuntimeStatus;
  ok: boolean;
  service: string;
  shellRuntime: ShellRuntimeStatus;
  [k: string]: unknown;
}
export interface OfficeRuntimeStatus {
  managedError?: string | null;
  managedStatus: ManagedOfficeRuntimeStatus;
  managedVersion: string;
  runtime?: OfficePythonRuntime | null;
  [k: string]: unknown;
}
export interface OfficePythonRuntime {
  executable: string;
  openpyxlVersion: string;
  pythonVersion: string;
  root: string;
  runtimeVersion: string;
  source: OfficeRuntimeSource;
  [k: string]: unknown;
}
export interface ShellRuntimeStatus {
  managedError?: string | null;
  managedStatus: ManagedPowerShellStatus;
  managedVersion: string;
  runtime: ShellRuntime;
  [k: string]: unknown;
}
export interface ShellRuntime {
  dialect: ShellDialect;
  program: string;
  source: ShellRuntimeSource;
  version?: string | null;
  [k: string]: unknown;
}
export interface LibraryIngestionResponseView {
  status: string;
  [k: string]: unknown;
}
export interface PluginView {
  compatible: boolean;
  mcpServers: McpServerView[];
  plugin: PluginDescriptor;
  skillIds: string[];
  threadEnabled: boolean;
  [k: string]: unknown;
}
export interface MediaHandlerInvocationResponse {
  bytesRead: number;
  contributionId: string;
  output: MediaHandlerResultEnvelopeV1;
  pluginId: string;
  runtime: MediaHandlerRuntime;
  [k: string]: unknown;
}
export interface MediaHandlerResultEnvelopeV1 {
  apiVersion: string;
  kind: MediaHandlerOperation;
  payload: unknown;
  [k: string]: unknown;
}
export interface AgentListItem {
  activity?: AgentActivityWindow | null;
  agent: AgentThreadRecord;
  availability: AgentAvailability;
  latestTurn?: AgentTurnRecord | null;
  [k: string]: unknown;
}
export interface AgentActivityWindow {
  agentThreadId: string;
  agentTurnId: string;
  cursor: number;
  modelRound?: number | null;
  reasoningTail?: string | null;
  recentEvents: ActivityEvent[];
  recentToolResults: ToolResultProjection[];
  turnStatus: AgentTurnStatus;
  [k: string]: unknown;
}
export interface ActivityEvent {
  createdAt: string;
  details?: ActivityEventDetails | null;
  kind: string;
  seq: number;
  [k: string]: unknown;
}
export interface ToolResultProjection {
  invocationId: string;
  kind: ToolResultKind;
  preview: unknown;
  resultRef: string;
  toolName?: string | null;
  truncated: boolean;
  [k: string]: unknown;
}
export interface AgentThreadRecord {
  agentType: string;
  archivedAt?: string | null;
  createdAt: string;
  id: string;
  parentAgentThreadId?: string | null;
  path: string;
  runtimeSnapshotId: string;
  sessionId: string;
  spawnPolicy: AgentSpawnPolicy;
  taskName: string;
  [k: string]: unknown;
}
export interface AgentSpawnPolicy {
  allowChildSpawns: boolean;
  maxDepth: number;
  maxDirectChildren: number;
  [k: string]: unknown;
}
export interface AgentTurnRecord {
  agentThreadId: string;
  completedAt?: string | null;
  createdAt: string;
  id: string;
  invocationId: number;
  outcomeRef?: string | null;
  requestedByAgentThreadId?: string | null;
  requestedByTurnId?: string | null;
  sequence: number;
  sessionId: string;
  startedAt?: string | null;
  status: AgentTurnStatus;
  taskMessage: string;
  [k: string]: unknown;
}
export interface ArtifactMetadata {
  bytes: number;
  contentType: string;
  createdAt: string;
  id: string;
  kind: string;
  metadata: unknown;
  storage: ArtifactStorageMetadata;
  threadId: string;
  [k: string]: unknown;
}
export interface WindowTarget {
  bounds: ScreenRect;
  executable?: string | null;
  isForeground: boolean;
  processId: number;
  title: string;
  /**
   * Opaque, runtime-issued identifier. On Windows this is formatted from an HWND but callers must never synthesize it: `observe` verifies it against a live process before binding it.
   */
  windowId: string;
  [k: string]: unknown;
}
export interface ScreenRect {
  height: number;
  width: number;
  x: number;
  y: number;
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
/**
 * Describes the exact structured input sent by a provider adapter using only hashes, safe field names, and local token estimates.
 */
export interface ProviderCacheTrace {
  configuration?: ProviderCacheTraceProperty[];
  prefixHash: string;
  previousResponseIdPresent: boolean;
  promptCacheKeyHash?: string | null;
  schemaVersion: number;
  segments: ProviderCacheTraceSegment[];
  toolCatalogHash?: string | null;
  [k: string]: unknown;
}
export interface ProviderCacheTraceProperty {
  name: string;
  valueHash: string;
  [k: string]: unknown;
}
/**
 * A content-free fingerprint of one provider-visible prompt segment. These records are safe to keep in the compact conversation projection: they retain enough structure to explain cache-prefix changes without persisting prompt text, tool output, or image bytes a second time.
 */
export interface ProviderCacheTraceSegment {
  contentHash: string;
  kind: ProviderCacheTraceSegmentKind;
  name?: string | null;
  source: string;
  tokenEstimate: number;
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
export interface ContextCompactionMetrics {
  activeConstraintRetentionPercent: number;
  cacheHitPercent?: number;
  cachedInputTokens?: number;
  checkpointTokens: number;
  factRetentionPercent: number;
  /**
   * Local estimate of the logical agent request before compaction.
   */
  inputTokens: number;
  latencyMs: number;
  /**
   * Local estimate after the checkpoint starts a new request epoch. The provider-reported result is only available after that request finishes.
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
export interface LibraryProviderDescriptor {
  capabilities: LibraryProviderCapabilities;
  description: string;
  id: string;
  name: string;
  title: string;
  [k: string]: unknown;
}
export interface LibraryProviderCapabilities {
  graphPaths: boolean;
  incrementalUpload: boolean;
  llmPlanning: boolean;
  temporalMemory: boolean;
  [k: string]: unknown;
}
export interface LibrarySourcePageView {
  authorizedTotal: number;
  hasMore: boolean;
  indexTotal: number;
  items: unknown[];
  limit: number;
  offset: number;
  total: number;
  [k: string]: unknown;
}
export interface McpToolDescriptor {
  /**
   * Provider-owned extension metadata from the MCP tool's `_meta` field. OpenTopia only acts on explicitly namespaced contracts it understands.
   */
  _meta?: {
    [k: string]: unknown;
  };
  annotations: unknown;
  description?: string | null;
  inputSchema: unknown;
  permissionLabels: string[];
  publicName: string;
  serverId: string;
  toolName: string;
  [k: string]: unknown;
}
export interface Approval {
  action: string;
  approvalId: string;
  createdAt: string;
  decidedAt?: string | null;
  reason: string;
  status: ApprovalStatus;
  threadId: string;
  [k: string]: unknown;
}
export interface UserInputRecord {
  answeredAt?: string | null;
  createdAt: string;
  request: UserInputRequest;
  response?: UserInputResponse | null;
  status: UserInputStatus;
  threadId: string;
  [k: string]: unknown;
}
export interface ProviderDriverDescriptor {
  adapter: ProviderAdapterKind;
  displayName: string;
  id: string;
  transport: ProviderTransportKind;
  trust: ProviderDriverTrust;
  [k: string]: unknown;
}
export interface SkillDescriptor {
  description: string;
  id: string;
  name: string;
  path: string;
  pluginId?: string | null;
  scope: SkillScope;
  [k: string]: unknown;
}
export interface TerminalEvent {
  command?: string | null;
  commandId: string;
  createdAt: string;
  cwd?: string | null;
  data?: string | null;
  exitCode?: number | null;
  id: string;
  message?: string | null;
  seq: number;
  success?: boolean | null;
  threadId: string;
  type: TerminalEventKind;
  [k: string]: unknown;
}
export interface ThreadMcpServerView {
  binding?: ThreadMcpServer | null;
  enabled: boolean;
  server: McpServerConfig;
  [k: string]: unknown;
}
export interface ThreadMcpServer {
  enabled: boolean;
  serverId: string;
  threadId: string;
  updatedAt: string;
  [k: string]: unknown;
}
export interface WorkflowDeliveryReceiptV1 {
  attempt: number;
  createdAt: string;
  deliveredAt?: string | null;
  deploymentId: string;
  error?: string | null;
  id: string;
  idempotencyKey: string;
  outputKind: string;
  providerResult?: unknown;
  responseStatus?: number | null;
  revision: number;
  runId: string;
  schemaVersion: number;
  status: WorkflowDeliveryStatusV1;
  updatedAt: string;
  [k: string]: unknown;
}
export interface WorkspaceTree {
  entries: WorkspaceEntry[];
  path: string;
  root: string;
  [k: string]: unknown;
}
export interface WorkspaceEntry {
  kind: WorkspaceEntryKind;
  modifiedAt?: string | null;
  name: string;
  path: string;
  size?: number | null;
  [k: string]: unknown;
}
export interface ComputerObservation {
  /**
   * Reserved for a future UIA adapter. It is omitted rather than fabricating accessibility data.
   */
  accessibilityTree?: {
    [k: string]: unknown;
  };
  /**
   * Coordinates in computer actions are relative to this image, never to CSS/DIP coordinates.
   */
  captureRect: ScreenRect;
  capturedAt: string;
  imageHeight: number;
  imageWidth: number;
  observationId: string;
  screenshot?: ComputerScreenshot | null;
  sessionId: string;
  target: WindowTarget;
  unstable: boolean;
  [k: string]: unknown;
}
export interface ComputerScreenshot {
  bytes: number[];
  mimeType: string;
  [k: string]: unknown;
}
export interface AppViewMessage {
  channel: string;
  payload: unknown;
  sentAt: string;
  sessionId: string;
  [k: string]: unknown;
}
export interface TurnUndoPreview {
  additions: number;
  canUndo: boolean;
  changeSet: TurnChangeSet;
  conflicts: TurnUndoConflict[];
  deletions: number;
  filesToChange: number;
  turnId: string;
  [k: string]: unknown;
}
export interface TurnUndoConflict {
  kind: TurnUndoConflictKind;
  path?: string | null;
  reason: string;
  [k: string]: unknown;
}
export interface FlowDefinitionV1 {
  budget: FlowBudgetV1;
  capabilities: CapabilityProjection;
  categories: string[];
  contentHash: string;
  description: string;
  flowId: string;
  graph: GraphDefinitionV1;
  id: string;
  inputSchema: unknown;
  name: string;
  outputSchema: unknown;
  owner: string;
  publishedAt: string;
  publishedBy: string;
  riskClass: AgentRiskClassV1;
  schemaVersion: number;
  source: FlowSourceV1;
  version: number;
  [k: string]: unknown;
}
export interface WorkspaceFilePreview {
  bytes: number;
  content: string;
  path: string;
  readonly: boolean;
  truncated: boolean;
  [k: string]: unknown;
}
export interface RefreshConnectionCapabilitiesResponse {
  capabilityRevision: ConnectionCapabilityRevisionV1;
  changed: boolean;
  connection: ConnectionV1;
  diff: ConnectionCapabilityDiffView;
  [k: string]: unknown;
}
export interface ConnectionCapabilityDiffView {
  addedCapabilityIds: string[];
  changedCapabilityIds: string[];
  removedCapabilityIds: string[];
  [k: string]: unknown;
}
export interface ResolveHumanTaskResponse {
  deliveryReceipt?: WorkflowDeliveryReceiptV1 | null;
  run?: FlowRunV1 | null;
  task: HumanTaskV1;
  [k: string]: unknown;
}
export interface UserInputResponseAccepted {
  accepted: boolean;
  resumed: boolean;
  [k: string]: unknown;
}
export interface ExternalActionResumeResponse {
  accepted: boolean;
  invocationId: number;
  resumed: boolean;
  turnId: string;
  [k: string]: unknown;
}
export interface BrowserOutput {
  contents: BrowserContent[];
  metadata: unknown;
  url?: string | null;
  [k: string]: unknown;
}
export interface GitWorkflowResponse {
  action: GitWorkflowActionKind;
  exitCode?: number | null;
  stderr: string;
  stdout: string;
  success: boolean;
  truncated: boolean;
  [k: string]: unknown;
}
export interface LibrarySearchResponseView {
  diagnostics: unknown;
  pack: unknown;
  [k: string]: unknown;
}
export interface PluginActivationResponse {
  activation: PluginActivationRecord;
  effectiveEnabled: boolean;
  [k: string]: unknown;
}
export interface CodexLoginStart {
  authUrl?: string | null;
  loginId: string;
  loginType: string;
  userCode?: string | null;
  verificationUrl?: string | null;
  [k: string]: unknown;
}
export interface AppViewSessionResponse {
  contentPath: string;
  descriptor: AppViewDescriptor;
  sessionId: string;
  startedAt: string;
  status: AppViewSessionStatus;
  stoppedAt?: string | null;
  threadId: string;
  [k: string]: unknown;
}
export interface TerminalStartResponse {
  commandId: string;
  historyUrl: string;
  status: string;
  streamUrl: string;
  threadId: string;
  [k: string]: unknown;
}
export interface AppViewSession {
  descriptor: AppViewDescriptor;
  sessionId: string;
  startedAt: string;
  status: AppViewSessionStatus;
  stoppedAt?: string | null;
  threadId: string;
  [k: string]: unknown;
}
export interface ProviderModelSyncResult {
  defaultModel: string;
  modelCapabilities: {
    [k: string]: ProviderModelCapabilities;
  };
  modelContextWindows: {
    [k: string]: number;
  };
  models: string[];
  provider: ProviderSettings;
  providerId: string;
  syncedAt: string;
  [k: string]: unknown;
}
export interface TestConnectionResponse {
  connection: ConnectionV1;
  health: ConnectionHealthView;
  [k: string]: unknown;
}
export interface ConnectionHealthView {
  authStatus: ConnectionAuthVerificationV1;
  checkedAt: string;
  message: string;
  ok: boolean;
  runtimeStatus: McpLifecycleStatus;
  toolsCount: number;
  [k: string]: unknown;
}
export interface ProviderHealthCheck {
  error?: string | null;
  latencyMs?: number | null;
  modelAvailable: boolean;
  openaiCompatibility?: OpenAiCompatibilityReport | null;
  reachable: boolean;
  [k: string]: unknown;
}
export interface TurnUndoResult {
  applied: boolean;
  changeSet: TurnChangeSet;
  filesChanged: number;
  preview: TurnUndoPreview;
  [k: string]: unknown;
}
