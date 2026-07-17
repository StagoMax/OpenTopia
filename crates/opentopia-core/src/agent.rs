use crate::browser::{BrowserRuntime, BrowserRuntimeConfig, LocalBrowserRuntime};
use crate::mcp::McpToolDescriptor;
use crate::mcp_host::McpExtensionHost;
use crate::model::{
    AgentEventPayload, Message, MessageRole, ModelContentPart, TaskPlan, TaskPlanStepStatus,
    ToolCall, ToolResult,
};
use crate::policy::{BasicPolicyEngine, PermissionMode};
use crate::provider::{
    MockProvider, ModelConversationMessage, ModelProvider, ModelRequest, ModelResponse,
    ModelStreamDelta, OpenAiCompatibleProvider, ProviderToolCall, ProviderToolCandidate,
    ProviderToolResult,
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
use std::time::Instant;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const DEFAULT_MAX_PROVIDER_TOOL_ROUNDS: usize = 8;
const DEFAULT_MAX_TOTAL_PROVIDER_TOOL_ROUNDS: usize = 24;
const DEFAULT_MAX_EQUIVALENT_TOOL_CALLS: usize = 3;
const DEFAULT_MAX_OBSERVATION_TOOL_CALLS_WITHOUT_WORKSPACE_CHANGE: usize = 12;
const MAX_COMPLETION_MODE_VIOLATIONS: usize = 2;
const MAX_IMPLEMENTATION_MODE_VIOLATIONS: usize = 3;
const DEFAULT_MAX_TURN_ELAPSED_MS: u64 = 15 * 60 * 1_000;

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
pub struct AgentExecutionBudget {
    pub max_tool_rounds: usize,
    #[serde(default = "default_max_total_tool_rounds")]
    pub max_total_tool_rounds: usize,
    #[serde(default = "default_max_equivalent_tool_calls")]
    pub max_equivalent_tool_calls: usize,
    #[serde(default = "default_max_observation_tool_calls_without_workspace_change")]
    pub max_observation_tool_calls_without_workspace_change: usize,
    pub max_elapsed_ms: u64,
}

impl Default for AgentExecutionBudget {
    fn default() -> Self {
        Self {
            max_tool_rounds: DEFAULT_MAX_PROVIDER_TOOL_ROUNDS,
            max_total_tool_rounds: DEFAULT_MAX_TOTAL_PROVIDER_TOOL_ROUNDS,
            max_equivalent_tool_calls: DEFAULT_MAX_EQUIVALENT_TOOL_CALLS,
            max_observation_tool_calls_without_workspace_change:
                DEFAULT_MAX_OBSERVATION_TOOL_CALLS_WITHOUT_WORKSPACE_CHANGE,
            max_elapsed_ms: DEFAULT_MAX_TURN_ELAPSED_MS,
        }
    }
}

impl AgentExecutionBudget {
    fn normalized(mut self) -> Self {
        self.max_tool_rounds = self.max_tool_rounds.max(1);
        self.max_total_tool_rounds = self.max_total_tool_rounds.max(self.max_tool_rounds);
        self.max_equivalent_tool_calls = self.max_equivalent_tool_calls.max(1);
        self.max_observation_tool_calls_without_workspace_change = self
            .max_observation_tool_calls_without_workspace_change
            .max(1);
        self.max_elapsed_ms = self.max_elapsed_ms.max(1);
        self
    }

    fn status(
        &self,
        used_tool_rounds: usize,
        total_tool_rounds: usize,
        started_at: Instant,
        context_budget: Option<&ContextBudget>,
    ) -> AgentBudgetStatus {
        let elapsed_ms = started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        let context_used_tokens = context_budget.map(|budget| budget.used_tokens);
        let context_max_tokens = context_budget.map(|budget| budget.max_tokens);
        AgentBudgetStatus {
            max_tool_rounds: self.max_tool_rounds,
            used_tool_rounds,
            remaining_tool_rounds: self.max_tool_rounds.saturating_sub(used_tool_rounds),
            max_total_tool_rounds: self.max_total_tool_rounds,
            total_tool_rounds,
            remaining_total_tool_rounds: self
                .max_total_tool_rounds
                .saturating_sub(total_tool_rounds),
            max_equivalent_tool_calls: self.max_equivalent_tool_calls,
            max_observation_tool_calls_without_workspace_change: self
                .max_observation_tool_calls_without_workspace_change,
            max_elapsed_ms: self.max_elapsed_ms,
            elapsed_ms,
            remaining_time_ms: self.max_elapsed_ms.saturating_sub(elapsed_ms),
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
pub struct AgentBudgetStatus {
    pub max_tool_rounds: usize,
    pub used_tool_rounds: usize,
    pub remaining_tool_rounds: usize,
    pub max_total_tool_rounds: usize,
    pub total_tool_rounds: usize,
    pub remaining_total_tool_rounds: usize,
    pub max_equivalent_tool_calls: usize,
    pub max_observation_tool_calls_without_workspace_change: usize,
    pub max_elapsed_ms: u64,
    pub elapsed_ms: u64,
    pub remaining_time_ms: u64,
    pub context_max_tokens: Option<usize>,
    pub context_used_tokens: Option<usize>,
    pub context_remaining_tokens: Option<usize>,
}

fn default_max_total_tool_rounds() -> usize {
    DEFAULT_MAX_TOTAL_PROVIDER_TOOL_ROUNDS
}

fn default_max_equivalent_tool_calls() -> usize {
    DEFAULT_MAX_EQUIVALENT_TOOL_CALLS
}

fn default_max_observation_tool_calls_without_workspace_change() -> usize {
    DEFAULT_MAX_OBSERVATION_TOOL_CALLS_WITHOUT_WORKSPACE_CHANGE
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentLoopGuardState {
    pub total_tool_rounds: usize,
    pub blocked_equivalent_calls: usize,
    #[serde(default)]
    equivalent_call_counts: BTreeMap<String, usize>,
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
    #[serde(default)]
    observation_tool_calls_since_workspace_change: usize,
    #[serde(default)]
    implementation_mode: bool,
    #[serde(default)]
    blocked_implementation_mode_calls: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentBudgetCheckpointReason {
    ToolRounds,
    ElapsedTime,
    ContextWindow,
}

impl AgentBudgetCheckpointReason {
    fn message(self) -> &'static str {
        match self {
            Self::ToolRounds => "The tool-decision budget for this execution slice was reached.",
            Self::ElapsedTime => "The execution time budget for this slice was reached.",
            Self::ContextWindow => "The context budget for this execution slice was reached.",
        }
    }
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
    #[serde(default)]
    pub execution_budget: AgentExecutionBudget,
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
    BudgetCheckpoint,
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
        let execution_budget = AgentExecutionBudget::default().normalized();
        let started_at = Instant::now();

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
                        &execution_budget.status(0, 0, started_at, budget.as_ref()),
                        &input.workspace_root,
                        &self.sandbox_config,
                        false,
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
            execution_budget,
            loop_guard,
            started_at,
            input.store,
            input.cancellation,
            model_user_message,
            input.user_content,
            tool_candidates,
            provider_tool_calls,
            Vec::new(),
            response.tool_calls,
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

        let is_budget_checkpoint =
            continuation.continuation_kind == AgentContinuationKind::BudgetCheckpoint;
        let execution_budget = continuation.execution_budget.clone().normalized();
        let mut loop_guard = continuation.loop_guard.clone();
        let started_at = Instant::now();

        match continuation.state {
            AgentContinuationState::Provider {
                model_user_message,
                model_user_content,
                tool_candidates,
                provider_tool_calls,
                mut provider_tool_results,
                mut pending_tool_calls,
                current_round,
            } => {
                if is_budget_checkpoint {
                    if !approved {
                        events.push(AgentEventPayload::TurnFinished {
                            summary: "Budget checkpoint left paused by the user.".to_string(),
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
                            execution_budget,
                            loop_guard.clone(),
                            started_at,
                            store,
                            cancellation,
                            model_user_message,
                            model_user_content,
                            tool_candidates,
                            provider_tool_calls,
                            provider_tool_results,
                            pending_tool_calls,
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
                    let policy = Arc::new(BasicPolicyEngine::new(
                        continuation.workspace_root.clone(),
                        PermissionMode::FullAccess,
                    ));
                    let mut ctx = ToolContext::local_with_sandbox_config(
                        continuation.workspace_root.clone(),
                        policy,
                        crate::sandbox::LocalSandboxConfig::danger_full_access(),
                    );
                    ctx.store = store.clone();
                    ctx.thread_id = Some(continuation.thread_id);
                    ctx.cancel = cancellation.clone();
                    ctx.approval_granted = true;
                    self.apply_subagent_context(&mut ctx, continuation.user_message_id);
                    let result = self
                        .execute_provider_tool_call(&pending, ctx, &mut events)
                        .await?;
                    loop_guard.observe_tool_result(
                        &pending,
                        &result,
                        execution_budget.max_observation_tool_calls_without_workspace_change,
                    );
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
                    execution_budget,
                    loop_guard,
                    started_at,
                    store,
                    cancellation,
                    model_user_message,
                    model_user_content,
                    tool_candidates,
                    provider_tool_calls,
                    provider_tool_results,
                    pending_tool_calls,
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
        conversation: Vec<ModelConversationMessage>,
        permission_mode: PermissionMode,
        mut budget: Option<ContextBudget>,
        execution_budget: AgentExecutionBudget,
        mut loop_guard: AgentLoopGuardState,
        started_at: Instant,
        store: Option<Arc<dyn SessionStore>>,
        cancellation: Option<CancellationToken>,
        model_user_message: String,
        model_user_content: Vec<ModelContentPart>,
        tool_candidates: Vec<ProviderToolCandidate>,
        mut provider_tool_calls: Vec<ProviderToolCall>,
        mut provider_tool_results: Vec<ProviderToolResult>,
        mut pending_tool_calls: Vec<ProviderToolCall>,
        mut current_round: usize,
        events: &mut TurnEvents,
    ) -> anyhow::Result<AgentTurnResult> {
        loop {
            if let Some(reason) = budget_checkpoint_reason(
                &execution_budget,
                current_round,
                started_at,
                budget.as_ref(),
            ) {
                let completion_is_pending = pending_tool_calls
                    .first()
                    .is_some_and(|call| call.name == "complete_task");
                let total_budget_is_exhausted =
                    loop_guard.total_tool_rounds >= execution_budget.max_total_tool_rounds;
                let should_finish_without_checkpoint =
                    matches!(reason, AgentBudgetCheckpointReason::ToolRounds)
                        && (completion_is_pending || total_budget_is_exhausted);
                if !should_finish_without_checkpoint {
                    return Ok(suspend_for_budget_checkpoint(
                        thread_id,
                        user_message_id,
                        workspace_root,
                        context_summary,
                        conversation,
                        permission_mode,
                        budget,
                        execution_budget,
                        loop_guard,
                        started_at,
                        reason,
                        model_user_message,
                        model_user_content,
                        tool_candidates,
                        provider_tool_calls,
                        provider_tool_results,
                        pending_tool_calls,
                        current_round,
                        std::mem::replace(events, TurnEvents::new(None)),
                    ));
                }
            }

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
                if loop_guard.implementation_mode
                    && !tool_call_can_change_workspace(&provider_call)
                    && provider_call.name != "complete_task"
                {
                    loop_guard.blocked_implementation_mode_calls += 1;
                    let result = blocked_implementation_mode_tool_result(&provider_call, events);
                    if let Some(ref mut budget) = budget {
                        budget.record_tokens(ContextBudget::estimate_tokens(&result.output));
                    }
                    provider_tool_results.push(result);
                    pending_tool_calls.remove(0);
                    if loop_guard.blocked_implementation_mode_calls
                        >= MAX_IMPLEMENTATION_MODE_VIOLATIONS
                    {
                        let output = incomplete_stagnation_output(&loop_guard);
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
                if let Some(equivalent_call_count) = loop_guard
                    .register_tool_call(&provider_call, execution_budget.max_equivalent_tool_calls)
                {
                    let result = blocked_equivalent_tool_result(
                        &provider_call,
                        equivalent_call_count,
                        execution_budget.max_equivalent_tool_calls,
                        events,
                    );
                    if let Some(ref mut budget) = budget {
                        budget.record_tokens(ContextBudget::estimate_tokens(&result.output));
                    }
                    provider_tool_results.push(result);
                    pending_tool_calls.remove(0);
                    continue;
                }

                let policy = Arc::new(BasicPolicyEngine::new(
                    workspace_root.clone(),
                    permission_mode,
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
                        loop_guard.observe_tool_result(
                            &provider_call,
                            &result,
                            execution_budget.max_observation_tool_calls_without_workspace_change,
                        );
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
                                    execution_budget: execution_budget.clone(),
                                    loop_guard,
                                    continuation_kind: AgentContinuationKind::Approval,
                                    state: AgentContinuationState::Provider {
                                        model_user_message,
                                        model_user_content,
                                        tool_candidates,
                                        provider_tool_calls,
                                        provider_tool_results,
                                        pending_tool_calls,
                                        current_round,
                                    },
                                },
                            },
                        });
                    }
                    Err(err) => return Err(err),
                }
            }

            let total_budget_exhausted =
                loop_guard.total_tool_rounds >= execution_budget.max_total_tool_rounds;
            let response = self
                .complete_model(
                    ModelRequest {
                        system_prompt: provider_system_prompt(
                            &execution_budget.status(
                                current_round,
                                loop_guard.total_tool_rounds,
                                started_at,
                                budget.as_ref(),
                            ),
                            &workspace_root,
                            &self.sandbox_config,
                            loop_guard.completion_mode,
                            loop_guard.implementation_mode,
                        ),
                        conversation: conversation.clone(),
                        user_message: model_user_message.clone(),
                        user_content: model_user_content.clone(),
                        tool_candidates: if !total_budget_exhausted
                            && current_round < execution_budget.max_tool_rounds
                        {
                            if loop_guard.completion_mode {
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
                            } else if loop_guard.implementation_mode {
                                tool_candidates
                                    .iter()
                                    .filter(|candidate| {
                                        matches!(
                                            candidate.name.as_str(),
                                            "write_file"
                                                | "apply_patch"
                                                | "spreadsheet"
                                                | "complete_task"
                                        )
                                    })
                                    .cloned()
                                    .collect()
                            } else {
                                tool_candidates.clone()
                            }
                        } else {
                            Vec::new()
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

            if response.tool_calls.is_empty() || total_budget_exhausted {
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
        max_equivalent_tool_calls: usize,
    ) -> Option<usize> {
        if call.name == "complete_task" {
            return None;
        }
        let signature = provider_tool_call_signature(call);
        let count = self.equivalent_call_counts.entry(signature).or_default();
        *count += 1;
        if *count > max_equivalent_tool_calls {
            self.blocked_equivalent_calls += 1;
            Some(*count)
        } else {
            None
        }
    }

    fn observe_tool_result(
        &mut self,
        call: &ProviderToolCall,
        result: &ProviderToolResult,
        max_observation_tool_calls_without_workspace_change: usize,
    ) {
        let workspace_mutation_succeeded = !result.is_error && tool_call_can_change_workspace(call);
        if workspace_mutation_succeeded {
            self.observation_tool_calls_since_workspace_change = 0;
            self.implementation_mode = false;
            self.blocked_implementation_mode_calls = 0;
        } else if call.name != "complete_task" {
            self.observation_tool_calls_since_workspace_change += 1;
        }

        if result.is_error {
            self.refresh_implementation_mode(max_observation_tool_calls_without_workspace_change);
            return;
        }

        let verification_command = successful_verification_command(call);
        if let Some(command) = verification_command.as_ref() {
            self.last_successful_verification = Some(command.clone());
        }

        let changed_state = if call.name == "update_plan" {
            self.latest_plan = result
                .metadata
                .get("taskPlan")
                .and_then(|value| serde_json::from_value(value.clone()).ok());
            let fingerprint = result.metadata.get("taskPlan").map(canonical_json_string);
            let changed = fingerprint != self.last_plan_fingerprint;
            self.last_plan_fingerprint = fingerprint;
            changed
        } else {
            workspace_mutation_succeeded
        };

        if changed_state {
            self.equivalent_call_counts.clear();
            if call.name != "update_plan" {
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
            self.implementation_mode = false;
            self.workspace_changed_since_verification = false;
        }
        self.refresh_implementation_mode(max_observation_tool_calls_without_workspace_change);
    }

    fn refresh_implementation_mode(
        &mut self,
        max_observation_tool_calls_without_workspace_change: usize,
    ) {
        if !self.completion_mode
            && self.observation_tool_calls_since_workspace_change
                >= max_observation_tool_calls_without_workspace_change
            && self
                .latest_plan
                .as_ref()
                .is_some_and(plan_needs_workspace_change)
        {
            self.implementation_mode = true;
        }
    }
}

fn provider_tool_call_signature(call: &ProviderToolCall) -> String {
    format!("{}:{}", call.name, canonical_json_string(&call.arguments))
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

fn plan_needs_workspace_change(plan: &TaskPlan) -> bool {
    plan.steps.iter().any(|step| {
        step.status != TaskPlanStepStatus::Completed && step_looks_like_workspace_change(&step.step)
    })
}

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

fn blocked_equivalent_tool_result(
    provider_call: &ProviderToolCall,
    equivalent_call_count: usize,
    max_equivalent_tool_calls: usize,
    events: &mut TurnEvents,
) -> ProviderToolResult {
    let call = ToolCall::new(&provider_call.name, provider_call.arguments.clone());
    events.push(AgentEventPayload::ToolCallStarted { call: call.clone() });
    let output = format!(
        "Equivalent tool call blocked after {max_equivalent_tool_calls} executions without an intervening state change. Do not retry the same call. Make a state-changing step, use different evidence, or finish the task."
    );
    let metadata = json!({
        "toolName": &provider_call.name,
        "providerToolCallId": &provider_call.id,
        "success": false,
        "loopGuardBlocked": true,
        "equivalentCallCount": equivalent_call_count,
        "maxEquivalentToolCalls": max_equivalent_tool_calls
    });
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

fn blocked_implementation_mode_tool_result(
    provider_call: &ProviderToolCall,
    events: &mut TurnEvents,
) -> ProviderToolResult {
    let call = ToolCall::new(&provider_call.name, provider_call.arguments.clone());
    events.push(AgentEventPayload::ToolCallStarted { call: call.clone() });
    let output = "Tool call blocked because exploration has continued without a workspace change. Implementation mode only permits write_file, apply_patch, spreadsheet write, or complete_task. Make the planned concrete change now.".to_string();
    let metadata = json!({
        "toolName": &provider_call.name,
        "providerToolCallId": &provider_call.id,
        "success": false,
        "implementationModeBlocked": true
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

fn incomplete_stagnation_output(loop_guard: &AgentLoopGuardState) -> String {
    let plan = loop_guard.latest_plan.as_ref();
    let remaining = plan
        .map(|plan| {
            plan.steps
                .iter()
                .filter(|step| step.status != TaskPlanStepStatus::Completed)
                .count()
        })
        .unwrap_or(0);
    format!(
        "Task remains incomplete. The turn was stopped after {MAX_IMPLEMENTATION_MODE_VIOLATIONS} implementation-mode violations without a workspace change. The durable plan retains {remaining} unfinished step(s) for a retry or user-directed continuation."
    )
}

fn budget_checkpoint_reason(
    execution_budget: &AgentExecutionBudget,
    used_tool_rounds: usize,
    started_at: Instant,
    context_budget: Option<&ContextBudget>,
) -> Option<AgentBudgetCheckpointReason> {
    if used_tool_rounds >= execution_budget.max_tool_rounds {
        return Some(AgentBudgetCheckpointReason::ToolRounds);
    }
    if started_at.elapsed().as_millis() >= u128::from(execution_budget.max_elapsed_ms) {
        return Some(AgentBudgetCheckpointReason::ElapsedTime);
    }
    context_budget
        .filter(|budget| budget.is_exceeded())
        .map(|_| AgentBudgetCheckpointReason::ContextWindow)
}

#[allow(clippy::too_many_arguments)]
fn suspend_for_budget_checkpoint(
    thread_id: Uuid,
    user_message_id: Uuid,
    workspace_root: PathBuf,
    context_summary: Option<String>,
    conversation: Vec<ModelConversationMessage>,
    permission_mode: PermissionMode,
    context_budget: Option<ContextBudget>,
    execution_budget: AgentExecutionBudget,
    loop_guard: AgentLoopGuardState,
    started_at: Instant,
    checkpoint_reason: AgentBudgetCheckpointReason,
    model_user_message: String,
    model_user_content: Vec<ModelContentPart>,
    tool_candidates: Vec<ProviderToolCandidate>,
    provider_tool_calls: Vec<ProviderToolCall>,
    provider_tool_results: Vec<ProviderToolResult>,
    pending_tool_calls: Vec<ProviderToolCall>,
    used_tool_rounds: usize,
    mut events: TurnEvents,
) -> AgentTurnResult {
    let approval_id = Uuid::new_v4();
    let status = execution_budget.status(
        used_tool_rounds,
        loop_guard.total_tool_rounds,
        started_at,
        context_budget.as_ref(),
    );
    let reason = format!(
        "{} Continue to grant another execution slice (slice rounds remaining: {}, total rounds remaining: {}, time remaining: {} ms, context remaining: {}).",
        checkpoint_reason.message(),
        status.remaining_tool_rounds,
        status.remaining_total_tool_rounds,
        status.remaining_time_ms,
        status
            .context_remaining_tokens
            .map(|tokens| tokens.to_string())
            .unwrap_or_else(|| "not tracked".to_string()),
    );
    events.push(AgentEventPayload::ApprovalRequested {
        approval_id,
        action: "Continue agent execution".to_string(),
        reason: reason.clone(),
    });
    events.push(AgentEventPayload::TurnSuspended {
        approval_id,
        reason,
    });
    AgentTurnResult {
        events: events.into_vec(),
        outcome: AgentTurnOutcome::Suspended {
            approval_id,
            continuation: AgentContinuation {
                thread_id,
                user_message_id,
                workspace_root,
                context_summary,
                conversation,
                permission_mode,
                context_budget,
                execution_budget,
                loop_guard,
                continuation_kind: AgentContinuationKind::BudgetCheckpoint,
                state: AgentContinuationState::Provider {
                    model_user_message,
                    model_user_content,
                    tool_candidates,
                    provider_tool_calls,
                    provider_tool_results,
                    pending_tool_calls,
                    // A resumed slice receives a fresh execution budget. Prior tool calls remain
                    // in the provider history, but they do not consume the new slice's allowance.
                    current_round: 0,
                },
            },
        },
    }
}

fn provider_system_prompt(
    status: &AgentBudgetStatus,
    workspace_root: &Path,
    sandbox_config: &LocalSandboxConfig,
    completion_mode: bool,
    implementation_mode: bool,
) -> String {
    let completion_instruction = if completion_mode {
        " A terminal verification command just succeeded. Completion mode is active: only update_plan and complete_task are available. Do not resume implementation or investigation. Submit the truthful final plan state now, or use complete_task if the requested scope is actually complete."
    } else {
        ""
    };
    let implementation_instruction = if implementation_mode {
        " Exploration has exceeded the no-progress limit. Implementation mode is active: only write_file, apply_patch, spreadsheet write, and complete_task are available. Make the concrete workspace change required by the current plan now. Do not inspect more files, run tests, or revise the plan until a workspace change succeeds."
    } else {
        ""
    };
    let workspace_scope = workspace_scope_instruction(workspace_root, sandbox_config);
    format!(
        "You are OpenTopia, a tool-using AI agent. Decide for yourself whether to observe, which available tools to call, how to validate their results, and when the task is complete. The harness provides capabilities, policy boundaries, isolation, and observability; it does not prescribe a workflow. Use tools only when they materially help. {workspace_scope} For non-trivial multi-step work, use update_plan as durable task memory and keep it current. Complete every step in the current requested scope before claiming completion; steps explicitly deferred by the user to a later phase may remain pending. After a successful test, build, lint, check, or verify command, finish with one final action: update_plan with all current steps completed (set current_scope_complete with verification when later-phase steps remain pending), or call complete_task with a concise summary, concrete verification evidence, and any deliberately deferred work. Do not perform more investigation after that final action. Delegate independent work with spawn_agent when useful, then use wait_agents to collect parallel results before synthesizing the final answer. Inspect every child status and error; a terminal child is not necessarily a successful child.{completion_instruction}{implementation_instruction}\n\nExecution budget for this slice: {}/{} tool-decision rounds used ({} remaining). Total tool-decision budget: {}/{} used ({} remaining). Equivalent tool calls without an intervening state change are limited to {}. Observation tool calls without a workspace change are limited to {} before implementation mode activates. Time budget: {} ms remaining of {} ms. Context budget: {} used of {} tokens ({} remaining). A slice budget checkpoint requires explicit continuation. When the total tool budget reaches zero, stop using tools and provide the best truthful final response. Do not claim work is complete merely because a budget is low.",
        status.used_tool_rounds,
        status.max_tool_rounds,
        status.remaining_tool_rounds,
        status.total_tool_rounds,
        status.max_total_tool_rounds,
        status.remaining_total_tool_rounds,
        status.max_equivalent_tool_calls,
        status.max_observation_tool_calls_without_workspace_change,
        status.remaining_time_ms,
        status.max_elapsed_ms,
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
        let execution_budget = AgentExecutionBudget::default().normalized();
        let prompt = provider_system_prompt(
            &execution_budget.status(0, 0, Instant::now(), None),
            &workspace,
            &sandbox_config,
            false,
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
            &execution_budget.status(0, 0, Instant::now(), None),
            &workspace,
            &LocalSandboxConfig::danger_full_access(),
            false,
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
    async fn stagnant_exploration_requires_a_workspace_change() {
        let workspace = test_workspace("implementation-mode");
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
            .expect("implementation mode recovers after a workspace change");

        assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
        let requests = provider.requests();
        assert_eq!(requests.len(), 5);
        for request in &requests[2..4] {
            let mut tool_names = request
                .tool_candidates
                .iter()
                .map(|candidate| candidate.name.as_str())
                .collect::<Vec<_>>();
            tool_names.sort_unstable();
            assert_eq!(
                tool_names,
                vec!["apply_patch", "complete_task", "spreadsheet", "write_file"]
            );
            assert!(request
                .system_prompt
                .contains("Implementation mode is active"));
        }
        assert!(requests[4]
            .tool_candidates
            .iter()
            .any(|candidate| candidate.name == "read_file"));
        assert!(result.events.iter().any(|event| matches!(
            event,
            AgentEventPayload::ToolCallFinished { result }
                if result.metadata.get("implementationModeBlocked") == Some(&Value::Bool(true))
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

    #[tokio::test]
    async fn repeated_implementation_mode_violations_stop_as_incomplete() {
        let workspace = test_workspace("implementation-mode-stop");
        for index in 0..11 {
            fs::write(workspace.join(format!("context-{index}.txt")), "context").unwrap();
        }
        let mut responses = vec![
            ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: "call_plan".to_string(),
                    name: "update_plan".to_string(),
                    arguments: json!({
                        "plan": [
                            { "step": "Implement CLI contract", "status": "in_progress" },
                            { "step": "Run tests", "status": "pending" }
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
        ];
        responses.extend((1..=3).map(|index| ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: format!("call_violation_{index}"),
                name: "list_files".to_string(),
                arguments: json!({ "path": "." }),
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
                    content: "Implement the CLI.".to_string(),
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
            .expect("stagnation stop closes the turn");

        assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
        assert_eq!(provider.requests().len(), 5);
        assert!(assistant_text(&result.events).contains("Task remains incomplete"));
        assert!(assistant_text(&result.events).contains("3 implementation-mode violations"));

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
    async fn approved_protected_metadata_write_uses_one_shot_sandbox_escalation() {
        let workspace = test_workspace("approved-sandbox-escalation");
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
            .expect("approved sandbox escalation resumes");

        assert!(matches!(resumed.outcome, AgentTurnOutcome::Completed));
        assert_eq!(
            fs::read_to_string(workspace.join(".codex/config.toml")).unwrap(),
            "approved metadata"
        );
        let _ = fs::remove_dir_all(workspace);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn sandbox_blocked_shell_write_resumes_with_one_shot_approval() {
        let workspace = test_workspace("approved-shell-sandbox-escalation");
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
        let agent = AgentCore::new(provider, ToolRegistry::with_builtins());

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
            .expect("approved sandbox escalation resumes");

        assert!(matches!(resumed.outcome, AgentTurnOutcome::Completed));
        assert_eq!(
            fs::read_to_string(&outside).unwrap().trim(),
            "approved-shell"
        );
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
    async fn tool_budget_suspends_without_a_harness_summary_and_resumes() {
        let workspace = test_workspace("tool-budget-checkpoint");
        fs::write(workspace.join("sample.txt"), "checkpoint content").unwrap();
        let tool_responses = (0..8)
            .map(|index| ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: format!("call_{index}"),
                    name: "read_file".to_string(),
                    arguments: json!({ "path": "sample.txt" }),
                }],
                usage: None,
            })
            .collect::<Vec<_>>();
        let provider = Arc::new(ScriptedProvider::new(
            tool_responses
                .into_iter()
                .chain(std::iter::once(ModelResponse::text(
                    "Completed after the user continued the checkpoint.",
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
            .expect("turn checkpoints at the tool budget");

        assert!(!result
            .events
            .iter()
            .any(|event| matches!(event, AgentEventPayload::AssistantMessage { .. })));
        assert!(!result.events.iter().any(|event| matches!(
            event,
            AgentEventPayload::ModelDelta { text }
                if text.contains("tool-call budget ended")
        )));
        let continuation = match result.outcome {
            AgentTurnOutcome::Suspended { continuation, .. } => continuation,
            AgentTurnOutcome::Completed => panic!("turn should pause at the budget"),
        };
        assert_eq!(
            continuation.continuation_kind,
            AgentContinuationKind::BudgetCheckpoint
        );
        match &continuation.state {
            AgentContinuationState::Provider {
                pending_tool_calls,
                current_round,
                ..
            } => {
                assert_eq!(pending_tool_calls.len(), 1);
                assert_eq!(pending_tool_calls[0].id, "call_7");
                assert_eq!(*current_round, 0);
            }
        }
        assert!(provider.requests()[0]
            .system_prompt
            .contains("Execution budget for this slice: 0/8"));

        let resumed = agent
            .resume_turn_streaming(continuation, true, None, None, None)
            .await
            .expect("budget checkpoint resumes");
        assert!(matches!(resumed.outcome, AgentTurnOutcome::Completed));
        assert!(assistant_text(&resumed.events).contains("Completed after the user continued"));
        assert_eq!(provider.requests().len(), 9);

        let _ = fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn total_tool_budget_forces_a_final_response_across_continuations() {
        let workspace = test_workspace("total-tool-budget");
        fs::write(workspace.join("sample.txt"), "bounded content").unwrap();
        let responses = (0..DEFAULT_MAX_TOTAL_PROVIDER_TOOL_ROUNDS)
            .map(|index| ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: format!("call_{index}"),
                    name: "read_file".to_string(),
                    arguments: json!({ "path": "sample.txt" }),
                }],
                usage: None,
            })
            .chain(std::iter::once(ModelResponse::text(
                "Stopped at the total tool budget.",
            )))
            .collect();
        let provider = Arc::new(ScriptedProvider::new(responses));
        let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());
        let mut result = agent
            .run_turn_detailed_streaming(
                AgentTurnInput {
                    thread_id: Uuid::new_v4(),
                    user_message_id: Uuid::new_v4(),
                    workspace_root: workspace.clone(),
                    content: "Exercise the total loop guard.".to_string(),
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
            .expect("initial slice runs");
        let mut checkpoints = 0;

        loop {
            let continuation = match result.outcome {
                AgentTurnOutcome::Completed => break,
                AgentTurnOutcome::Suspended { continuation, .. } => continuation,
            };
            checkpoints += 1;
            assert_eq!(
                continuation.continuation_kind,
                AgentContinuationKind::BudgetCheckpoint
            );
            result = agent
                .resume_turn_streaming(continuation, true, None, None, None)
                .await
                .expect("budget continuation runs");
        }

        assert_eq!(checkpoints, 2);
        let requests = provider.requests();
        assert_eq!(requests.len(), DEFAULT_MAX_TOTAL_PROVIDER_TOOL_ROUNDS + 1);
        let final_request = requests.last().expect("final provider request");
        assert!(final_request.tool_candidates.is_empty());
        assert!(final_request
            .system_prompt
            .contains("Total tool-decision budget: 24/24"));

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
