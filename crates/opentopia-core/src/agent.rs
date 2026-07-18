use crate::browser::{BrowserRuntime, BrowserRuntimeConfig, LocalBrowserRuntime};
use crate::mcp::McpToolDescriptor;
use crate::mcp_host::McpExtensionHost;
use crate::model::{
    AgentEventPayload, Message, MessageRole, ModelContentPart, TaskPlan, TaskPlanStepStatus,
    ToolCall, ToolResult,
};
use crate::policy::{BasicPolicyEngine, PermissionMode};
use crate::provider::{
    MockProvider, ModelConversationMessage, ModelConversationRole, ModelProvider, ModelRequest,
    ModelResponse, ModelStreamDelta, OpenAiCompatibleProvider, ProviderToolCall,
    ProviderToolCandidate, ProviderToolResult,
};
use crate::sandbox::{LocalSandboxConfig, SandboxMode};
use crate::settings::{AppSettings, ProviderKind};
use crate::store::SessionStore;
use crate::subagents::SubagentScheduler;
use crate::tools::{
    browser_domain_approval_action, browser_domain_from_url, McpToolWrapper, ToolContext,
    ToolRegistry,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const DEFAULT_MAX_EQUIVALENT_TOOL_CALLS: usize = 3;
const DEFAULT_MAX_REPEATED_TOOL_CYCLES: usize = 3;
const DEFAULT_MAX_LOOP_GUARD_CORRECTIONS: usize = 3;
const MAX_TRACKED_TOOL_SIGNATURES: usize = 24;
const MAX_DETECTED_TOOL_CYCLE_LENGTH: usize = 6;
const MIN_RETAINED_TOOL_RESULTS_AFTER_COMPACTION: usize = 4;
const MAX_COMPACTED_TOOL_HISTORY_CHARS: usize = 12_000;
const MAX_COMPLETION_MODE_VIOLATIONS: usize = 2;

pub type AgentEventSender = mpsc::UnboundedSender<AgentEventPayload>;

#[derive(Debug, Clone)]
pub struct AgentTurnResult {
    pub events: Vec<AgentEventPayload>,
    pub outcome: AgentTurnOutcome,
}

#[derive(Debug, Clone)]
pub enum AgentTurnOutcome {
    Completed,
    Suspended {
        approval_id: Uuid,
        continuation: AgentContinuation,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentLoopPolicy {
    #[serde(default = "default_max_equivalent_tool_calls")]
    pub max_equivalent_tool_calls: usize,
    #[serde(default = "default_max_repeated_tool_cycles")]
    pub max_repeated_tool_cycles: usize,
    #[serde(default = "default_max_loop_guard_corrections")]
    pub max_loop_guard_corrections: usize,
}

impl Default for AgentLoopPolicy {
    fn default() -> Self {
        Self {
            max_equivalent_tool_calls: DEFAULT_MAX_EQUIVALENT_TOOL_CALLS,
            max_repeated_tool_cycles: DEFAULT_MAX_REPEATED_TOOL_CYCLES,
            max_loop_guard_corrections: DEFAULT_MAX_LOOP_GUARD_CORRECTIONS,
        }
    }
}

impl AgentLoopPolicy {
    fn normalized(mut self) -> Self {
        self.max_equivalent_tool_calls = self.max_equivalent_tool_calls.max(1);
        self.max_repeated_tool_cycles = self.max_repeated_tool_cycles.max(2);
        self.max_loop_guard_corrections = self.max_loop_guard_corrections.max(1);
        self
    }

    fn status(
        &self,
        total_tool_rounds: usize,
        loop_guard_corrections: usize,
        context_budget: Option<&ContextBudget>,
    ) -> AgentLoopStatus {
        let context_used_tokens = context_budget.map(|budget| budget.used_tokens);
        let context_max_tokens = context_budget.map(|budget| budget.max_tokens);
        AgentLoopStatus {
            total_tool_rounds,
            max_equivalent_tool_calls: self.max_equivalent_tool_calls,
            max_repeated_tool_cycles: self.max_repeated_tool_cycles,
            max_loop_guard_corrections: self.max_loop_guard_corrections,
            loop_guard_corrections,
            context_max_tokens,
            context_used_tokens,
            context_remaining_tokens: context_max_tokens
                .zip(context_used_tokens)
                .map(|(max, used)| max.saturating_sub(used)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentLoopStatus {
    pub total_tool_rounds: usize,
    pub max_equivalent_tool_calls: usize,
    pub max_repeated_tool_cycles: usize,
    pub max_loop_guard_corrections: usize,
    pub loop_guard_corrections: usize,
    pub context_max_tokens: Option<usize>,
    pub context_used_tokens: Option<usize>,
    pub context_remaining_tokens: Option<usize>,
}

fn default_max_equivalent_tool_calls() -> usize {
    DEFAULT_MAX_EQUIVALENT_TOOL_CALLS
}

fn default_max_repeated_tool_cycles() -> usize {
    DEFAULT_MAX_REPEATED_TOOL_CYCLES
}

fn default_max_loop_guard_corrections() -> usize {
    DEFAULT_MAX_LOOP_GUARD_CORRECTIONS
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentLoopGuardState {
    pub total_tool_rounds: usize,
    pub blocked_equivalent_calls: usize,
    #[serde(default)]
    equivalent_call_counts: BTreeMap<String, usize>,
    #[serde(default)]
    recent_tool_signatures: Vec<String>,
    #[serde(default)]
    progress_tool_signatures: BTreeMap<String, usize>,
    #[serde(default)]
    last_result_fingerprints: BTreeMap<String, String>,
    #[serde(default)]
    loop_guard_corrections_since_progress: usize,
    #[serde(default)]
    last_plan_fingerprint: Option<String>,
    #[serde(default)]
    last_successful_verification: Option<String>,
    #[serde(default)]
    latest_plan: Option<TaskPlan>,
    #[serde(default)]
    workspace_changed_since_verification: bool,
    #[serde(default)]
    completion_mode: bool,
    #[serde(default)]
    blocked_completion_mode_calls: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentContinuation {
    pub thread_id: Uuid,
    pub user_message_id: Uuid,
    pub workspace_root: PathBuf,
    pub context_summary: Option<String>,
    pub conversation: Vec<ModelConversationMessage>,
    pub permission_mode: PermissionMode,
    pub context_budget: Option<ContextBudget>,
    #[serde(default, alias = "executionBudget")]
    pub loop_policy: AgentLoopPolicy,
    #[serde(default)]
    pub loop_guard: AgentLoopGuardState,
    #[serde(default)]
    pub continuation_kind: AgentContinuationKind,
    pub state: AgentContinuationState,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentContinuationKind {
    #[default]
    Approval,
    // Kept so continuations persisted by versions before progress-based loop detection still load.
    #[serde(rename = "budget_checkpoint")]
    LegacyBudgetCheckpoint,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentContinuationState {
    Provider {
        model_user_message: String,
        #[serde(default)]
        model_user_content: Vec<ModelContentPart>,
        tool_candidates: Vec<ProviderToolCandidate>,
        provider_tool_calls: Vec<ProviderToolCall>,
        provider_tool_results: Vec<ProviderToolResult>,
        pending_tool_calls: Vec<ProviderToolCall>,
        #[serde(default)]
        compacted_tool_history: String,
        current_round: usize,
    },
}

struct TurnEvents {
    items: Vec<AgentEventPayload>,
    sender: Option<AgentEventSender>,
}

impl TurnEvents {
    fn new(sender: Option<AgentEventSender>) -> Self {
        Self {
            items: Vec::new(),
            sender,
        }
    }

    fn push(&mut self, payload: AgentEventPayload) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(payload.clone());
        }
        self.items.push(payload);
    }

    fn into_vec(self) -> Vec<AgentEventPayload> {
        self.items
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextBudget {
    pub max_tokens: usize,
    pub used_tokens: usize,
    pub warnings: Vec<String>,
}

impl ContextBudget {
    pub fn new(max_tokens: usize) -> Self {
        Self {
            max_tokens,
            used_tokens: 0,
            warnings: Vec::new(),
        }
    }

    pub fn record_tokens(&mut self, tokens: usize) {
        self.used_tokens += tokens;
        let usage_pct = self.used_tokens as f64 / self.max_tokens as f64;
        if usage_pct >= 0.90 && usage_pct < 0.95 {
            let msg = format!(
                "Context budget at {:.1}% (used {} / max {} tokens)",
                usage_pct * 100.0,
                self.used_tokens,
                self.max_tokens
            );
            if !self.warnings.iter().any(|w| w.contains("90%")) {
                self.warnings.push(msg);
            }
        } else if usage_pct >= 0.95 && usage_pct < 1.0 {
            let msg = format!(
                "Context budget critically high at {:.1}% (used {} / max {} tokens)",
                usage_pct * 100.0,
                self.used_tokens,
                self.max_tokens
            );
            if !self.warnings.iter().any(|w| w.contains("95%")) {
                self.warnings.push(msg);
            }
        }
    }

    pub fn is_exceeded(&self) -> bool {
        self.used_tokens >= self.max_tokens
    }

    pub fn estimate_tokens(text: &str) -> usize {
        (text.len() + 3) / 4
    }
}

#[derive(Clone)]
pub struct AgentCore {
    provider: Arc<dyn ModelProvider>,
    tools: ToolRegistry,
    pub mcp_host: Option<McpExtensionHost>,
    sandbox_config: LocalSandboxConfig,
    browser: Arc<dyn BrowserRuntime>,
    subagents: Option<SubagentScheduler>,
    subagent_depth: u8,
    subagent_parent_turn_id: Option<Uuid>,
}

impl Default for AgentCore {
    fn default() -> Self {
        Self {
            provider: Arc::new(MockProvider),
            tools: ToolRegistry::with_builtins(),
            mcp_host: None,
            sandbox_config: LocalSandboxConfig::from_env(),
            browser: Arc::new(LocalBrowserRuntime::new(BrowserRuntimeConfig::default())),
            subagents: None,
            subagent_depth: 0,
            subagent_parent_turn_id: None,
        }
    }
}

impl AgentCore {
    pub fn from_env() -> Self {
        let provider: Arc<dyn ModelProvider> = OpenAiCompatibleProvider::from_env()
            .map(|provider| Arc::new(provider) as Arc<dyn ModelProvider>)
            .unwrap_or_else(|| Arc::new(MockProvider));
        Self {
            provider,
            tools: ToolRegistry::with_builtins(),
            mcp_host: None,
            sandbox_config: LocalSandboxConfig::from_env(),
            browser: Arc::new(LocalBrowserRuntime::new(BrowserRuntimeConfig::default())),
            subagents: None,
            subagent_depth: 0,
            subagent_parent_turn_id: None,
        }
    }

    pub fn from_settings(settings: &AppSettings) -> Self {
        let active = settings.active_provider();
        let provider: Arc<dyn ModelProvider> = if active.kind == ProviderKind::Mock {
            Arc::new(MockProvider)
        } else {
            OpenAiCompatibleProvider::from_settings(active)
                .map(|provider| Arc::new(provider) as Arc<dyn ModelProvider>)
                .unwrap_or_else(|| Arc::new(MockProvider))
        };
        Self {
            provider,
            tools: ToolRegistry::with_builtins(),
            mcp_host: None,
            sandbox_config: settings.sandbox.to_local_sandbox_config(),
            browser: Arc::new(LocalBrowserRuntime::new(BrowserRuntimeConfig::default())),
            subagents: None,
            subagent_depth: 0,
            subagent_parent_turn_id: None,
        }
    }

    pub fn new(provider: Arc<dyn ModelProvider>, tools: ToolRegistry) -> Self {
        Self {
            provider,
            tools,
            mcp_host: None,
            sandbox_config: LocalSandboxConfig::from_env(),
            browser: Arc::new(LocalBrowserRuntime::new(BrowserRuntimeConfig::default())),
            subagents: None,
            subagent_depth: 0,
            subagent_parent_turn_id: None,
        }
    }

    pub fn with_sandbox_config(mut self, sandbox_config: LocalSandboxConfig) -> Self {
        self.sandbox_config = sandbox_config;
        self
    }

    pub fn set_sandbox_config(&mut self, sandbox_config: LocalSandboxConfig) {
        self.sandbox_config = sandbox_config;
    }

    pub fn set_browser_runtime(&mut self, browser: Arc<dyn BrowserRuntime>) {
        self.browser = browser;
    }

    pub fn set_subagent_scheduler(&mut self, scheduler: SubagentScheduler) {
        self.subagents = Some(scheduler);
    }

    pub fn set_subagent_context(&mut self, parent_turn_id: Uuid, depth: u8) {
        self.subagent_parent_turn_id = Some(parent_turn_id);
        self.subagent_depth = depth;
    }

    fn apply_subagent_context(&self, context: &mut ToolContext, fallback_turn_id: Uuid) {
        context.subagents = self.subagents.clone();
        context.parent_turn_id = Some(self.subagent_parent_turn_id.unwrap_or(fallback_turn_id));
        context.subagent_depth = self.subagent_depth;
        context.browser = Some(self.browser.clone());
    }

    pub fn with_mcp_host(mut self, host: McpExtensionHost) -> Self {
        self.mcp_host = Some(host);
        self
    }

    pub fn set_mcp_host(&mut self, host: McpExtensionHost) {
        self.mcp_host = Some(host);
    }

    pub fn clear_mcp_host(&mut self) {
        self.mcp_host = None;
    }

    pub async fn mcp_tool_catalog(&self) -> Vec<McpToolDescriptor> {
        match self.mcp_host.as_ref() {
            Some(host) => host.all_cached_tools().await,
            None => Vec::new(),
        }
    }

    pub async fn sync_mcp_tools(&mut self) -> Vec<String> {
        let host = match self.mcp_host.as_ref() {
            Some(host) => host.clone(),
            None => return Vec::new(),
        };
        let descriptors = host.all_cached_tools().await;
        let mut registered = Vec::new();
        for desc in descriptors {
            let wrapper = McpToolWrapper::new(host.clone(), desc);
            let name = wrapper.descriptor().public_name.clone();
            registered.push(name.clone());
            self.tools.insert(name, Arc::new(wrapper));
        }
        registered
    }

    pub async fn sync_mcp_tools_for_servers(&mut self, server_ids: &[Uuid]) -> Vec<String> {
        let host = match self.mcp_host.as_ref() {
            Some(host) => host.clone(),
            None => return Vec::new(),
        };
        let mut registered = Vec::new();
        for server_id in server_ids {
            for desc in host.cached_tools(*server_id).await {
                let wrapper = McpToolWrapper::new(host.clone(), desc);
                let name = wrapper.descriptor().public_name.clone();
                registered.push(name.clone());
                self.tools.insert(name, Arc::new(wrapper));
            }
        }
        registered
    }

    pub async fn run_turn(&self, input: AgentTurnInput) -> anyhow::Result<Vec<AgentEventPayload>> {
        Ok(self.run_turn_detailed_streaming(input, None).await?.events)
    }

    pub async fn run_turn_streaming(
        &self,
        input: AgentTurnInput,
        sender: Option<AgentEventSender>,
    ) -> anyhow::Result<Vec<AgentEventPayload>> {
        Ok(self
            .run_turn_detailed_streaming(input, sender)
            .await?
            .events)
    }

    pub async fn run_turn_detailed_streaming(
        &self,
        input: AgentTurnInput,
        sender: Option<AgentEventSender>,
    ) -> anyhow::Result<AgentTurnResult> {
        let mut events = TurnEvents::new(sender);
        let mut budget = input.context_budget;
        let loop_policy = AgentLoopPolicy::default().normalized();

        events.push(AgentEventPayload::TurnStarted {
            user_message_id: input.user_message_id,
        });

        if let Some(ref mut budget) = budget {
            let input_tokens = ContextBudget::estimate_tokens(&input.content);
            budget.record_tokens(input_tokens);
        }

        let model_user_message =
            provider_user_message(&input.content, input.context_summary.as_deref());
        let tool_candidates = self.provider_tool_candidates();
        let response = self
            .complete_model(
                ModelRequest {
                    system_prompt: provider_system_prompt(
                        &loop_policy.status(0, 0, budget.as_ref()),
                        &input.workspace_root,
                        &self.sandbox_config,
                        false,
                    ),
                    conversation: input.conversation.clone(),
                    user_message: model_user_message.clone(),
                    user_content: input.user_content.clone(),
                    tool_candidates: tool_candidates.clone(),
                    previous_tool_calls: Vec::new(),
                    tool_results: Vec::new(),
                },
                &mut events,
            )
            .await?;
        if let Some(ref mut budget) = budget {
            budget.record_tokens(ContextBudget::estimate_tokens(&response.text));
        }
        if response.tool_calls.is_empty() {
            return Ok(finalize_provider_turn(
                input.thread_id,
                response,
                Vec::new(),
                budget,
                events,
            ));
        }

        let provider_tool_calls = response.tool_calls.clone();
        let loop_guard = AgentLoopGuardState {
            total_tool_rounds: 1,
            ..AgentLoopGuardState::default()
        };
        self.continue_provider_turn(
            input.thread_id,
            input.user_message_id,
            input.workspace_root,
            input.context_summary,
            input.conversation,
            input.permission_mode,
            budget,
            loop_policy,
            loop_guard,
            input.store,
            input.cancellation,
            model_user_message,
            input.user_content,
            tool_candidates,
            provider_tool_calls,
            Vec::new(),
            response.tool_calls,
            String::new(),
            1,
            &mut events,
        )
        .await
    }

    pub async fn resume_turn_streaming(
        &self,
        continuation: AgentContinuation,
        approved: bool,
        store: Option<Arc<dyn SessionStore>>,
        cancellation: Option<CancellationToken>,
        sender: Option<AgentEventSender>,
    ) -> anyhow::Result<AgentTurnResult> {
        let mut events = TurnEvents::new(sender);
        events.push(AgentEventPayload::TurnStarted {
            user_message_id: continuation.user_message_id,
        });

        let is_legacy_budget_checkpoint =
            continuation.continuation_kind == AgentContinuationKind::LegacyBudgetCheckpoint;
        let loop_policy = continuation.loop_policy.clone().normalized();
        let mut loop_guard = continuation.loop_guard.clone();

        match continuation.state {
            AgentContinuationState::Provider {
                model_user_message,
                model_user_content,
                tool_candidates,
                provider_tool_calls,
                mut provider_tool_results,
                mut pending_tool_calls,
                compacted_tool_history,
                current_round,
            } => {
                if is_legacy_budget_checkpoint {
                    if !approved {
                        events.push(AgentEventPayload::TurnFinished {
                            summary: "Legacy execution checkpoint left paused by the user."
                                .to_string(),
                        });
                        return Ok(AgentTurnResult {
                            events: events.into_vec(),
                            outcome: AgentTurnOutcome::Completed,
                        });
                    }

                    return self
                        .continue_provider_turn(
                            continuation.thread_id,
                            continuation.user_message_id,
                            continuation.workspace_root,
                            continuation.context_summary,
                            continuation.conversation,
                            continuation.permission_mode,
                            continuation.context_budget,
                            loop_policy,
                            loop_guard.clone(),
                            store,
                            cancellation,
                            model_user_message,
                            model_user_content,
                            tool_candidates,
                            provider_tool_calls,
                            provider_tool_results,
                            pending_tool_calls,
                            compacted_tool_history,
                            current_round,
                            &mut events,
                        )
                        .await;
                }

                let pending = pending_tool_calls
                    .first()
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("provider continuation has no pending call"))?;
                pending_tool_calls.remove(0);
                if approved {
                    let approved_sandbox = approved_sandbox_config_for_call(
                        &self.sandbox_config,
                        &continuation.workspace_root,
                        &pending,
                    );
                    let policy = Arc::new(BasicPolicyEngine::new_with_sandbox_config(
                        continuation.workspace_root.clone(),
                        continuation.permission_mode,
                        &approved_sandbox,
                    ));
                    let mut ctx = ToolContext::local_with_sandbox_config(
                        continuation.workspace_root.clone(),
                        policy,
                        approved_sandbox,
                    );
                    ctx.store = store.clone();
                    ctx.thread_id = Some(continuation.thread_id);
                    ctx.cancel = cancellation.clone();
                    ctx.approval_granted = true;
                    self.apply_subagent_context(&mut ctx, continuation.user_message_id);
                    let result = match self
                        .execute_provider_tool_call(&pending, ctx, &mut events)
                        .await
                    {
                        Ok(mut result) => {
                            if let Some(metadata) = result.metadata.as_object_mut() {
                                metadata.insert("approvalGranted".to_string(), json!(true));
                                metadata.insert("sandboxEscalation".to_string(), json!("scoped"));
                            }
                            result
                        }
                        Err(err) if err.to_string().contains("approval required") => {
                            let output = format!(
                                "The approved tool call remained blocked by the configured sandbox: {err}"
                            );
                            ProviderToolResult {
                                call_id: pending.id.clone(),
                                name: pending.name.clone(),
                                output: output.clone(),
                                content: vec![ModelContentPart::text(output)],
                                is_error: true,
                                metadata: json!({
                                    "approvalGranted": true,
                                    "sandboxEscalation": "denied",
                                    "sandboxEscalationDenied": true,
                                }),
                            }
                        }
                        Err(err) => return Err(err),
                    };
                    loop_guard.observe_tool_result(&pending, &result);
                    provider_tool_results.push(result);
                } else {
                    provider_tool_results.push(ProviderToolResult {
                        call_id: pending.id.clone(),
                        name: pending.name.clone(),
                        output: "The user denied this tool call.".to_string(),
                        content: vec![ModelContentPart::text("The user denied this tool call.")],
                        is_error: true,
                        metadata: json!({ "approvalDenied": true }),
                    });
                }

                let mut context_budget = continuation.context_budget;
                if let Some(ref mut budget) = context_budget {
                    if let Some(result) = provider_tool_results.last() {
                        budget.record_tokens(ContextBudget::estimate_tokens(&result.output));
                    }
                }

                self.continue_provider_turn(
                    continuation.thread_id,
                    continuation.user_message_id,
                    continuation.workspace_root,
                    continuation.context_summary,
                    continuation.conversation,
                    continuation.permission_mode,
                    context_budget,
                    loop_policy,
                    loop_guard,
                    store,
                    cancellation,
                    model_user_message,
                    model_user_content,
                    tool_candidates,
                    provider_tool_calls,
                    provider_tool_results,
                    pending_tool_calls,
                    compacted_tool_history,
                    current_round,
                    &mut events,
                )
                .await
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn continue_provider_turn(
        &self,
        thread_id: Uuid,
        user_message_id: Uuid,
        workspace_root: PathBuf,
        context_summary: Option<String>,
        mut conversation: Vec<ModelConversationMessage>,
        permission_mode: PermissionMode,
        mut budget: Option<ContextBudget>,
        loop_policy: AgentLoopPolicy,
        mut loop_guard: AgentLoopGuardState,
        store: Option<Arc<dyn SessionStore>>,
        cancellation: Option<CancellationToken>,
        model_user_message: String,
        model_user_content: Vec<ModelContentPart>,
        tool_candidates: Vec<ProviderToolCandidate>,
        mut provider_tool_calls: Vec<ProviderToolCall>,
        mut provider_tool_results: Vec<ProviderToolResult>,
        mut pending_tool_calls: Vec<ProviderToolCall>,
        mut compacted_tool_history: String,
        mut current_round: usize,
        events: &mut TurnEvents,
    ) -> anyhow::Result<AgentTurnResult> {
        loop {
            while let Some(provider_call) = pending_tool_calls.first().cloned() {
                if loop_guard.completion_mode
                    && !matches!(provider_call.name.as_str(), "update_plan" | "complete_task")
                {
                    loop_guard.blocked_completion_mode_calls += 1;
                    let result = blocked_completion_mode_tool_result(&provider_call, events);
                    if let Some(ref mut budget) = budget {
                        budget.record_tokens(ContextBudget::estimate_tokens(&result.output));
                    }
                    provider_tool_results.push(result);
                    pending_tool_calls.remove(0);
                    if loop_guard.blocked_completion_mode_calls >= MAX_COMPLETION_MODE_VIOLATIONS {
                        let output = fallback_verified_completion_output(
                            &mut loop_guard,
                            &model_user_message,
                            events,
                        )
                        .unwrap_or_else(|| {
                                "Terminal verification succeeded, but the provider did not submit a final task state. The turn was stopped after repeated completion-mode violations; the durable plan remains unchanged."
                                    .to_string()
                            });
                        return Ok(finalize_provider_turn(
                            thread_id,
                            ModelResponse::text(output),
                            provider_tool_results,
                            budget,
                            std::mem::replace(events, TurnEvents::new(None)),
                        ));
                    }
                    continue;
                }
                if let Some(violation) = loop_guard.register_tool_call(&provider_call, &loop_policy)
                {
                    let result =
                        blocked_loop_tool_result(&provider_call, &violation, &loop_policy, events);
                    if let Some(ref mut budget) = budget {
                        budget.record_tokens(ContextBudget::estimate_tokens(&result.output));
                    }
                    provider_tool_results.push(result);
                    pending_tool_calls.remove(0);
                    if loop_guard.loop_guard_corrections_since_progress
                        >= loop_policy.max_loop_guard_corrections
                    {
                        let output = loop_stagnation_output(&loop_guard, &violation);
                        return Ok(finalize_provider_turn(
                            thread_id,
                            ModelResponse::text(output),
                            provider_tool_results,
                            budget,
                            std::mem::replace(events, TurnEvents::new(None)),
                        ));
                    }
                    continue;
                }

                let policy = Arc::new(BasicPolicyEngine::new_with_sandbox_config(
                    workspace_root.clone(),
                    permission_mode,
                    &self.sandbox_config,
                ));
                let mut ctx = ToolContext::local_with_sandbox_config(
                    workspace_root.clone(),
                    policy,
                    self.sandbox_config.clone(),
                );
                ctx.store = store.clone();
                ctx.thread_id = Some(thread_id);
                ctx.cancel = cancellation.clone();
                self.apply_subagent_context(&mut ctx, user_message_id);
                match self
                    .execute_provider_tool_call(&provider_call, ctx, events)
                    .await
                {
                    Ok(result) => {
                        loop_guard.observe_tool_result(&provider_call, &result);
                        let completion_output = explicit_task_completion_output(&result)
                            .or_else(|| verified_plan_completion_output(&result, &loop_guard));
                        if let Some(ref mut budget) = budget {
                            budget.record_tokens(ContextBudget::estimate_tokens(&result.output));
                        }
                        provider_tool_results.push(result);
                        pending_tool_calls.remove(0);
                        if let Some(output) = completion_output {
                            return Ok(finalize_provider_turn(
                                thread_id,
                                ModelResponse::text(output),
                                provider_tool_results,
                                budget,
                                std::mem::replace(events, TurnEvents::new(None)),
                            ));
                        }
                    }
                    Err(err) if err.to_string().contains("approval required") => {
                        let reason = err.to_string();
                        let approval_id = Uuid::new_v4();
                        events.push(AgentEventPayload::ApprovalRequested {
                            approval_id,
                            reason: reason.clone(),
                            action: provider_tool_approval_action(&provider_call),
                        });
                        events.push(AgentEventPayload::TurnSuspended {
                            approval_id,
                            reason,
                        });
                        return Ok(AgentTurnResult {
                            events: std::mem::replace(events, TurnEvents::new(None)).into_vec(),
                            outcome: AgentTurnOutcome::Suspended {
                                approval_id,
                                continuation: AgentContinuation {
                                    thread_id,
                                    user_message_id,
                                    workspace_root,
                                    context_summary,
                                    conversation,
                                    permission_mode,
                                    context_budget: budget,
                                    loop_policy: loop_policy.clone(),
                                    loop_guard,
                                    continuation_kind: AgentContinuationKind::Approval,
                                    state: AgentContinuationState::Provider {
                                        model_user_message,
                                        model_user_content,
                                        tool_candidates,
                                        provider_tool_calls,
                                        provider_tool_results,
                                        pending_tool_calls,
                                        compacted_tool_history,
                                        current_round,
                                    },
                                },
                            },
                        });
                    }
                    Err(err) => return Err(err),
                }
            }

            compact_completed_tool_history(
                &mut conversation,
                &mut provider_tool_calls,
                &mut provider_tool_results,
                &mut compacted_tool_history,
                &mut budget,
            );
            let response = self
                .complete_model(
                    ModelRequest {
                        system_prompt: provider_system_prompt(
                            &loop_policy.status(
                                loop_guard.total_tool_rounds,
                                loop_guard.loop_guard_corrections_since_progress,
                                budget.as_ref(),
                            ),
                            &workspace_root,
                            &self.sandbox_config,
                            loop_guard.completion_mode,
                        ),
                        conversation: conversation.clone(),
                        user_message: model_user_message.clone(),
                        user_content: model_user_content.clone(),
                        tool_candidates: if loop_guard.completion_mode {
                            tool_candidates
                                .iter()
                                .filter(|candidate| {
                                    matches!(
                                        candidate.name.as_str(),
                                        "update_plan" | "complete_task"
                                    )
                                })
                                .cloned()
                                .collect()
                        } else {
                            tool_candidates.clone()
                        },
                        previous_tool_calls: provider_tool_calls.clone(),
                        tool_results: provider_tool_results.clone(),
                    },
                    events,
                )
                .await?;
            if let Some(ref mut budget) = budget {
                budget.record_tokens(ContextBudget::estimate_tokens(&response.text));
            }

            if response.tool_calls.is_empty() {
                return Ok(finalize_provider_turn(
                    thread_id,
                    response,
                    provider_tool_results,
                    budget,
                    std::mem::replace(events, TurnEvents::new(None)),
                ));
            }

            current_round += 1;
            loop_guard.total_tool_rounds += 1;
            pending_tool_calls = response.tool_calls;
            provider_tool_calls.extend(pending_tool_calls.clone());
            if let Some(ref mut budget) = budget {
                budget.record_tokens(0);
            }
        }
    }

    async fn complete_model(
        &self,
        request: ModelRequest,
        events: &mut TurnEvents,
    ) -> anyhow::Result<ModelResponse> {
        let round = events
            .items
            .iter()
            .filter(|event| matches!(event, AgentEventPayload::ModelRequest { .. }))
            .count()
            + 1;
        let request_snapshot = serde_json::to_value(&request)
            .unwrap_or_else(|error| json!({ "serializationError": error.to_string() }));
        events.push(AgentEventPayload::ModelRequest {
            round,
            request: request_snapshot,
        });
        let mut on_delta = |delta| {
            match delta {
                ModelStreamDelta::Text { text } => {
                    events.push(AgentEventPayload::ModelDelta { text });
                }
                ModelStreamDelta::Reasoning { text } => {
                    events.push(AgentEventPayload::ReasoningDelta { text });
                }
                ModelStreamDelta::Usage { usage } => {
                    events.push(AgentEventPayload::TokenUsage {
                        input_tokens: usage.input_tokens as usize,
                        output_tokens: usage.output_tokens as usize,
                        total_tokens: usage.total_tokens as usize,
                    });
                }
                ModelStreamDelta::ToolCall { .. } => {}
            }
            Ok(())
        };
        self.provider.stream(request, &mut on_delta).await
    }

    fn provider_tool_candidates(&self) -> Vec<ProviderToolCandidate> {
        let subagents_available = self.subagents.is_some();
        self.tools
            .list()
            .into_iter()
            .filter(|name| subagents_available || !is_subagent_tool(name))
            .filter_map(|name| {
                self.tools.get(&name).map(|tool| ProviderToolCandidate {
                    name,
                    description: tool.description().to_string(),
                    input_schema: tool.schema(),
                })
            })
            .collect()
    }

    async fn execute_provider_tool_call(
        &self,
        provider_call: &ProviderToolCall,
        ctx: ToolContext,
        events: &mut TurnEvents,
    ) -> anyhow::Result<ProviderToolResult> {
        let call = ToolCall::new(&provider_call.name, provider_call.arguments.clone());
        let result = self
            .execute_tool_call(
                call,
                ctx,
                events,
                Some(json!({ "providerToolCallId": &provider_call.id })),
            )
            .await;

        match result {
            Ok(result) => {
                let is_error = tool_result_is_error(&result);
                let content = result.content_or_legacy_text();
                Ok(ProviderToolResult {
                    call_id: provider_call.id.clone(),
                    name: provider_call.name.clone(),
                    output: result.output,
                    content,
                    is_error,
                    metadata: result.metadata,
                })
            }
            Err(err) if err.to_string().contains("approval required") => Err(err),
            Err(err) if err.to_string().contains("cancelled") => Err(err),
            Err(err) => Ok(ProviderToolResult {
                call_id: provider_call.id.clone(),
                name: provider_call.name.clone(),
                output: err.to_string(),
                content: vec![ModelContentPart::text(err.to_string())],
                is_error: true,
                metadata: json!({
                    "toolName": &provider_call.name,
                    "providerToolCallId": &provider_call.id,
                    "success": false,
                    "error": err.to_string()
                }),
            }),
        }
    }

    async fn execute_tool_call(
        &self,
        call: ToolCall,
        ctx: ToolContext,
        events: &mut TurnEvents,
        metadata_overlay: Option<Value>,
    ) -> anyhow::Result<crate::model::ToolResult> {
        let name = call.name.clone();
        let approval_granted = ctx.approval_granted;
        events.push(AgentEventPayload::ToolCallStarted { call: call.clone() });
        let tool = match self.tools.get(&name) {
            Some(tool) => tool,
            None => {
                let err = anyhow::anyhow!("{} tool not registered", name);
                let mut metadata = json!({
                    "toolName": &name,
                    "success": false,
                    "error": err.to_string()
                });
                insert_approval_execution_metadata(&mut metadata, approval_granted, Some(&err));
                merge_metadata_overlay(&mut metadata, metadata_overlay.as_ref());
                events.push(AgentEventPayload::ToolCallFinished {
                    result: ToolResult {
                        call_id: call.id,
                        output: err.to_string(),
                        content: vec![ModelContentPart::text(err.to_string())],
                        metadata,
                    },
                });
                return Err(err);
            }
        };
        let mut result = match tool.execute(call.clone(), ctx).await {
            Ok(result) => result,
            Err(err) => {
                let mut metadata = json!({
                    "toolName": &name,
                    "success": false,
                    "error": err.to_string()
                });
                insert_approval_execution_metadata(&mut metadata, approval_granted, Some(&err));
                merge_metadata_overlay(&mut metadata, metadata_overlay.as_ref());
                events.push(AgentEventPayload::ToolCallFinished {
                    result: ToolResult {
                        call_id: call.id,
                        output: err.to_string(),
                        content: vec![ModelContentPart::text(err.to_string())],
                        metadata,
                    },
                });
                return Err(err);
            }
        };
        if let Some(object) = result.metadata.as_object_mut() {
            object.insert("toolName".to_string(), json!(&name));
        }
        insert_approval_execution_metadata(&mut result.metadata, approval_granted, None);
        merge_metadata_overlay(&mut result.metadata, metadata_overlay.as_ref());
        events.push(AgentEventPayload::ToolCallFinished {
            result: result.clone(),
        });
        if name == "update_plan" {
            if let Some(value) = result.metadata.get("taskPlan") {
                if let Ok(plan) = serde_json::from_value::<TaskPlan>(value.clone()) {
                    events.push(AgentEventPayload::PlanUpdated { plan });
                }
            }
        }
        Ok(result)
    }
}

fn insert_approval_execution_metadata(
    metadata: &mut Value,
    approval_granted: bool,
    error: Option<&anyhow::Error>,
) {
    if !approval_granted {
        return;
    }
    let denied = error.is_some_and(|error| error.to_string().contains("approval required"));
    if let Some(object) = metadata.as_object_mut() {
        object.insert("approvalGranted".to_string(), json!(true));
        object.insert(
            "sandboxEscalation".to_string(),
            json!(if denied { "denied" } else { "scoped" }),
        );
        if denied {
            object.insert("sandboxEscalationDenied".to_string(), json!(true));
        }
    }
}

fn finalize_provider_turn(
    thread_id: Uuid,
    response: ModelResponse,
    provider_tool_results: Vec<ProviderToolResult>,
    mut budget: Option<ContextBudget>,
    mut events: TurnEvents,
) -> AgentTurnResult {
    if let Some(ref mut budget) = budget {
        for warning in &budget.warnings {
            events.push(AgentEventPayload::ModelDelta {
                text: format!("**Context budget warning:** {}\n", warning),
            });
        }
    }

    let text = if response.text.trim().is_empty() {
        local_provider_tool_summary(&provider_tool_results)
    } else {
        response.text
    };
    let assistant_message = Message::text(thread_id, MessageRole::Assistant, text);
    events.push(AgentEventPayload::AssistantMessage {
        message: assistant_message,
    });
    events.push(AgentEventPayload::TurnFinished {
        summary: if provider_tool_results.is_empty() {
            "Provider agent turn completed.".to_string()
        } else {
            "Provider tool loop completed.".to_string()
        },
    });
    AgentTurnResult {
        events: events.into_vec(),
        outcome: AgentTurnOutcome::Completed,
    }
}

impl AgentLoopGuardState {
    fn register_tool_call(
        &mut self,
        call: &ProviderToolCall,
        policy: &AgentLoopPolicy,
    ) -> Option<ToolLoopViolation> {
        if call.name == "complete_task" {
            return None;
        }
        let signature = provider_tool_call_signature(call);
        self.recent_tool_signatures.push(signature.clone());
        if self.recent_tool_signatures.len() > MAX_TRACKED_TOOL_SIGNATURES {
            self.recent_tool_signatures.remove(0);
        }

        let count = self
            .equivalent_call_counts
            .entry(signature.clone())
            .or_default();
        *count += 1;
        if let Some((cycle_length, repetitions)) = repeated_tool_cycle(
            &self.recent_tool_signatures,
            policy.max_repeated_tool_cycles,
        ) {
            self.loop_guard_corrections_since_progress += 1;
            return Some(ToolLoopViolation::RepeatedCycle {
                cycle_length,
                repetitions,
            });
        }
        if *count > policy.max_equivalent_tool_calls {
            self.blocked_equivalent_calls += 1;
            self.loop_guard_corrections_since_progress += 1;
            return Some(ToolLoopViolation::EquivalentCall { count: *count });
        }
        None
    }

    fn observe_tool_result(&mut self, call: &ProviderToolCall, result: &ProviderToolResult) {
        let workspace_mutation_succeeded = !result.is_error && tool_call_can_change_workspace(call);
        if result.is_error {
            return;
        }

        let signature = provider_tool_call_signature(call);
        let result_fingerprint = provider_tool_result_fingerprint(result);
        let observation_changed = self
            .last_result_fingerprints
            .get(&signature)
            .is_some_and(|previous| previous != &result_fingerprint);
        self.last_result_fingerprints
            .insert(signature.clone(), result_fingerprint);
        let verification_command = successful_verification_command(call);
        if let Some(command) = verification_command.as_ref() {
            self.last_successful_verification = Some(command.clone());
        }

        let plan_progress = if call.name == "update_plan" {
            let next_plan = result
                .metadata
                .get("taskPlan")
                .and_then(|value| serde_json::from_value(value.clone()).ok());
            let fingerprint = result.metadata.get("taskPlan").map(canonical_json_string);
            let changed = fingerprint != self.last_plan_fingerprint
                && plan_made_progress(self.latest_plan.as_ref(), next_plan.as_ref());
            self.latest_plan = next_plan;
            self.last_plan_fingerprint = fingerprint;
            changed
        } else {
            false
        };
        let progress_tool_advanced =
            if workspace_mutation_succeeded || verification_command.is_some() {
                let count = self
                    .progress_tool_signatures
                    .entry(signature.clone())
                    .or_default();
                *count += 1;
                *count == 1
            } else {
                false
            };
        let changed_state = plan_progress || progress_tool_advanced || observation_changed;

        if changed_state {
            self.equivalent_call_counts.clear();
            self.equivalent_call_counts.insert(signature.clone(), 1);
            self.recent_tool_signatures.clear();
            self.recent_tool_signatures.push(signature);
            self.loop_guard_corrections_since_progress = 0;
            if workspace_mutation_succeeded && progress_tool_advanced {
                self.last_successful_verification = None;
                self.workspace_changed_since_verification = true;
                self.completion_mode = false;
            }
        }
        if verification_command.is_some()
            && self.workspace_changed_since_verification
            && self.latest_plan.as_ref().is_some_and(|plan| {
                plan.steps
                    .iter()
                    .any(|step| step_looks_like_verification(&step.step))
            })
        {
            self.completion_mode = true;
            self.blocked_completion_mode_calls = 0;
            self.workspace_changed_since_verification = false;
        }
    }
}

#[derive(Debug, Clone)]
enum ToolLoopViolation {
    EquivalentCall {
        count: usize,
    },
    RepeatedCycle {
        cycle_length: usize,
        repetitions: usize,
    },
}

fn repeated_tool_cycle(
    signatures: &[String],
    required_repetitions: usize,
) -> Option<(usize, usize)> {
    let max_cycle_length = MAX_DETECTED_TOOL_CYCLE_LENGTH.min(
        signatures
            .len()
            .checked_div(required_repetitions)
            .unwrap_or_default(),
    );
    for cycle_length in 2..=max_cycle_length {
        let compared = cycle_length * required_repetitions;
        let suffix = &signatures[signatures.len() - compared..];
        let cycle = &suffix[..cycle_length];
        let is_multi_step = cycle.iter().skip(1).any(|signature| signature != &cycle[0]);
        if is_multi_step
            && suffix
                .chunks_exact(cycle_length)
                .all(|chunk| chunk == cycle)
        {
            return Some((cycle_length, required_repetitions));
        }
    }
    None
}

fn plan_made_progress(previous: Option<&TaskPlan>, next: Option<&TaskPlan>) -> bool {
    let Some(next) = next else {
        return false;
    };
    let Some(previous) = previous else {
        return !next.steps.is_empty();
    };
    let previous_completed = previous
        .steps
        .iter()
        .filter(|step| step.status == TaskPlanStepStatus::Completed)
        .count();
    let next_completed = next
        .steps
        .iter()
        .filter(|step| step.status == TaskPlanStepStatus::Completed)
        .count();
    if next_completed > previous_completed {
        return true;
    }

    next.steps.iter().any(|next_step| {
        next_step.status == TaskPlanStepStatus::InProgress
            && previous.steps.iter().all(|previous_step| {
                previous_step.step != next_step.step
                    || previous_step.status == TaskPlanStepStatus::Pending
            })
    })
}

fn provider_tool_call_signature(call: &ProviderToolCall) -> String {
    format!("{}:{}", call.name, canonical_json_string(&call.arguments))
}

fn provider_tool_result_fingerprint(result: &ProviderToolResult) -> String {
    let stable_output = serde_json::from_str::<Value>(&result.output)
        .ok()
        .map(|value| canonical_json_string(&stable_result_value(&value)))
        .unwrap_or_else(|| result.output.clone());
    let output = if stable_output.chars().count() <= 1_024 {
        stable_output
    } else {
        let char_count = stable_output.chars().count();
        let prefix = stable_output.chars().take(512).collect::<String>();
        let suffix = stable_output
            .chars()
            .rev()
            .take(512)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>();
        format!("{prefix}[...{char_count} chars...]{suffix}")
    };
    format!(
        "{}:{}:{}",
        result.is_error,
        canonical_json_string(&stable_result_value(&result.metadata)),
        output
    )
}

fn stable_result_value(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(stable_result_value).collect()),
        Value::Object(values) => Value::Object(
            values
                .iter()
                .filter(|(key, _)| {
                    !matches!(
                        key.as_str(),
                        "providerToolCallId"
                            | "callId"
                            | "requestId"
                            | "durationMs"
                            | "elapsedMs"
                            | "startedAt"
                            | "finishedAt"
                            | "updatedAt"
                            | "timestamp"
                    )
                })
                .map(|(key, value)| (key.clone(), stable_result_value(value)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn canonical_json_string(value: &Value) -> String {
    fn canonicalize(value: &Value) -> Value {
        match value {
            Value::Array(values) => Value::Array(values.iter().map(canonicalize).collect()),
            Value::Object(values) => {
                let sorted = values
                    .iter()
                    .map(|(key, value)| (key.clone(), canonicalize(value)))
                    .collect::<BTreeMap<_, _>>();
                serde_json::to_value(sorted).unwrap_or(Value::Null)
            }
            _ => value.clone(),
        }
    }

    serde_json::to_string(&canonicalize(value)).unwrap_or_else(|_| "null".to_string())
}

fn explicit_task_completion_output(result: &ProviderToolResult) -> Option<String> {
    (!result.is_error && result.metadata.get("taskCompletion").is_some())
        .then(|| result.output.clone())
}

fn successful_verification_command(call: &ProviderToolCall) -> Option<String> {
    if call.name != "shell" {
        return None;
    }
    let command = call.arguments.get("command")?.as_str()?.trim();
    let normalized = command.to_lowercase();
    [
        "npm test",
        "pnpm test",
        "yarn test",
        "cargo test",
        "pytest",
        "node --test",
        "node test/",
        "node test\\",
        "npm run build",
        "npm run check",
        "npm run lint",
        "pnpm build",
        "pnpm check",
        "pnpm lint",
        "cargo check",
    ]
    .iter()
    .any(|pattern| normalized.contains(pattern))
    .then(|| command.to_string())
}

fn step_looks_like_verification(step: &str) -> bool {
    let step = step.to_lowercase();
    [
        "test", "verify", "check", "lint", "build", "测试", "验证", "检查", "构建",
    ]
    .iter()
    .any(|keyword| step.contains(keyword))
}

#[cfg(test)]
fn step_looks_like_workspace_change(step: &str) -> bool {
    let step = step.to_lowercase();
    [
        "implement",
        "fix",
        "repair",
        "create",
        "write",
        "modify",
        "update",
        "add",
        "remove",
        "migrate",
        "refactor",
        "cli",
        "实现",
        "修复",
        "创建",
        "写入",
        "修改",
        "更新",
        "添加",
        "删除",
        "迁移",
        "重构",
    ]
    .iter()
    .any(|keyword| step.contains(keyword))
}

fn tool_call_can_change_workspace(call: &ProviderToolCall) -> bool {
    match call.name.as_str() {
        "write_file" | "apply_patch" => true,
        "spreadsheet" => call.arguments.get("action").and_then(Value::as_str) == Some("write"),
        _ => false,
    }
}

fn verified_plan_completion_output(
    result: &ProviderToolResult,
    loop_guard: &AgentLoopGuardState,
) -> Option<String> {
    if result.name != "update_plan" || result.is_error {
        return None;
    }
    let plan: TaskPlan = serde_json::from_value(result.metadata.get("taskPlan")?.clone()).ok()?;
    let completed = plan
        .steps
        .iter()
        .filter(|step| step.status == TaskPlanStepStatus::Completed)
        .count();
    let pending = plan
        .steps
        .iter()
        .filter(|step| step.status == TaskPlanStepStatus::Pending)
        .count();
    let has_in_progress = plan
        .steps
        .iter()
        .any(|step| step.status == TaskPlanStepStatus::InProgress);
    let all_steps_complete = !plan.steps.is_empty() && completed == plan.steps.len();
    let current_scope_complete = result
        .metadata
        .get("currentScopeComplete")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let verification = loop_guard.last_successful_verification.as_deref()?;
    if has_in_progress || completed == 0 || (!all_steps_complete && !current_scope_complete) {
        return None;
    }

    let mut output = format!(
        "Current requested scope completed with {completed}/{} plan steps completed.\n\nVerification:\n- {verification}",
        plan.steps.len()
    );
    if pending > 0 {
        output.push_str(&format!(
            "\n\nRemaining work:\n- {pending} plan step(s) explicitly deferred to a later phase."
        ));
    }
    Some(output)
}

fn blocked_loop_tool_result(
    provider_call: &ProviderToolCall,
    violation: &ToolLoopViolation,
    policy: &AgentLoopPolicy,
    events: &mut TurnEvents,
) -> ProviderToolResult {
    let call = ToolCall::new(&provider_call.name, provider_call.arguments.clone());
    events.push(AgentEventPayload::ToolCallStarted { call: call.clone() });
    let (output, violation_metadata) = match violation {
        ToolLoopViolation::EquivalentCall { count } => (
            format!(
                "Equivalent tool call blocked after {} executions without meaningful progress. Do not retry the same operation. Change the approach, use new evidence, make a concrete state change, or finish truthfully.",
                policy.max_equivalent_tool_calls
            ),
            json!({
                "kind": "equivalent_call",
                "equivalentCallCount": count,
                "maxEquivalentToolCalls": policy.max_equivalent_tool_calls
            }),
        ),
        ToolLoopViolation::RepeatedCycle {
            cycle_length,
            repetitions,
        } => (
            format!(
                "Repeated tool-call cycle blocked after {repetitions} repetitions of a {cycle_length}-step pattern without meaningful progress. Break the cycle with a different approach or finish truthfully."
            ),
            json!({
                "kind": "repeated_cycle",
                "cycleLength": cycle_length,
                "cycleRepetitions": repetitions,
                "maxRepeatedToolCycles": policy.max_repeated_tool_cycles
            }),
        ),
    };
    let mut metadata = json!({
        "toolName": &provider_call.name,
        "providerToolCallId": &provider_call.id,
        "success": false,
        "loopGuardBlocked": true,
        "loopGuardCorrectionLimit": policy.max_loop_guard_corrections
    });
    if let (Some(target), Some(source)) = (metadata.as_object_mut(), violation_metadata.as_object())
    {
        target.extend(source.clone());
    }
    events.push(AgentEventPayload::ToolCallFinished {
        result: ToolResult {
            call_id: call.id,
            output: output.clone(),
            content: vec![ModelContentPart::text(output.clone())],
            metadata: metadata.clone(),
        },
    });
    ProviderToolResult {
        call_id: provider_call.id.clone(),
        name: provider_call.name.clone(),
        output: output.clone(),
        content: vec![ModelContentPart::text(output)],
        is_error: true,
        metadata,
    }
}

fn blocked_completion_mode_tool_result(
    provider_call: &ProviderToolCall,
    events: &mut TurnEvents,
) -> ProviderToolResult {
    let call = ToolCall::new(&provider_call.name, provider_call.arguments.clone());
    events.push(AgentEventPayload::ToolCallStarted { call: call.clone() });
    let output = "Tool call blocked because terminal verification already succeeded. Completion mode only permits update_plan or complete_task. Submit the truthful final state now.".to_string();
    let metadata = json!({
        "toolName": &provider_call.name,
        "providerToolCallId": &provider_call.id,
        "success": false,
        "completionModeBlocked": true
    });
    events.push(AgentEventPayload::ToolCallFinished {
        result: ToolResult {
            call_id: call.id,
            output: output.clone(),
            metadata: metadata.clone(),
            content: vec![ModelContentPart::text(output.clone())],
        },
    });
    ProviderToolResult {
        call_id: provider_call.id.clone(),
        name: provider_call.name.clone(),
        output: output.clone(),
        content: vec![ModelContentPart::text(output)],
        metadata,
        is_error: true,
    }
}

fn fallback_verified_completion_output(
    loop_guard: &mut AgentLoopGuardState,
    model_user_message: &str,
    events: &mut TurnEvents,
) -> Option<String> {
    let verification = loop_guard.last_successful_verification.clone()?;
    let mut plan = loop_guard.latest_plan.clone()?;
    let resumes_deferred_work = request_resumes_deferred_work(model_user_message);
    let mut completed = 0;
    let mut deferred = 0;
    for step in &mut plan.steps {
        if step.status != TaskPlanStepStatus::Completed {
            if !resumes_deferred_work && step_is_explicitly_deferred(&step.step) {
                step.status = TaskPlanStepStatus::Pending;
                deferred += 1;
            } else {
                step.status = TaskPlanStepStatus::Completed;
            }
        }
        if step.status == TaskPlanStepStatus::Completed {
            completed += 1;
        }
    }
    plan.explanation = Some(
        "Runtime fallback reconciled the durable plan after successful verification and repeated completion-mode violations."
            .to_string(),
    );
    loop_guard.latest_plan = Some(plan.clone());
    events.push(AgentEventPayload::PlanUpdated { plan: plan.clone() });

    let mut output = format!(
        "Current requested scope closed by the runtime fallback with {completed}/{} plan steps completed after the provider repeatedly violated completion mode.\n\nVerification:\n- {verification}",
        plan.steps.len()
    );
    if deferred > 0 {
        output.push_str(&format!(
            "\n\nRemaining work:\n- {deferred} plan step(s) remain explicitly deferred."
        ));
    }
    Some(output)
}

fn request_resumes_deferred_work(request: &str) -> bool {
    let request = request.to_lowercase();
    [
        "continue",
        "resume",
        "recover",
        "remaining work",
        "phase 2",
        "phase-2",
        "session 2",
        "session-2",
        "继续",
        "恢复",
        "剩余",
        "第二阶段",
    ]
    .iter()
    .any(|keyword| request.contains(keyword))
}

fn step_is_explicitly_deferred(step: &str) -> bool {
    let step = step.to_lowercase();
    [
        "session 2",
        "session-2",
        "phase 2",
        "phase-2",
        "later session",
        "later phase",
        "future work",
        "deferred",
        "out of scope",
        "follow-up",
        "第二阶段",
        "稍后",
        "后续",
        "延期",
        "暂缓",
        "不在范围",
    ]
    .iter()
    .any(|keyword| step.contains(keyword))
}

fn loop_stagnation_output(
    loop_guard: &AgentLoopGuardState,
    violation: &ToolLoopViolation,
) -> String {
    let plan = loop_guard.latest_plan.as_ref();
    let remaining = plan
        .map(|plan| {
            plan.steps
                .iter()
                .filter(|step| step.status != TaskPlanStepStatus::Completed)
                .count()
        })
        .unwrap_or(0);
    let pattern = match violation {
        ToolLoopViolation::EquivalentCall { .. } => "the same tool call",
        ToolLoopViolation::RepeatedCycle { .. } => "the same tool-call cycle",
    };
    format!(
        "Task remains incomplete. The runtime stopped {pattern} after {} loop-guard corrections produced no meaningful progress. There is no elapsed-time or total-round limit; execution stopped only because the model kept returning to an equivalent problem state. The durable plan retains {remaining} unfinished step(s) for a retry or user-directed continuation.",
        loop_guard.loop_guard_corrections_since_progress
    )
}

fn compact_completed_tool_history(
    conversation: &mut Vec<ModelConversationMessage>,
    provider_tool_calls: &mut Vec<ProviderToolCall>,
    provider_tool_results: &mut Vec<ProviderToolResult>,
    compacted_tool_history: &mut String,
    budget: &mut Option<ContextBudget>,
) {
    const COMPACTION_MARKER: &str = "[Automatically compacted tool history]";
    let Some(context_budget) = budget.as_mut() else {
        return;
    };
    if context_budget.used_tokens.saturating_mul(100) < context_budget.max_tokens.saturating_mul(80)
    {
        return;
    }

    let target_tokens = context_budget.max_tokens.saturating_mul(65) / 100;
    let mut dropped_tokens = 0usize;
    let mut summary_lines = Vec::new();
    while context_budget.used_tokens.saturating_sub(dropped_tokens) > target_tokens
        && provider_tool_results.len() > MIN_RETAINED_TOOL_RESULTS_AFTER_COMPACTION
    {
        let result = provider_tool_results.remove(0);
        let call = provider_tool_calls
            .iter()
            .position(|call| call.id == result.call_id)
            .map(|index| provider_tool_calls.remove(index));
        dropped_tokens =
            dropped_tokens.saturating_add(ContextBudget::estimate_tokens(&result.output));
        let arguments = call
            .as_ref()
            .map(|call| truncate_for_summary(&canonical_json_string(&call.arguments), 240))
            .unwrap_or_else(|| "{}".to_string());
        summary_lines.push(format!(
            "- {} {}: {}\n  {}",
            result.name,
            arguments,
            if result.is_error {
                "failed"
            } else {
                "succeeded"
            },
            truncate_for_summary(&result.output, 480).replace('\n', " ")
        ));
    }
    if summary_lines.is_empty() {
        return;
    }

    let old_summary_tokens = ContextBudget::estimate_tokens(compacted_tool_history);
    if !compacted_tool_history.is_empty() {
        compacted_tool_history.push('\n');
    }
    compacted_tool_history.push_str(&summary_lines.join("\n"));
    let summary_char_limit = context_budget
        .max_tokens
        .saturating_mul(4)
        .saturating_div(5)
        .min(MAX_COMPACTED_TOOL_HISTORY_CHARS);
    *compacted_tool_history = truncate_for_summary(compacted_tool_history, summary_char_limit);
    let summary_content = format!(
        "{COMPACTION_MARKER}\nEarlier completed tool calls were compacted automatically to keep the long-running turn inside the model context window. Treat these records as durable observations; do not repeat them unless later state makes them stale.\n{}",
        compacted_tool_history
    );
    if let Some(message) = conversation
        .iter_mut()
        .find(|message| message.content.starts_with(COMPACTION_MARKER))
    {
        message.content = summary_content;
        message.content_parts.clear();
    } else {
        conversation.push(ModelConversationMessage {
            role: ModelConversationRole::System,
            content: summary_content,
            content_parts: Vec::new(),
        });
    }

    let new_summary_tokens = ContextBudget::estimate_tokens(compacted_tool_history);
    context_budget.used_tokens = context_budget
        .used_tokens
        .saturating_sub(dropped_tokens)
        .saturating_sub(old_summary_tokens)
        .saturating_add(new_summary_tokens);
    context_budget.warnings.clear();
}

fn provider_system_prompt(
    status: &AgentLoopStatus,
    workspace_root: &Path,
    sandbox_config: &LocalSandboxConfig,
    completion_mode: bool,
) -> String {
    let completion_instruction = if completion_mode {
        " A terminal verification command just succeeded. Completion mode is active: only update_plan and complete_task are available. Do not resume implementation or investigation. Submit the truthful final plan state now, or use complete_task if the requested scope is actually complete."
    } else {
        ""
    };
    let workspace_scope = workspace_scope_instruction(workspace_root, sandbox_config);
    format!(
        "You are OpenTopia, a tool-using AI agent. Decide for yourself whether to observe, which available tools to call, how to validate their results, and when the task is complete. The harness provides capabilities, policy boundaries, isolation, and observability; it does not prescribe a workflow. Use tools only when they materially help. {workspace_scope} For non-trivial multi-step work, use update_plan as durable task memory and keep it current. Complete every step in the current requested scope before claiming completion; steps explicitly deferred by the user to a later phase may remain pending. After a successful test, build, lint, check, or verify command, finish with one final action: update_plan with all current steps completed (set current_scope_complete with verification when later-phase steps remain pending), or call complete_task with a concise summary, concrete verification evidence, and any deliberately deferred work. Do not perform more investigation after that final action. Delegate independent work with spawn_agent when useful, then use wait_agents to collect parallel results before synthesizing the final answer. Inspect every child status and error; a terminal child is not necessarily a successful child.{completion_instruction}\n\nThis turn has no elapsed-time or total tool-round limit. It has completed {} tool-decision rounds so far. Loop protection is progress-based: an equivalent call may repeat up to {} times, a repeated multi-step cycle is detected after {} repetitions, and {} ignored loop corrections without meaningful progress stop only that repeated pattern. Current loop corrections since progress: {}. Tool history is compacted automatically near the context-window boundary. Context budget: {} used of {} tokens ({} remaining).",
        status.total_tool_rounds,
        status.max_equivalent_tool_calls,
        status.max_repeated_tool_cycles,
        status.max_loop_guard_corrections,
        status.loop_guard_corrections,
        status.context_used_tokens.map(|tokens| tokens.to_string()).unwrap_or_else(|| "not tracked".to_string()),
        status.context_max_tokens.map(|tokens| tokens.to_string()).unwrap_or_else(|| "not tracked".to_string()),
        status.context_remaining_tokens.map(|tokens| tokens.to_string()).unwrap_or_else(|| "not tracked".to_string()),
    )
}

fn workspace_scope_instruction(
    workspace_root: &Path,
    sandbox_config: &LocalSandboxConfig,
) -> String {
    let workspace_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let additional_roots = sandbox_config
        .effective_readable_roots(&workspace_root)
        .into_iter()
        .filter(|root| root != &workspace_root)
        .map(|root| root.display().to_string())
        .collect::<Vec<_>>();
    let additional_roots = if additional_roots.is_empty() {
        "none".to_string()
    } else {
        additional_roots.join(", ")
    };
    let full_access_note = if sandbox_config.sandbox_mode == SandboxMode::DangerFullAccess {
        " Full-access capability is not an instruction to explore outside the workspace."
    } else {
        ""
    };
    format!(
        "The thread workspace root is '{}'. Resolve every relative file path and shell working directory against this root; the default shell working directory is this root. Begin with the workspace and complete the task there whenever it contains enough information. Do not list, search, read, or probe parent directories or unrelated absolute paths for context. Access outside the workspace only when the user explicitly requests it or the path is an additional configured readable root. Configured additional readable roots: {additional_roots}.{full_access_note}",
        workspace_root.display()
    )
}

fn is_subagent_tool(name: &str) -> bool {
    matches!(
        name,
        "spawn_agent" | "send_input" | "cancel_agent" | "wait_agent" | "wait_agents"
    )
}

fn provider_user_message(user_content: &str, context_summary: Option<&str>) -> String {
    let durable_context = context_summary
        .map(|summary| format!("Durable context from earlier turns:\n{summary}\n\n"))
        .unwrap_or_default();
    format!("{durable_context}User request:\n{user_content}")
}

fn provider_tool_approval_action(call: &ProviderToolCall) -> String {
    match call.name.as_str() {
        "list_files" => format!(
            "/list {}",
            call.arguments
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or(".")
        ),
        "read_file" => format!(
            "/read {}",
            call.arguments
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("")
        ),
        "search" => {
            let path = call
                .arguments
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or(".");
            let query = call
                .arguments
                .get("query")
                .and_then(Value::as_str)
                .unwrap_or("");
            format!("/search {} -- {}", path, query)
        }
        "write_file" => {
            let path = call
                .arguments
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("");
            let content = call
                .arguments
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or("");
            format!("/write {}\n{}", path, content)
        }
        "shell" => format!(
            "/run {}",
            call.arguments
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("")
        ),
        "git_diff" => "/diff".to_string(),
        "apply_patch" => format!(
            "/patch {}",
            call.arguments
                .get("patch")
                .and_then(Value::as_str)
                .unwrap_or("")
        ),
        "browser" => call
            .arguments
            .get("url")
            .and_then(Value::as_str)
            .and_then(|url| browser_domain_from_url(url).ok())
            .map(|host| browser_domain_approval_action(&host))
            .unwrap_or_else(|| format!("browser {}", call.arguments)),
        _ => format!("/mcp {} {}", call.name, call.arguments),
    }
}

fn approved_sandbox_config_for_call(
    base: &LocalSandboxConfig,
    workspace_root: &Path,
    call: &ProviderToolCall,
) -> LocalSandboxConfig {
    let mut config = base.clone();
    let path = |key: &str| {
        call.arguments
            .get(key)
            .and_then(Value::as_str)
            .map(|value| approval_path(workspace_root, value))
    };

    match call.name.as_str() {
        "list_files" | "read_file" | "search" => {
            if let Some(path) = path("path") {
                config.grant_read_path(path);
            }
        }
        "write_file" => {
            if let Some(path) = path("path") {
                config.grant_write_path(path);
            }
        }
        "spreadsheet" => {
            if let Some(path) = path("path").or_else(|| path("sourcePath")) {
                config.grant_read_path(path);
            }
            if let Some(path) = path("outputPath") {
                config.grant_write_path(path);
            }
        }
        _ => {}
    }

    config
}

fn approval_path(workspace_root: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        workspace_root.join(path)
    }
}

fn tool_result_is_error(result: &ToolResult) -> bool {
    result
        .metadata
        .get("success")
        .and_then(Value::as_bool)
        .is_some_and(|success| !success)
        || result
            .metadata
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

fn merge_metadata_overlay(metadata: &mut Value, overlay: Option<&Value>) {
    let Some(Value::Object(overlay)) = overlay else {
        return;
    };

    if !metadata.is_object() {
        *metadata = json!({});
    }
    let Some(object) = metadata.as_object_mut() else {
        return;
    };
    for (key, value) in overlay {
        object.insert(key.clone(), value.clone());
    }
}

fn local_provider_tool_summary(results: &[ProviderToolResult]) -> String {
    if results.is_empty() {
        return "The provider did not return a final summary.".to_string();
    }

    let rendered = results
        .iter()
        .map(|result| {
            format!(
                "Tool `{}` returned:\n```text\n{}\n```",
                result.name,
                truncate_for_summary(&result.output, 4_000)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    format!(
        "The tool-call budget ended before the provider returned a final response. Completed tool results:\n\n{}",
        rendered
    )
}

fn truncate_for_summary(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }

    let mut truncated: String = value.chars().take(max_chars).collect();
    truncated.push_str("\n\n[output truncated]");
    truncated
}

#[derive(Debug, Clone)]
pub struct AgentTurnInput {
    pub thread_id: Uuid,
    pub user_message_id: Uuid,
    pub workspace_root: PathBuf,
    pub content: String,
    pub user_content: Vec<ModelContentPart>,
    pub context_summary: Option<String>,
    pub conversation: Vec<ModelConversationMessage>,
    pub permission_mode: PermissionMode,
    pub context_budget: Option<ContextBudget>,
    pub store: Option<Arc<dyn SessionStore>>,
    pub cancellation: Option<CancellationToken>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MessagePart;
    use crate::settings::ProviderHealthCheck;
    use std::collections::VecDeque;
    use std::fs;
    use std::sync::Mutex;

    struct ScriptedProvider {
        requests: Mutex<Vec<ModelRequest>>,
        responses: Mutex<VecDeque<ModelResponse>>,
    }

    impl ScriptedProvider {
        fn new(responses: Vec<ModelResponse>) -> Self {
            Self {
                requests: Mutex::new(Vec::new()),
                responses: Mutex::new(responses.into()),
            }
        }

        fn requests(&self) -> Vec<ModelRequest> {
            self.requests.lock().expect("requests lock").clone()
        }
    }

    #[async_trait::async_trait]
    impl ModelProvider for ScriptedProvider {
        async fn complete(&self, request: ModelRequest) -> anyhow::Result<ModelResponse> {
            self.requests.lock().expect("requests lock").push(request);
            self.responses
                .lock()
                .expect("responses lock")
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("no scripted response"))
        }

        async fn check_health(&self) -> anyhow::Result<ProviderHealthCheck> {
            Ok(ProviderHealthCheck {
                reachable: true,
                latency_ms: None,
                model_available: true,
                error: None,
            })
        }
    }

    struct ReasoningProvider;

    #[async_trait::async_trait]
    impl ModelProvider for ReasoningProvider {
        async fn complete(&self, _request: ModelRequest) -> anyhow::Result<ModelResponse> {
            Ok(ModelResponse::text("已完成检查"))
        }

        async fn stream(
            &self,
            request: ModelRequest,
            on_delta: &mut crate::provider::ModelStreamCallback<'_>,
        ) -> anyhow::Result<ModelResponse> {
            let response = self.complete(request).await?;
            on_delta(ModelStreamDelta::Reasoning {
                text: "正在检查项目结构".to_string(),
            })?;
            on_delta(ModelStreamDelta::Text {
                text: response.text.clone(),
            })?;
            Ok(response)
        }

        async fn check_health(&self) -> anyhow::Result<ProviderHealthCheck> {
            Ok(ProviderHealthCheck {
                reachable: true,
                latency_ms: None,
                model_available: true,
                error: None,
            })
        }
    }

    #[test]
    fn system_prompt_prioritizes_workspace_and_limits_parent_discovery() {
        let workspace = test_workspace("system-prompt-workspace-scope");
        let additional_root = test_workspace("system-prompt-additional-root");
        let mut sandbox_config = LocalSandboxConfig::default();
        sandbox_config.read_paths = vec![additional_root.clone()];
        let loop_policy = AgentLoopPolicy::default().normalized();
        let prompt = provider_system_prompt(
            &loop_policy.status(0, 0, None),
            &workspace,
            &sandbox_config,
            false,
        );

        assert!(prompt.contains(&format!(
            "The thread workspace root is '{}'",
            workspace.canonicalize().unwrap().display()
        )));
        assert!(prompt.contains("default shell working directory is this root"));
        assert!(prompt.contains("complete the task there whenever it contains enough information"));
        assert!(prompt.contains("Do not list, search, read, or probe parent directories"));
        assert!(prompt.contains(&additional_root.display().to_string()));

        let full_access_prompt = provider_system_prompt(
            &loop_policy.status(0, 0, None),
            &workspace,
            &LocalSandboxConfig::danger_full_access(),
            false,
        );
        assert!(full_access_prompt.contains(
            "Full-access capability is not an instruction to explore outside the workspace"
        ));

        fs::remove_dir_all(workspace).unwrap();
        fs::remove_dir_all(additional_root).unwrap();
    }

    #[tokio::test]
    async fn provider_reasoning_stream_becomes_a_reasoning_event() {
        let workspace = test_workspace("provider-reasoning-event");
        let agent = AgentCore::new(Arc::new(ReasoningProvider), ToolRegistry::with_builtins());

        let events = agent
            .run_turn(AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "检查项目".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::FullAccess,
                context_budget: None,
                store: None,
                cancellation: None,
            })
            .await
            .expect("turn succeeds");

        assert!(events.iter().any(|event| matches!(
            event,
            AgentEventPayload::ReasoningDelta { text }
                if text == "正在检查项目结构"
        )));

        fs::remove_dir_all(workspace).unwrap();
    }

    #[tokio::test]
    async fn provider_tool_loop_executes_tool_and_requests_summary() {
        let workspace = test_workspace("provider-tool-loop");
        fs::write(workspace.join("sample.txt"), "hello from provider loop").unwrap();
        let provider = Arc::new(ScriptedProvider::new(vec![
            ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: "call_read".to_string(),
                    name: "read_file".to_string(),
                    arguments: json!({ "path": "sample.txt" }),
                }],
                usage: None,
            },
            ModelResponse::text("I read sample.txt and found hello from provider loop."),
        ]));
        let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());

        let events = agent
            .run_turn(AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "What is in sample.txt?".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::FullAccess,
                context_budget: None,
                store: None,
                cancellation: None,
            })
            .await
            .expect("turn succeeds");

        assert!(events.iter().any(|event| matches!(
            event,
            AgentEventPayload::ToolCallStarted { call } if call.name == "read_file"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEventPayload::ToolCallFinished { result }
                if result.metadata.get("providerToolCallId").and_then(Value::as_str) == Some("call_read")
        )));
        assert!(assistant_text(&events).contains("I read sample.txt"));

        let requests = provider.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests[0]
            .tool_candidates
            .iter()
            .any(|candidate| candidate.name == "read_file"));
        assert_eq!(requests[1].previous_tool_calls[0].id, "call_read");
        assert_eq!(requests[1].tool_results[0].call_id, "call_read");
        assert!(requests[1].tool_results[0]
            .output
            .contains("hello from provider loop"));

        let _ = fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn complete_task_finishes_without_an_extra_provider_round() {
        let workspace = test_workspace("complete-task");
        let provider = Arc::new(ScriptedProvider::new(vec![ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: "call_complete".to_string(),
                name: "complete_task".to_string(),
                arguments: json!({
                    "summary": "Implemented and verified the requested scope.",
                    "verification": ["cargo test passed"],
                    "remaining_work": []
                }),
            }],
            usage: None,
        }]));
        let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());

        let result = agent
            .run_turn_detailed_streaming(
                AgentTurnInput {
                    thread_id: Uuid::new_v4(),
                    user_message_id: Uuid::new_v4(),
                    workspace_root: workspace.clone(),
                    content: "Complete the task and report verification.".to_string(),
                    user_content: Vec::new(),
                    context_summary: None,
                    conversation: Vec::new(),
                    permission_mode: PermissionMode::FullAccess,
                    context_budget: None,
                    store: None,
                    cancellation: None,
                },
                None,
            )
            .await
            .expect("explicit completion succeeds");

        assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
        assert_eq!(provider.requests().len(), 1);
        assert!(assistant_text(&result.events).contains("Implemented and verified"));
        assert!(assistant_text(&result.events).contains("cargo test passed"));
        assert!(result.events.iter().any(|event| matches!(
            event,
            AgentEventPayload::ToolCallFinished { result }
                if result.metadata.get("taskCompletion").is_some()
        )));

        let _ = fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn verified_final_plan_update_completes_the_current_scope() {
        let workspace = test_workspace("verified-plan-completion");
        fs::create_dir_all(workspace.join("test")).unwrap();
        fs::write(
            workspace.join("test").join("check.js"),
            "console.log('passed');",
        )
        .unwrap();
        let provider = Arc::new(ScriptedProvider::new(vec![
            ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: "call_test".to_string(),
                    name: "shell".to_string(),
                    arguments: json!({ "command": "node test/check.js" }),
                }],
                usage: None,
            },
            ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: "call_plan".to_string(),
                    name: "update_plan".to_string(),
                    arguments: json!({
                        "current_scope_complete": true,
                        "verification": ["node test/check.js"],
                        "plan": [
                            { "step": "Implement current scope", "status": "completed" },
                            { "step": "Later session work", "status": "pending" }
                        ]
                    }),
                }],
                usage: None,
            },
        ]));
        let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());

        let result = agent
            .run_turn_detailed_streaming(
                AgentTurnInput {
                    thread_id: Uuid::new_v4(),
                    user_message_id: Uuid::new_v4(),
                    workspace_root: workspace.clone(),
                    content: "Complete this phase and leave the later session pending.".to_string(),
                    user_content: Vec::new(),
                    context_summary: None,
                    conversation: Vec::new(),
                    permission_mode: PermissionMode::FullAccess,
                    context_budget: None,
                    store: None,
                    cancellation: None,
                },
                None,
            )
            .await
            .expect("verified plan completion succeeds");

        assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
        assert_eq!(provider.requests().len(), 2);
        assert!(assistant_text(&result.events).contains("Current requested scope completed"));
        assert!(assistant_text(&result.events).contains("explicitly deferred"));

        let _ = fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn successful_terminal_verification_enters_completion_mode() {
        let workspace = test_workspace("completion-mode");
        fs::create_dir_all(workspace.join("test")).unwrap();
        fs::write(
            workspace.join("test").join("check.js"),
            "console.log('passed');",
        )
        .unwrap();
        let provider = Arc::new(ScriptedProvider::new(vec![
            ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: "call_plan".to_string(),
                    name: "update_plan".to_string(),
                    arguments: json!({
                        "plan": [
                            { "step": "Implement current scope", "status": "in_progress" },
                            { "step": "Run tests and verify", "status": "pending" },
                            { "step": "Session 2: implement CLI", "status": "pending" }
                        ]
                    }),
                }],
                usage: None,
            },
            ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: "call_write".to_string(),
                    name: "write_file".to_string(),
                    arguments: json!({ "path": "result.txt", "content": "done" }),
                }],
                usage: None,
            },
            ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: "call_test".to_string(),
                    name: "shell".to_string(),
                    arguments: json!({ "command": "node test/check.js" }),
                }],
                usage: None,
            },
            ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: "call_disallowed_after_test".to_string(),
                    name: "shell".to_string(),
                    arguments: json!({ "command": "type result.txt" }),
                }],
                usage: None,
            },
            ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: "call_final_plan".to_string(),
                    name: "update_plan".to_string(),
                    arguments: json!({
                        "explanation": "Implementation and verification completed.",
                        "current_scope_complete": true,
                        "verification": ["node test/check.js passed"],
                        "plan": [
                            { "step": "Implement current scope", "status": "completed" },
                            { "step": "Run tests and verify", "status": "completed" },
                            { "step": "Session 2: implement CLI", "status": "pending" }
                        ]
                    }),
                }],
                usage: None,
            },
        ]));
        let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());

        let result = agent
            .run_turn_detailed_streaming(
                AgentTurnInput {
                    thread_id: Uuid::new_v4(),
                    user_message_id: Uuid::new_v4(),
                    workspace_root: workspace.clone(),
                    content: "Implement and verify this phase.".to_string(),
                    user_content: Vec::new(),
                    context_summary: None,
                    conversation: Vec::new(),
                    permission_mode: PermissionMode::FullAccess,
                    context_budget: None,
                    store: None,
                    cancellation: None,
                },
                None,
            )
            .await
            .expect("verified completion mode succeeds");

        assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
        let requests = provider.requests();
        assert_eq!(requests.len(), 5);
        for request in &requests[3..] {
            let mut final_tool_names = request
                .tool_candidates
                .iter()
                .map(|candidate| candidate.name.as_str())
                .collect::<Vec<_>>();
            final_tool_names.sort_unstable();
            assert_eq!(final_tool_names, vec!["complete_task", "update_plan"]);
        }
        assert!(requests[3]
            .system_prompt
            .contains("Completion mode is active"));
        assert!(result.events.iter().any(|event| matches!(
            event,
            AgentEventPayload::ToolCallFinished { result }
                if result.metadata.get("completionModeBlocked") == Some(&Value::Bool(true))
        )));
        assert!(assistant_text(&result.events).contains("Current requested scope completed"));
        assert!(result.events.iter().any(|event| matches!(
            event,
            AgentEventPayload::PlanUpdated { plan }
                if plan.explanation.as_deref() == Some("Implementation and verification completed.")
                    && plan.steps[0].status == TaskPlanStepStatus::Completed
                    && plan.steps[2].status == TaskPlanStepStatus::Pending
        )));

        let _ = fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn many_distinct_observations_do_not_disable_tools() {
        let workspace = test_workspace("distinct-observations");
        fs::create_dir_all(workspace.join("src")).unwrap();
        for index in 0..11 {
            fs::write(workspace.join(format!("context-{index}.txt")), "context").unwrap();
        }
        let provider = Arc::new(ScriptedProvider::new(vec![
            ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: "call_plan".to_string(),
                    name: "update_plan".to_string(),
                    arguments: json!({
                        "plan": [
                            { "step": "Implement CLI contract", "status": "in_progress" },
                            { "step": "Run tests and verify", "status": "pending" }
                        ]
                    }),
                }],
                usage: None,
            },
            ModelResponse {
                text: String::new(),
                tool_calls: (0..11)
                    .map(|index| ProviderToolCall {
                        id: format!("call_read_{index}"),
                        name: "read_file".to_string(),
                        arguments: json!({ "path": format!("context-{index}.txt") }),
                    })
                    .collect(),
                usage: None,
            },
            ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: "call_stagnant_read".to_string(),
                    name: "list_files".to_string(),
                    arguments: json!({ "path": "." }),
                }],
                usage: None,
            },
            ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: "call_write".to_string(),
                    name: "write_file".to_string(),
                    arguments: json!({ "path": "src/cli.js", "content": "export {};\n" }),
                }],
                usage: None,
            },
            ModelResponse::text("The implementation is now in place."),
        ]));
        let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());

        let result = agent
            .run_turn_detailed_streaming(
                AgentTurnInput {
                    thread_id: Uuid::new_v4(),
                    user_message_id: Uuid::new_v4(),
                    workspace_root: workspace.clone(),
                    content: "Implement the CLI after inspecting the task context.".to_string(),
                    user_content: Vec::new(),
                    context_summary: None,
                    conversation: Vec::new(),
                    permission_mode: PermissionMode::FullAccess,
                    context_budget: None,
                    store: None,
                    cancellation: None,
                },
                None,
            )
            .await
            .expect("distinct observations remain allowed");

        assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
        let requests = provider.requests();
        assert_eq!(requests.len(), 5);
        for request in &requests[2..] {
            assert!(request
                .tool_candidates
                .iter()
                .any(|candidate| candidate.name == "read_file"));
            assert!(request
                .system_prompt
                .contains("no elapsed-time or total tool-round limit"));
        }
        assert!(!result.events.iter().any(|event| matches!(
            event,
            AgentEventPayload::ToolCallFinished { result }
                if result.metadata.get("loopGuardBlocked") == Some(&Value::Bool(true))
        )));
        assert_eq!(
            fs::read_to_string(workspace.join("src").join("cli.js")).unwrap(),
            "export {};\n"
        );

        let _ = fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn repeated_completion_mode_violations_use_verified_fallback() {
        let workspace = test_workspace("completion-mode-fallback");
        fs::create_dir_all(workspace.join("test")).unwrap();
        fs::write(
            workspace.join("test").join("check.js"),
            "console.log('passed');",
        )
        .unwrap();
        let provider = Arc::new(ScriptedProvider::new(vec![
            ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: "call_plan".to_string(),
                    name: "update_plan".to_string(),
                    arguments: json!({
                        "plan": [
                            { "step": "Implement current scope", "status": "in_progress" },
                            { "step": "Run tests and verify", "status": "pending" },
                            { "step": "Session 2: implement CLI", "status": "pending" }
                        ]
                    }),
                }],
                usage: None,
            },
            ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: "call_write".to_string(),
                    name: "write_file".to_string(),
                    arguments: json!({ "path": "result.txt", "content": "done" }),
                }],
                usage: None,
            },
            ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: "call_test".to_string(),
                    name: "shell".to_string(),
                    arguments: json!({ "command": "node test/check.js" }),
                }],
                usage: None,
            },
            ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: "call_violation_1".to_string(),
                    name: "read_file".to_string(),
                    arguments: json!({ "path": "result.txt" }),
                }],
                usage: None,
            },
            ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: "call_violation_2".to_string(),
                    name: "git_diff".to_string(),
                    arguments: json!({}),
                }],
                usage: None,
            },
        ]));
        let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());

        let result = agent
            .run_turn_detailed_streaming(
                AgentTurnInput {
                    thread_id: Uuid::new_v4(),
                    user_message_id: Uuid::new_v4(),
                    workspace_root: workspace.clone(),
                    content: "Implement and verify this phase.".to_string(),
                    user_content: Vec::new(),
                    context_summary: None,
                    conversation: Vec::new(),
                    permission_mode: PermissionMode::FullAccess,
                    context_budget: None,
                    store: None,
                    cancellation: None,
                },
                None,
            )
            .await
            .expect("verified fallback closes the turn");

        assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
        assert_eq!(provider.requests().len(), 5);
        assert!(assistant_text(&result.events).contains("runtime fallback"));
        assert!(result.events.iter().any(|event| matches!(
            event,
            AgentEventPayload::PlanUpdated { plan }
                if plan.explanation.as_deref().is_some_and(|value| value.starts_with("Runtime fallback"))
                    && plan.steps[0].status == TaskPlanStepStatus::Completed
                    && plan.steps[1].status == TaskPlanStepStatus::Completed
                    && plan.steps[2].status == TaskPlanStepStatus::Pending
        )));

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn verified_fallback_completes_resumed_phase_steps() {
        let mut loop_guard = AgentLoopGuardState {
            last_successful_verification: Some("npm test".to_string()),
            latest_plan: Some(TaskPlan {
                explanation: None,
                steps: vec![
                    crate::model::TaskPlanStep {
                        step: "Phase 1: implement library".to_string(),
                        status: TaskPlanStepStatus::Completed,
                    },
                    crate::model::TaskPlanStep {
                        step: "Phase 2: implement CLI".to_string(),
                        status: TaskPlanStepStatus::Pending,
                    },
                ],
            }),
            ..AgentLoopGuardState::default()
        };
        let mut events = TurnEvents::new(None);

        let output = fallback_verified_completion_output(
            &mut loop_guard,
            "User request:\nContinue the remaining work after restart.",
            &mut events,
        )
        .expect("fallback output");

        assert!(!output.contains("Remaining work"));
        assert!(loop_guard.latest_plan.as_ref().is_some_and(|plan| plan
            .steps
            .iter()
            .all(|step| step.status == TaskPlanStepStatus::Completed)));
    }

    #[test]
    fn chinese_plan_keywords_drive_loop_guards() {
        assert!(step_looks_like_verification("运行测试并验证结果"));
        assert!(step_looks_like_workspace_change("实现并修复命令行工具"));
        assert!(request_resumes_deferred_work("继续完成第二阶段的剩余工作"));
        assert!(step_is_explicitly_deferred("后续阶段暂缓实现"));
    }

    #[test]
    fn changing_tool_results_reset_equivalent_call_detection() {
        let policy = AgentLoopPolicy::default().normalized();
        let mut guard = AgentLoopGuardState::default();
        for index in 0..12 {
            let call = ProviderToolCall {
                id: format!("call_{index}"),
                name: "read_file".to_string(),
                arguments: json!({ "path": "changing.txt" }),
            };
            assert!(guard.register_tool_call(&call, &policy).is_none());
            guard.observe_tool_result(
                &call,
                &ProviderToolResult {
                    call_id: call.id.clone(),
                    name: call.name.clone(),
                    output: format!("state {index}"),
                    content: Vec::new(),
                    is_error: false,
                    metadata: json!({
                        "providerToolCallId": call.id,
                        "durationMs": index,
                        "success": true
                    }),
                },
            );
        }
    }

    #[test]
    fn result_fingerprint_ignores_transport_ids_and_timing() {
        let result = |call_id: &str, duration_ms: u64, status: &str| ProviderToolResult {
            call_id: call_id.to_string(),
            name: "wait_agents".to_string(),
            output: json!({
                "status": status,
                "updatedAt": format!("2026-07-18T12:00:{duration_ms:02}Z")
            })
            .to_string(),
            content: Vec::new(),
            is_error: false,
            metadata: json!({
                "providerToolCallId": call_id,
                "durationMs": duration_ms,
                "success": true
            }),
        };
        assert_eq!(
            provider_tool_result_fingerprint(&result("call_1", 1, "running")),
            provider_tool_result_fingerprint(&result("call_2", 9, "running"))
        );
        assert_ne!(
            provider_tool_result_fingerprint(&result("call_1", 1, "running")),
            provider_tool_result_fingerprint(&result("call_2", 9, "completed"))
        );
    }

    #[test]
    fn legacy_budget_continuation_deserializes_into_loop_policy() {
        let continuation = AgentContinuation {
            thread_id: Uuid::new_v4(),
            user_message_id: Uuid::new_v4(),
            workspace_root: PathBuf::from("workspace"),
            context_summary: None,
            conversation: Vec::new(),
            permission_mode: PermissionMode::Auto,
            context_budget: None,
            loop_policy: AgentLoopPolicy::default(),
            loop_guard: AgentLoopGuardState::default(),
            continuation_kind: AgentContinuationKind::LegacyBudgetCheckpoint,
            state: AgentContinuationState::Provider {
                model_user_message: "continue".to_string(),
                model_user_content: Vec::new(),
                tool_candidates: Vec::new(),
                provider_tool_calls: Vec::new(),
                provider_tool_results: Vec::new(),
                pending_tool_calls: Vec::new(),
                compacted_tool_history: String::new(),
                current_round: 8,
            },
        };
        let mut value = serde_json::to_value(continuation).unwrap();
        let object = value.as_object_mut().unwrap();
        object.remove("loopPolicy");
        object.insert(
            "executionBudget".to_string(),
            json!({
                "maxToolRounds": 8,
                "maxTotalToolRounds": 24,
                "maxEquivalentToolCalls": 3,
                "maxObservationToolCallsWithoutWorkspaceChange": 12,
                "maxElapsedMs": 900_000
            }),
        );
        value
            .get_mut("state")
            .and_then(Value::as_object_mut)
            .unwrap()
            .remove("compactedToolHistory");

        let restored: AgentContinuation = serde_json::from_value(value).unwrap();
        assert_eq!(restored.loop_policy.max_equivalent_tool_calls, 3);
        assert_eq!(
            restored.loop_policy.max_repeated_tool_cycles,
            DEFAULT_MAX_REPEATED_TOOL_CYCLES
        );
        assert!(matches!(
            restored.continuation_kind,
            AgentContinuationKind::LegacyBudgetCheckpoint
        ));
    }

    #[tokio::test]
    async fn repeated_multi_step_cycle_stops_as_incomplete() {
        let workspace = test_workspace("repeated-tool-cycle");
        fs::write(workspace.join("a.txt"), "a").unwrap();
        fs::write(workspace.join("b.txt"), "b").unwrap();
        let mut responses = vec![ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: "call_plan".to_string(),
                name: "update_plan".to_string(),
                arguments: json!({
                    "plan": [
                        { "step": "Resolve the current problem", "status": "in_progress" }
                    ]
                }),
            }],
            usage: None,
        }];
        responses.extend((0..8).map(|index| ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: format!("call_cycle_{index}"),
                name: "read_file".to_string(),
                arguments: json!({ "path": if index % 2 == 0 { "a.txt" } else { "b.txt" } }),
            }],
            usage: None,
        }));
        let provider = Arc::new(ScriptedProvider::new(responses));
        let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());

        let result = agent
            .run_turn_detailed_streaming(
                AgentTurnInput {
                    thread_id: Uuid::new_v4(),
                    user_message_id: Uuid::new_v4(),
                    workspace_root: workspace.clone(),
                    content: "Resolve the problem without repeating the same investigation."
                        .to_string(),
                    user_content: Vec::new(),
                    context_summary: None,
                    conversation: Vec::new(),
                    permission_mode: PermissionMode::FullAccess,
                    context_budget: None,
                    store: None,
                    cancellation: None,
                },
                None,
            )
            .await
            .expect("cycle detection closes the turn");

        assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
        assert_eq!(provider.requests().len(), 9);
        assert!(assistant_text(&result.events).contains("Task remains incomplete"));
        assert!(assistant_text(&result.events).contains("same tool-call cycle"));
        assert!(assistant_text(&result.events).contains("no elapsed-time or total-round limit"));
        assert!(result.events.iter().any(|event| matches!(
            event,
            AgentEventPayload::ToolCallFinished { result }
                if result.metadata.get("kind").and_then(Value::as_str)
                    == Some("repeated_cycle")
        )));

        let _ = fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn equivalent_tool_calls_are_blocked_until_state_changes() {
        let workspace = test_workspace("equivalent-tool-loop");
        fs::write(workspace.join("sample.txt"), "stable content").unwrap();
        let responses = (0..4)
            .map(|index| ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: format!("call_read_{index}"),
                    name: "read_file".to_string(),
                    arguments: json!({ "path": "sample.txt" }),
                }],
                usage: None,
            })
            .chain(std::iter::once(ModelResponse::text(
                "Stopped retrying the equivalent read.",
            )))
            .collect();
        let provider = Arc::new(ScriptedProvider::new(responses));
        let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());

        let events = agent
            .run_turn(AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Inspect sample.txt without looping.".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::FullAccess,
                context_budget: None,
                store: None,
                cancellation: None,
            })
            .await
            .expect("loop guard returns the blocked result to the provider");

        assert!(events.iter().any(|event| matches!(
            event,
            AgentEventPayload::ToolCallFinished { result }
                if result.metadata.get("loopGuardBlocked").and_then(Value::as_bool) == Some(true)
        )));
        let requests = provider.requests();
        assert_eq!(requests.len(), 5);
        assert_eq!(
            requests[4].tool_results[3]
                .metadata
                .get("equivalentCallCount")
                .and_then(Value::as_u64),
            Some(4)
        );

        let _ = fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn approve_mode_workspace_write_completes_without_suspension() {
        let workspace = test_workspace("approve-workspace-write");
        let provider = Arc::new(ScriptedProvider::new(vec![
            ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: "call_write".to_string(),
                    name: "write_file".to_string(),
                    arguments: json!({ "path": "approved.txt", "content": "approved once" }),
                }],
                usage: None,
            },
            ModelResponse::text("Approved file written."),
        ]));
        let agent = AgentCore::new(provider, ToolRegistry::with_builtins());
        let result = agent
            .run_turn_detailed_streaming(
                AgentTurnInput {
                    thread_id: Uuid::new_v4(),
                    user_message_id: Uuid::new_v4(),
                    workspace_root: workspace.clone(),
                    content: "Create approved.txt with the requested content.".to_string(),
                    user_content: Vec::new(),
                    context_summary: None,
                    conversation: Vec::new(),
                    permission_mode: PermissionMode::Approve,
                    context_budget: None,
                    store: None,
                    cancellation: None,
                },
                None,
            )
            .await
            .expect("workspace write completes");
        assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
        assert_eq!(
            fs::read_to_string(workspace.join("approved.txt")).unwrap(),
            "approved once"
        );
        let _ = fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn approved_protected_metadata_write_uses_one_shot_path_grant() {
        let workspace = test_workspace("approved-path-grant");
        let provider = Arc::new(ScriptedProvider::new(vec![
            ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: "call_write_metadata".to_string(),
                    name: "write_file".to_string(),
                    arguments: json!({
                        "path": ".codex/config.toml",
                        "content": "approved metadata"
                    }),
                }],
                usage: None,
            },
            ModelResponse::text("Approved metadata written."),
        ]));
        let agent = AgentCore::new(provider, ToolRegistry::with_builtins());
        let result = agent
            .run_turn_detailed_streaming(
                AgentTurnInput {
                    thread_id: Uuid::new_v4(),
                    user_message_id: Uuid::new_v4(),
                    workspace_root: workspace.clone(),
                    content: "Update the protected metadata configuration.".to_string(),
                    user_content: Vec::new(),
                    context_summary: None,
                    conversation: Vec::new(),
                    permission_mode: PermissionMode::Auto,
                    context_budget: None,
                    store: None,
                    cancellation: None,
                },
                None,
            )
            .await
            .expect("protected metadata write suspends");
        assert!(!workspace.join(".codex/config.toml").exists());
        let continuation = match result.outcome {
            AgentTurnOutcome::Suspended { continuation, .. } => continuation,
            AgentTurnOutcome::Completed => panic!("protected write should wait for approval"),
        };

        let resumed = agent
            .resume_turn_streaming(continuation, true, None, None, None)
            .await
            .expect("approved path grant resumes");

        assert!(matches!(resumed.outcome, AgentTurnOutcome::Completed));
        assert_eq!(
            fs::read_to_string(workspace.join(".codex/config.toml")).unwrap(),
            "approved metadata"
        );
        let _ = fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn approved_external_write_grant_does_not_authorize_a_sibling_call() {
        let workspace = test_workspace("approved-external-path-grant");
        let outside = test_workspace("approved-external-path-target");
        let approved_path = outside.join("approved.txt");
        let sibling_path = outside.join("not-approved.txt");
        let provider = Arc::new(ScriptedProvider::new(vec![
            ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: "call_approved_path".to_string(),
                    name: "write_file".to_string(),
                    arguments: json!({
                        "path": approved_path,
                        "content": "approved once"
                    }),
                }],
                usage: None,
            },
            ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: "call_sibling_path".to_string(),
                    name: "write_file".to_string(),
                    arguments: json!({
                        "path": sibling_path,
                        "content": "must require its own approval"
                    }),
                }],
                usage: None,
            },
        ]));
        let agent = AgentCore::new(provider, ToolRegistry::with_builtins())
            .with_sandbox_config(LocalSandboxConfig::enforce());

        let result = agent
            .run_turn_detailed_streaming(
                AgentTurnInput {
                    thread_id: Uuid::new_v4(),
                    user_message_id: Uuid::new_v4(),
                    workspace_root: workspace.clone(),
                    content: "Write only the explicitly approved external file.".to_string(),
                    user_content: Vec::new(),
                    context_summary: None,
                    conversation: Vec::new(),
                    permission_mode: PermissionMode::Approve,
                    context_budget: None,
                    store: None,
                    cancellation: None,
                },
                None,
            )
            .await
            .expect("external write waits for approval");
        let continuation = match result.outcome {
            AgentTurnOutcome::Suspended { continuation, .. } => continuation,
            AgentTurnOutcome::Completed => panic!("external write should wait for approval"),
        };

        let resumed = agent
            .resume_turn_streaming(continuation, true, None, None, None)
            .await
            .expect("approved external path is written");

        assert!(matches!(
            resumed.outcome,
            AgentTurnOutcome::Suspended { .. }
        ));
        assert_eq!(fs::read_to_string(&approved_path).unwrap(), "approved once");
        assert!(!sibling_path.exists());
        let _ = fs::remove_dir_all(workspace);
        let _ = fs::remove_dir_all(outside);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn approved_shell_command_cannot_disable_the_os_sandbox() {
        let workspace = test_workspace("approved-shell-remains-sandboxed");
        let outside = std::env::current_dir()
            .expect("current directory")
            .parent()
            .expect("workspace parent")
            .join(format!("opentopia-approved-outside-{}.txt", Uuid::new_v4()));
        let escaped_outside = outside.to_string_lossy().replace('\'', "''");
        let command = format!(
            "$ErrorActionPreference='Stop'; Set-Content -LiteralPath '{escaped_outside}' -Value approved-shell"
        );
        let provider = Arc::new(ScriptedProvider::new(vec![
            ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: "call_shell".to_string(),
                    name: "shell".to_string(),
                    arguments: json!({ "command": command }),
                }],
                usage: None,
            },
            ModelResponse::text("Approved shell command completed."),
        ]));
        let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins())
            .with_sandbox_config(LocalSandboxConfig::enforce());

        let result = agent
            .run_turn_detailed_streaming(
                AgentTurnInput {
                    thread_id: Uuid::new_v4(),
                    user_message_id: Uuid::new_v4(),
                    workspace_root: workspace.clone(),
                    content: "Run the requested external write command.".to_string(),
                    user_content: Vec::new(),
                    context_summary: None,
                    conversation: Vec::new(),
                    permission_mode: PermissionMode::Auto,
                    context_budget: None,
                    store: None,
                    cancellation: None,
                },
                None,
            )
            .await
            .expect("sandbox denial suspends the turn");
        assert!(!outside.exists());
        let continuation = match result.outcome {
            AgentTurnOutcome::Suspended { continuation, .. } => continuation,
            AgentTurnOutcome::Completed => panic!("sandbox denial should wait for approval"),
        };

        let resumed = agent
            .resume_turn_streaming(continuation, true, None, None, None)
            .await
            .expect("approved call returns the sandbox denial to the model");

        assert!(matches!(resumed.outcome, AgentTurnOutcome::Completed));
        assert!(!outside.exists());
        let requests = provider.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[1].tool_results[0]
                .metadata
                .get("sandboxEscalationDenied")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert!(resumed.events.iter().any(|event| matches!(
            event,
            AgentEventPayload::ToolCallFinished { result }
                if result
                    .metadata
                    .get("sandboxEscalationDenied")
                    .and_then(Value::as_bool)
                    == Some(true)
        )));
        let _ = fs::remove_file(outside);
        let _ = fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn denied_protected_metadata_write_completes_without_execution() {
        let workspace = test_workspace("denied-protected-continuation");
        let provider = Arc::new(ScriptedProvider::new(vec![
            ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: "call_denied_write".to_string(),
                    name: "write_file".to_string(),
                    arguments: json!({
                        "path": ".codex/denied.txt",
                        "content": "never written"
                    }),
                }],
                usage: None,
            },
            ModelResponse::text("The file was not written because approval was denied."),
        ]));
        let agent = AgentCore::new(provider, ToolRegistry::with_builtins());
        let result = agent
            .run_turn_detailed_streaming(
                AgentTurnInput {
                    thread_id: Uuid::new_v4(),
                    user_message_id: Uuid::new_v4(),
                    workspace_root: workspace.clone(),
                    content: "Create protected metadata with the requested content.".to_string(),
                    user_content: Vec::new(),
                    context_summary: None,
                    conversation: Vec::new(),
                    permission_mode: PermissionMode::Approve,
                    context_budget: None,
                    store: None,
                    cancellation: None,
                },
                None,
            )
            .await
            .expect("turn suspends");
        let continuation = match result.outcome {
            AgentTurnOutcome::Suspended { continuation, .. } => continuation,
            AgentTurnOutcome::Completed => panic!("turn should wait for approval"),
        };

        let resumed = agent
            .resume_turn_streaming(continuation, false, None, None, None)
            .await
            .expect("denied turn resolves");
        assert!(matches!(resumed.outcome, AgentTurnOutcome::Completed));
        assert!(!workspace.join(".codex/denied.txt").exists());
        let _ = fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn denied_protected_tool_call_is_returned_to_model_as_error() {
        let workspace = test_workspace("denied-provider-continuation");
        let provider = Arc::new(ScriptedProvider::new(vec![
            ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: "call_write".to_string(),
                    name: "write_file".to_string(),
                    arguments: json!({
                        "path": ".codex/denied-provider.txt",
                        "content": "must not exist"
                    }),
                }],
                usage: None,
            },
            ModelResponse::text("I did not write the file because approval was denied."),
        ]));
        let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());
        let result = agent
            .run_turn_detailed_streaming(
                AgentTurnInput {
                    thread_id: Uuid::new_v4(),
                    user_message_id: Uuid::new_v4(),
                    workspace_root: workspace.clone(),
                    content: "Create protected provider metadata".to_string(),
                    user_content: Vec::new(),
                    context_summary: None,
                    conversation: Vec::new(),
                    permission_mode: PermissionMode::Approve,
                    context_budget: None,
                    store: None,
                    cancellation: None,
                },
                None,
            )
            .await
            .expect("provider turn suspends");
        let continuation = match result.outcome {
            AgentTurnOutcome::Suspended { continuation, .. } => continuation,
            AgentTurnOutcome::Completed => panic!("protected write should require approval"),
        };

        let resumed = agent
            .resume_turn_streaming(continuation, false, None, None, None)
            .await
            .expect("provider receives denial result");
        assert!(matches!(resumed.outcome, AgentTurnOutcome::Completed));
        assert!(assistant_text(&resumed.events).contains("approval was denied"));
        assert!(!workspace.join(".codex/denied-provider.txt").exists());
        let requests = provider.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].tool_results[0].is_error);
        assert_eq!(
            requests[1].tool_results[0]
                .metadata
                .get("approvalDenied")
                .and_then(Value::as_bool),
            Some(true)
        );
        let _ = fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn turn_cancellation_reaches_shell_execution_context() {
        let workspace = test_workspace("turn-shell-cancellation");
        let cancellation = CancellationToken::new();
        let command = if cfg!(windows) {
            "powershell -NoProfile -Command \"Start-Sleep -Seconds 30\""
        } else {
            "sh -c 'sleep 30'"
        };
        let provider = Arc::new(ScriptedProvider::new(vec![ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: "call_sleep".to_string(),
                name: "shell".to_string(),
                arguments: json!({ "command": command }),
            }],
            usage: None,
        }]));
        let agent = AgentCore::new(provider, ToolRegistry::with_builtins());
        let workspace_for_turn = workspace.clone();
        let cancellation_for_turn = cancellation.clone();
        let task = tokio::spawn(async move {
            agent
                .run_turn(AgentTurnInput {
                    thread_id: Uuid::new_v4(),
                    user_message_id: Uuid::new_v4(),
                    workspace_root: workspace_for_turn,
                    content: "Run a long-running command.".to_string(),
                    user_content: Vec::new(),
                    context_summary: None,
                    conversation: Vec::new(),
                    permission_mode: PermissionMode::FullAccess,
                    context_budget: None,
                    store: None,
                    cancellation: Some(cancellation_for_turn),
                })
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        cancellation.cancel();
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .expect("cancelled shell returns promptly")
            .expect("turn task joins");
        assert!(result
            .expect_err("cancelled shell should fail the command turn")
            .to_string()
            .contains("cancelled"));
        let _ = fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn provider_tool_loop_supports_multiple_rounds() {
        let workspace = test_workspace("provider-multi-tool-loop");
        fs::write(workspace.join("first.txt"), "first result").unwrap();
        fs::write(workspace.join("second.txt"), "second result").unwrap();
        let provider = Arc::new(ScriptedProvider::new(vec![
            ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: "call_first".to_string(),
                    name: "read_file".to_string(),
                    arguments: json!({ "path": "first.txt" }),
                }],
                usage: None,
            },
            ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: "call_second".to_string(),
                    name: "read_file".to_string(),
                    arguments: json!({ "path": "second.txt" }),
                }],
                usage: None,
            },
            ModelResponse::text("Both files were inspected."),
        ]));
        let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());

        let events = agent
            .run_turn(AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Inspect both files.".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::FullAccess,
                context_budget: None,
                store: None,
                cancellation: None,
            })
            .await
            .expect("turn succeeds");

        assert!(assistant_text(&events).contains("Both files were inspected."));
        let requests = provider.requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[2].previous_tool_calls.len(), 2);
        assert_eq!(requests[2].tool_results.len(), 2);
        assert!(requests[2]
            .tool_candidates
            .iter()
            .any(|tool| tool.name == "read_file"));
        assert!(requests[2].tool_results[0].output.contains("first result"));
        assert!(requests[2].tool_results[1].output.contains("second result"));

        let _ = fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn eight_tool_rounds_continue_without_approval() {
        let workspace = test_workspace("eight-tool-rounds");
        for index in 0..8 {
            fs::write(workspace.join(format!("sample-{index}.txt")), "content").unwrap();
        }
        let tool_responses = (0..8)
            .map(|index| ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: format!("call_{index}"),
                    name: "read_file".to_string(),
                    arguments: json!({ "path": format!("sample-{index}.txt") }),
                }],
                usage: None,
            })
            .collect::<Vec<_>>();
        let provider = Arc::new(ScriptedProvider::new(
            tool_responses
                .into_iter()
                .chain(std::iter::once(ModelResponse::text(
                    "Completed all eight distinct observations without a checkpoint.",
                )))
                .collect(),
        ));
        let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());

        let result = agent
            .run_turn_detailed_streaming(
                AgentTurnInput {
                    thread_id: Uuid::new_v4(),
                    user_message_id: Uuid::new_v4(),
                    workspace_root: workspace.clone(),
                    content: "Inspect sample.txt until the work is complete.".to_string(),
                    user_content: Vec::new(),
                    context_summary: None,
                    conversation: Vec::new(),
                    permission_mode: PermissionMode::FullAccess,
                    context_budget: None,
                    store: None,
                    cancellation: None,
                },
                None,
            )
            .await
            .expect("turn continues without a checkpoint");

        assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
        assert!(assistant_text(&result.events).contains("without a checkpoint"));
        assert!(!result.events.iter().any(|event| matches!(
            event,
            AgentEventPayload::ApprovalRequested { action, .. }
                if action == "Continue agent execution"
        )));
        assert_eq!(provider.requests().len(), 9);
        assert!(provider.requests()[8]
            .system_prompt
            .contains("no elapsed-time or total tool-round limit"));

        let _ = fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn more_than_twenty_four_distinct_tool_rounds_can_complete() {
        let workspace = test_workspace("unbounded-tool-rounds");
        for index in 0..30 {
            fs::write(workspace.join(format!("sample-{index}.txt")), "content").unwrap();
        }
        let responses = (0..30)
            .map(|index| ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: format!("call_{index}"),
                    name: "read_file".to_string(),
                    arguments: json!({ "path": format!("sample-{index}.txt") }),
                }],
                usage: None,
            })
            .chain(std::iter::once(ModelResponse::text(
                "Completed after thirty distinct tool rounds.",
            )))
            .collect();
        let provider = Arc::new(ScriptedProvider::new(responses));
        let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());
        let result = agent
            .run_turn_detailed_streaming(
                AgentTurnInput {
                    thread_id: Uuid::new_v4(),
                    user_message_id: Uuid::new_v4(),
                    workspace_root: workspace.clone(),
                    content: "Inspect all thirty distinct inputs.".to_string(),
                    user_content: Vec::new(),
                    context_summary: None,
                    conversation: Vec::new(),
                    permission_mode: PermissionMode::FullAccess,
                    context_budget: None,
                    store: None,
                    cancellation: None,
                },
                None,
            )
            .await
            .expect("long turn completes without continuation");

        assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
        assert!(assistant_text(&result.events).contains("thirty distinct tool rounds"));
        let requests = provider.requests();
        assert_eq!(requests.len(), 31);
        let final_request = requests.last().expect("final provider request");
        assert!(!final_request.tool_candidates.is_empty());
        assert!(final_request
            .system_prompt
            .contains("completed 30 tool-decision rounds"));

        let _ = fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn long_turn_compacts_completed_tool_history_automatically() {
        let workspace = test_workspace("automatic-tool-history-compaction");
        for index in 0..10 {
            fs::write(
                workspace.join(format!("large-{index}.txt")),
                format!("record-{index}-{}", "x".repeat(2_000)),
            )
            .unwrap();
        }
        let provider = Arc::new(ScriptedProvider::new(vec![
            ModelResponse {
                text: String::new(),
                tool_calls: (0..10)
                    .map(|index| ProviderToolCall {
                        id: format!("call_{index}"),
                        name: "read_file".to_string(),
                        arguments: json!({ "path": format!("large-{index}.txt") }),
                    })
                    .collect(),
                usage: None,
            },
            ModelResponse::text("Completed after automatic context maintenance."),
        ]));
        let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());

        let result = agent
            .run_turn_detailed_streaming(
                AgentTurnInput {
                    thread_id: Uuid::new_v4(),
                    user_message_id: Uuid::new_v4(),
                    workspace_root: workspace.clone(),
                    content: "Inspect all large records.".to_string(),
                    user_content: Vec::new(),
                    context_summary: None,
                    conversation: Vec::new(),
                    permission_mode: PermissionMode::FullAccess,
                    context_budget: Some(ContextBudget::new(4_096)),
                    store: None,
                    cancellation: None,
                },
                None,
            )
            .await
            .expect("history compaction is automatic");

        assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
        let requests = provider.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].previous_tool_calls.len() < 10);
        assert!(requests[1].previous_tool_calls.len() >= 4);
        assert!(requests[1].conversation.iter().any(|message| message
            .content
            .starts_with("[Automatically compacted tool history]")));
        assert!(!result.events.iter().any(|event| matches!(
            event,
            AgentEventPayload::ApprovalRequested { action, .. }
                if action == "Continue agent execution"
        )));

        let _ = fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn provider_request_includes_durable_context_summary() {
        let workspace = test_workspace("provider-durable-context");
        let provider = Arc::new(ScriptedProvider::new(vec![ModelResponse::text(
            "Continued from durable context.",
        )]));
        let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());

        agent
            .run_turn(AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Continue the implementation.".to_string(),
                user_content: Vec::new(),
                context_summary: Some("Decision: keep the Rust sidecar API stable.".to_string()),
                conversation: Vec::new(),
                permission_mode: PermissionMode::FullAccess,
                context_budget: None,
                store: None,
                cancellation: None,
            })
            .await
            .expect("turn succeeds");

        let requests = provider.requests();
        assert_eq!(requests.len(), 1);
        assert!(requests[0]
            .user_message
            .contains("Durable context from earlier turns:"));
        assert!(requests[0]
            .user_message
            .contains("keep the Rust sidecar API stable"));

        let _ = fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn provider_request_does_not_prefetch_workspace_listing() {
        let workspace = test_workspace("no-workspace-preflight");
        fs::write(workspace.join("private.txt"), "workspace marker").unwrap();
        let provider = Arc::new(ScriptedProvider::new(vec![ModelResponse::text(
            "No workspace inspection was required.",
        )]));
        let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());

        let events = agent
            .run_turn(AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Explain the available tools.".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::FullAccess,
                context_budget: None,
                store: None,
                cancellation: None,
            })
            .await
            .expect("turn succeeds");

        assert!(!events.iter().any(|event| matches!(
            event,
            AgentEventPayload::ToolCallStarted { call } if call.name == "list_files"
        )));
        let requests = provider.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].user_message,
            "User request:\nExplain the available tools."
        );
        assert!(!requests[0].user_message.contains("Workspace root listing"));
        assert!(!requests[0].user_message.contains("workspace marker"));
        assert!(requests[0]
            .tool_candidates
            .iter()
            .any(|candidate| candidate.name == "list_files"));
        let (round, snapshot) = events
            .iter()
            .find_map(|event| match event {
                AgentEventPayload::ModelRequest { round, request } => Some((round, request)),
                _ => None,
            })
            .expect("model request snapshot");
        assert_eq!(*round, 1);
        assert_eq!(snapshot["userMessage"], requests[0].user_message);
        assert_eq!(
            snapshot["toolCandidates"],
            serde_json::to_value(&requests[0].tool_candidates).unwrap()
        );

        let _ = fs::remove_dir_all(workspace);
    }

    fn test_workspace(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("opentopia-{name}-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn assistant_text(events: &[AgentEventPayload]) -> String {
        events
            .iter()
            .filter_map(|event| match event {
                AgentEventPayload::AssistantMessage { message } => Some(
                    message
                        .parts
                        .iter()
                        .filter_map(|part| match part {
                            MessagePart::Text { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
