use crate::mcp::McpToolDescriptor;
use crate::mcp_host::McpExtensionHost;
use crate::model::{AgentEventPayload, ContextSummary, Message, MessageRole, ToolCall, ToolResult};
use crate::policy::{BasicPolicyEngine, PermissionMode};
use crate::provider::{
    MockProvider, ModelConversationMessage, ModelProvider, ModelRequest, ModelResponse,
    ModelStreamDelta, OpenAiCompatibleProvider, ProviderToolCall, ProviderToolCandidate,
    ProviderToolResult,
};
use crate::sandbox::LocalSandboxConfig;
use crate::settings::{AppSettings, ProviderKind};
use crate::store::SessionStore;
use crate::subagents::SubagentScheduler;
use crate::tools::{McpToolWrapper, ToolContext, ToolRegistry};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const MAX_PROVIDER_TOOL_ROUNDS: usize = 8;

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
pub struct AgentContinuation {
    pub thread_id: Uuid,
    pub user_message_id: Uuid,
    pub workspace_root: PathBuf,
    pub context_summary: Option<String>,
    pub conversation: Vec<ModelConversationMessage>,
    pub permission_mode: PermissionMode,
    pub context_budget: Option<ContextBudget>,
    pub state: AgentContinuationState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentContinuationState {
    DirectCommand {
        content: String,
    },
    Provider {
        model_user_message: String,
        listed_files: String,
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

        events.push(AgentEventPayload::TurnStarted {
            user_message_id: input.user_message_id,
        });

        let policy = Arc::new(BasicPolicyEngine::new(
            input.workspace_root.clone(),
            input.permission_mode,
        ));

        if let Some(ref mut budget) = budget {
            let input_tokens = ContextBudget::estimate_tokens(&input.content);
            budget.record_tokens(input_tokens);
        }

        if let Some(task) = ParsedTask::parse(&input.content) {
            let mut tool_ctx = ToolContext::local_with_sandbox_config(
                input.workspace_root.clone(),
                policy,
                self.sandbox_config.clone(),
            );
            tool_ctx.store = input.store.clone();
            tool_ctx.thread_id = Some(input.thread_id);
            tool_ctx.cancel = input.cancellation.clone();
            self.apply_subagent_context(&mut tool_ctx, input.user_message_id);

            let result = match self.execute_parsed_task(task, tool_ctx, &mut events).await {
                Ok(result) => {
                    if let Some(ref mut budget) = budget {
                        let output_tokens = ContextBudget::estimate_tokens(&result);
                        budget.record_tokens(output_tokens);
                        if budget.is_exceeded() {
                            events.push(AgentEventPayload::ContextCompacted {
                                summary: ContextSummary::new(
                                    input.thread_id,
                                    0,
                                    1,
                                    format!(
                                        "Context budget exceeded ({} / {} tokens). Consider summarizing earlier turns.",
                                        budget.used_tokens, budget.max_tokens
                                    ),
                                ),
                            });
                        } else {
                            for warning in &budget.warnings {
                                events.push(AgentEventPayload::ModelDelta {
                                    text: format!("**Context budget warning:** {}\n", warning),
                                });
                            }
                        }
                    }
                    result
                }
                Err(err) if err.to_string().contains("approval required") => {
                    let reason = err.to_string();
                    let approval_id = Uuid::new_v4();
                    events.push(AgentEventPayload::ApprovalRequested {
                        approval_id,
                        reason: reason.clone(),
                        action: input.content.clone(),
                    });
                    events.push(AgentEventPayload::TurnSuspended {
                        approval_id,
                        reason,
                    });
                    return Ok(AgentTurnResult {
                        events: events.into_vec(),
                        outcome: AgentTurnOutcome::Suspended {
                            approval_id,
                            continuation: AgentContinuation {
                                thread_id: input.thread_id,
                                user_message_id: input.user_message_id,
                                workspace_root: input.workspace_root,
                                context_summary: input.context_summary,
                                conversation: input.conversation,
                                permission_mode: input.permission_mode,
                                context_budget: budget,
                                state: AgentContinuationState::DirectCommand {
                                    content: input.content,
                                },
                            },
                        },
                    });
                }
                Err(err) => return Err(err),
            };
            let assistant_message = Message::text(input.thread_id, MessageRole::Assistant, result);
            events.push(AgentEventPayload::AssistantMessage {
                message: assistant_message,
            });
            events.push(AgentEventPayload::TurnFinished {
                summary: "Command task completed.".to_string(),
            });
            return Ok(AgentTurnResult {
                events: events.into_vec(),
                outcome: AgentTurnOutcome::Completed,
            });
        }

        let listed_files = self
            .execute_tool(
                "list_files",
                json!({ "path": "." }),
                {
                    let mut ctx = ToolContext::local_with_sandbox_config(
                        input.workspace_root.clone(),
                        policy.clone(),
                        self.sandbox_config.clone(),
                    );
                    ctx.store = input.store.clone();
                    ctx.thread_id = Some(input.thread_id);
                    ctx.cancel = input.cancellation.clone();
                    self.apply_subagent_context(&mut ctx, input.user_message_id);
                    ctx
                },
                &mut events,
            )
            .await?
            .output;

        if let Some(ref mut budget) = budget {
            let list_tokens = ContextBudget::estimate_tokens(&listed_files);
            budget.record_tokens(list_tokens);
        }

        let model_user_message = provider_user_message(
            &input.content,
            &listed_files,
            input.context_summary.as_deref(),
        );
        let tool_candidates = self.provider_tool_candidates();
        let response = self
            .complete_model(
                ModelRequest {
                    system_prompt: provider_system_prompt(),
                    conversation: input.conversation.clone(),
                    user_message: model_user_message.clone(),
                    tool_candidates: tool_candidates.clone(),
                    previous_tool_calls: Vec::new(),
                    tool_results: Vec::new(),
                },
                &mut events,
            )
            .await?;
        if response.tool_calls.is_empty() {
            return Ok(finalize_provider_turn(
                input.thread_id,
                listed_files,
                response,
                Vec::new(),
                budget,
                events,
            ));
        }

        let provider_tool_calls = response.tool_calls.clone();
        self.continue_provider_turn(
            input.thread_id,
            input.user_message_id,
            input.workspace_root,
            input.context_summary,
            input.conversation,
            input.permission_mode,
            budget,
            input.store,
            input.cancellation,
            model_user_message,
            listed_files,
            tool_candidates,
            provider_tool_calls,
            Vec::new(),
            response.tool_calls,
            0,
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

        match continuation.state {
            AgentContinuationState::DirectCommand { content } => {
                if !approved {
                    let message = Message::text(
                        continuation.thread_id,
                        MessageRole::Assistant,
                        "Approval was denied, so the requested action was not executed.",
                    );
                    events.push(AgentEventPayload::AssistantMessage { message });
                    events.push(AgentEventPayload::TurnFinished {
                        summary: "Approval denied; command skipped.".to_string(),
                    });
                    return Ok(AgentTurnResult {
                        events: events.into_vec(),
                        outcome: AgentTurnOutcome::Completed,
                    });
                }

                let task = ParsedTask::parse(&content)
                    .ok_or_else(|| anyhow::anyhow!("approved command is no longer parseable"))?;
                let policy = Arc::new(BasicPolicyEngine::new(
                    continuation.workspace_root.clone(),
                    PermissionMode::FullAccess,
                ));
                let mut ctx = ToolContext::local_with_sandbox_config(
                    continuation.workspace_root,
                    policy,
                    crate::sandbox::LocalSandboxConfig::danger_full_access(),
                );
                ctx.store = store;
                ctx.thread_id = Some(continuation.thread_id);
                ctx.cancel = cancellation;
                self.apply_subagent_context(&mut ctx, continuation.user_message_id);
                let result = self.execute_parsed_task(task, ctx, &mut events).await?;
                let message = Message::text(continuation.thread_id, MessageRole::Assistant, result);
                events.push(AgentEventPayload::AssistantMessage { message });
                events.push(AgentEventPayload::TurnFinished {
                    summary: "Approved command completed.".to_string(),
                });
                Ok(AgentTurnResult {
                    events: events.into_vec(),
                    outcome: AgentTurnOutcome::Completed,
                })
            }
            AgentContinuationState::Provider {
                model_user_message,
                listed_files,
                tool_candidates,
                provider_tool_calls,
                mut provider_tool_results,
                mut pending_tool_calls,
                current_round,
            } => {
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
                    self.apply_subagent_context(&mut ctx, continuation.user_message_id);
                    provider_tool_results.push(
                        self.execute_provider_tool_call(&pending, ctx, &mut events)
                            .await?,
                    );
                } else {
                    provider_tool_results.push(ProviderToolResult {
                        call_id: pending.id.clone(),
                        name: pending.name.clone(),
                        output: "The user denied this tool call.".to_string(),
                        is_error: true,
                        metadata: json!({ "approvalDenied": true }),
                    });
                }

                self.continue_provider_turn(
                    continuation.thread_id,
                    continuation.user_message_id,
                    continuation.workspace_root,
                    continuation.context_summary,
                    continuation.conversation,
                    continuation.permission_mode,
                    continuation.context_budget,
                    store,
                    cancellation,
                    model_user_message,
                    listed_files,
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
        store: Option<Arc<dyn SessionStore>>,
        cancellation: Option<CancellationToken>,
        model_user_message: String,
        listed_files: String,
        tool_candidates: Vec<ProviderToolCandidate>,
        mut provider_tool_calls: Vec<ProviderToolCall>,
        mut provider_tool_results: Vec<ProviderToolResult>,
        mut pending_tool_calls: Vec<ProviderToolCall>,
        mut current_round: usize,
        events: &mut TurnEvents,
    ) -> anyhow::Result<AgentTurnResult> {
        loop {
            while let Some(provider_call) = pending_tool_calls.first().cloned() {
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
                        provider_tool_results.push(result);
                        pending_tool_calls.remove(0);
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
                                    state: AgentContinuationState::Provider {
                                        model_user_message,
                                        listed_files,
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

            let mut response = self
                .complete_model(
                    ModelRequest {
                        system_prompt: provider_system_prompt(),
                        conversation: conversation.clone(),
                        user_message: model_user_message.clone(),
                        tool_candidates: if current_round + 1 < MAX_PROVIDER_TOOL_ROUNDS {
                            tool_candidates.clone()
                        } else {
                            Vec::new()
                        },
                        previous_tool_calls: provider_tool_calls.clone(),
                        tool_results: provider_tool_results.clone(),
                    },
                    events,
                )
                .await?;

            if response.tool_calls.is_empty() {
                return Ok(finalize_provider_turn(
                    thread_id,
                    listed_files,
                    response,
                    provider_tool_results,
                    budget,
                    std::mem::replace(events, TurnEvents::new(None)),
                ));
            }

            current_round += 1;
            if current_round >= MAX_PROVIDER_TOOL_ROUNDS {
                events.push(AgentEventPayload::ModelDelta {
                    text: format!(
                        "Provider reached the autonomous tool limit of {} rounds.\n",
                        MAX_PROVIDER_TOOL_ROUNDS
                    ),
                });
                if response.text.trim().is_empty() {
                    response =
                        ModelResponse::text(local_provider_tool_summary(&provider_tool_results));
                }
                return Ok(finalize_provider_turn(
                    thread_id,
                    listed_files,
                    response,
                    provider_tool_results,
                    budget,
                    std::mem::replace(events, TurnEvents::new(None)),
                ));
            }

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
        self.tools
            .list()
            .into_iter()
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
                Ok(ProviderToolResult {
                    call_id: provider_call.id.clone(),
                    name: provider_call.name.clone(),
                    output: result.output,
                    is_error,
                    metadata: result.metadata,
                })
            }
            Err(err) if err.to_string().contains("approval required") => Err(err),
            Err(err) => Ok(ProviderToolResult {
                call_id: provider_call.id.clone(),
                name: provider_call.name.clone(),
                output: err.to_string(),
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

    async fn execute_parsed_task(
        &self,
        task: ParsedTask,
        ctx: ToolContext,
        events: &mut TurnEvents,
    ) -> anyhow::Result<String> {
        let result = match task {
            ParsedTask::List { path } => {
                self.execute_tool("list_files", json!({ "path": path }), ctx, events)
                    .await?
            }
            ParsedTask::Read { path } => {
                self.execute_tool("read_file", json!({ "path": path }), ctx, events)
                    .await?
            }
            ParsedTask::Search { path, query } => {
                self.execute_tool(
                    "search",
                    json!({ "path": path, "query": query }),
                    ctx,
                    events,
                )
                .await?
            }
            ParsedTask::Write { path, content } => {
                let result = self
                    .execute_tool(
                        "write_file",
                        json!({ "path": path, "content": content }),
                        ctx,
                        events,
                    )
                    .await?;
                if let Some(changed_path) = result
                    .metadata
                    .get("changedPath")
                    .and_then(|value| value.as_str())
                {
                    events.push(AgentEventPayload::FileChanged {
                        path: changed_path.into(),
                        summary: "File written by write_file tool.".to_string(),
                    });
                }
                result
            }
            ParsedTask::Run { command } => {
                self.execute_tool("shell", json!({ "command": command }), ctx, events)
                    .await?
            }
            ParsedTask::Diff => {
                self.execute_tool("git_diff", json!({}), ctx, events)
                    .await?
            }
            ParsedTask::Patch { patch } => {
                let result = self
                    .execute_tool("apply_patch", json!({ "patch": patch }), ctx, events)
                    .await?;
                events.push(AgentEventPayload::FileChanged {
                    path: input_placeholder_path(),
                    summary: "Patch applied by apply_patch tool.".to_string(),
                });
                result
            }
            ParsedTask::Mcp {
                public_name,
                arguments,
            } => {
                self.execute_tool(&public_name, arguments, ctx, events)
                    .await?
            }
            ParsedTask::Invalid { reason } => anyhow::bail!("{reason}"),
        };

        Ok(format!(
            "Completed `{}`.\n\n```text\n{}\n```",
            result
                .metadata
                .get("toolName")
                .and_then(|value| value.as_str())
                .unwrap_or("tool"),
            result.output
        ))
    }

    async fn execute_tool(
        &self,
        name: &str,
        input: Value,
        ctx: ToolContext,
        events: &mut TurnEvents,
    ) -> anyhow::Result<crate::model::ToolResult> {
        let call = ToolCall::new(name, input);
        self.execute_tool_call(call, ctx, events, None).await
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
        Ok(result)
    }
}

fn finalize_provider_turn(
    thread_id: Uuid,
    _listed_files: String,
    response: ModelResponse,
    provider_tool_results: Vec<ProviderToolResult>,
    mut budget: Option<ContextBudget>,
    mut events: TurnEvents,
) -> AgentTurnResult {
    if let Some(ref mut budget) = budget {
        for result in &provider_tool_results {
            budget.record_tokens(ContextBudget::estimate_tokens(&result.output));
        }
        budget.record_tokens(ContextBudget::estimate_tokens(&response.text));
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

fn provider_system_prompt() -> String {
    "You are OpenTopia, a local-first coding agent. Use the provided tools when you need workspace context or need to perform a permitted local action. After tool results are provided, write a concise assistant summary for the user.".to_string()
}

fn provider_user_message(
    user_content: &str,
    listed_files: &str,
    context_summary: Option<&str>,
) -> String {
    let durable_context = context_summary
        .map(|summary| format!("Durable context from earlier turns:\n{summary}\n\n"))
        .unwrap_or_default();
    format!(
        "{durable_context}User request:\n{}\n\nWorkspace root listing:\n{}",
        user_content, listed_files
    )
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
        "The provider requested another tool round, so OpenTopia stopped after the first autonomous round. Results from the completed tool call(s):\n\n{}",
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
    pub context_summary: Option<String>,
    pub conversation: Vec<ModelConversationMessage>,
    pub permission_mode: PermissionMode,
    pub context_budget: Option<ContextBudget>,
    pub store: Option<Arc<dyn SessionStore>>,
    pub cancellation: Option<CancellationToken>,
}

#[derive(Debug, Clone)]
enum ParsedTask {
    List {
        path: String,
    },
    Read {
        path: String,
    },
    Search {
        path: String,
        query: String,
    },
    Write {
        path: String,
        content: String,
    },
    Run {
        command: String,
    },
    Diff,
    Patch {
        patch: String,
    },
    Mcp {
        public_name: String,
        arguments: Value,
    },
    Invalid {
        reason: String,
    },
}

impl ParsedTask {
    fn parse(input: &str) -> Option<Self> {
        let trimmed = input.trim();
        if trimmed.eq_ignore_ascii_case("/diff") || trimmed.eq_ignore_ascii_case("diff") {
            return Some(Self::Diff);
        }
        if let Some(path) =
            strip_command(trimmed, "/list").or_else(|| strip_command(trimmed, "list"))
        {
            return Some(Self::List {
                path: default_path(path),
            });
        }
        if let Some(path) =
            strip_command(trimmed, "/read").or_else(|| strip_command(trimmed, "read"))
        {
            return Some(Self::Read {
                path: path.trim().to_string(),
            });
        }
        if let Some(rest) =
            strip_command(trimmed, "/search").or_else(|| strip_command(trimmed, "search"))
        {
            if let Some((path, query)) = parse_search_args(rest) {
                return Some(Self::Search { path, query });
            }
        }
        if let Some(command) = strip_command(trimmed, "/run")
            .or_else(|| strip_command(trimmed, "run"))
            .or_else(|| strip_command(trimmed, "shell:"))
        {
            return Some(Self::Run {
                command: command.trim().to_string(),
            });
        }
        if let Some(rest) =
            strip_command(trimmed, "/write").or_else(|| strip_command(trimmed, "write"))
        {
            let mut lines = rest.lines();
            let path = lines.next()?.trim().to_string();
            let content = lines.collect::<Vec<_>>().join("\n");
            if !path.is_empty() {
                return Some(Self::Write { path, content });
            }
        }
        if let Some(patch) =
            strip_command(trimmed, "/patch").or_else(|| strip_command(trimmed, "patch"))
        {
            if !patch.trim().is_empty() {
                return Some(Self::Patch {
                    patch: patch.to_string(),
                });
            }
        }
        if let Some(rest) = strip_command(trimmed, "/mcp").or_else(|| strip_command(trimmed, "mcp"))
        {
            return Some(match parse_mcp_args(rest) {
                Ok((public_name, arguments)) => Self::Mcp {
                    public_name,
                    arguments,
                },
                Err(reason) => Self::Invalid { reason },
            });
        }
        None
    }
}

fn input_placeholder_path() -> PathBuf {
    PathBuf::from(".")
}

fn strip_command<'a>(input: &'a str, command: &str) -> Option<&'a str> {
    if command.ends_with(':') {
        return input
            .strip_prefix(command)
            .map(str::trim)
            .filter(|value| !value.is_empty());
    }

    if input == command {
        return Some("");
    }

    input.strip_prefix(command).and_then(|rest| {
        if rest.chars().next().is_some_and(char::is_whitespace) {
            Some(rest.trim())
        } else {
            None
        }
    })
}

fn default_path(path: &str) -> String {
    if path.trim().is_empty() {
        ".".to_string()
    } else {
        path.trim().to_string()
    }
}

fn parse_search_args(input: &str) -> Option<(String, String)> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }

    if let Some((path, query)) = input.split_once(" -- ") {
        let query = query.trim();
        if query.is_empty() {
            return None;
        }
        return Some((default_path(path), query.to_string()));
    }

    Some((".".to_string(), input.to_string()))
}

fn parse_mcp_args(input: &str) -> Result<(String, Value), String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("Usage: /mcp server__tool {\"arg\":\"value\"}".to_string());
    }

    let split_at = input.find(char::is_whitespace).unwrap_or(input.len());
    let (public_name, rest) = input.split_at(split_at);
    let public_name = public_name.trim();
    if public_name.is_empty() {
        return Err("MCP tool name cannot be empty.".to_string());
    }

    let arguments = rest.trim();
    if arguments.is_empty() {
        return Ok((public_name.to_string(), json!({})));
    }

    let value = serde_json::from_str::<Value>(arguments)
        .map_err(|err| format!("Invalid /mcp JSON arguments: {err}"))?;
    if !value.is_object() {
        return Err("/mcp arguments must be a JSON object.".to_string());
    }

    Ok((public_name.to_string(), value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MessagePart;
    use crate::settings::ProviderHealthCheck;
    use std::collections::VecDeque;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
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

    struct CountingProvider {
        calls: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl ModelProvider for CountingProvider {
        async fn complete(&self, _request: ModelRequest) -> anyhow::Result<ModelResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(ModelResponse::text("provider should not be needed"))
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
    async fn approved_direct_command_resumes_exact_suspended_action() {
        let workspace = test_workspace("approved-direct-continuation");
        let agent = AgentCore::default();
        let result = agent
            .run_turn_detailed_streaming(
                AgentTurnInput {
                    thread_id: Uuid::new_v4(),
                    user_message_id: Uuid::new_v4(),
                    workspace_root: workspace.clone(),
                    content: "/write approved.txt\napproved once".to_string(),
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
        assert!(!workspace.join("approved.txt").exists());
        let continuation = match result.outcome {
            AgentTurnOutcome::Suspended { continuation, .. } => continuation,
            AgentTurnOutcome::Completed => panic!("turn should wait for approval"),
        };

        let resumed = agent
            .resume_turn_streaming(continuation, true, None, None, None)
            .await
            .expect("approved turn resumes");
        assert!(matches!(resumed.outcome, AgentTurnOutcome::Completed));
        assert_eq!(
            fs::read_to_string(workspace.join("approved.txt")).unwrap(),
            "approved once"
        );
        let _ = fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn approved_protected_metadata_write_uses_one_shot_sandbox_escalation() {
        let workspace = test_workspace("approved-sandbox-escalation");
        let agent = AgentCore::default();
        let result = agent
            .run_turn_detailed_streaming(
                AgentTurnInput {
                    thread_id: Uuid::new_v4(),
                    user_message_id: Uuid::new_v4(),
                    workspace_root: workspace.clone(),
                    content: "/write .codex/config.toml\napproved metadata".to_string(),
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
            "/run $ErrorActionPreference='Stop'; Set-Content -LiteralPath '{escaped_outside}' -Value approved-shell"
        );
        let agent = AgentCore::default();

        let result = agent
            .run_turn_detailed_streaming(
                AgentTurnInput {
                    thread_id: Uuid::new_v4(),
                    user_message_id: Uuid::new_v4(),
                    workspace_root: workspace.clone(),
                    content: command,
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
    async fn denied_direct_command_completes_without_execution() {
        let workspace = test_workspace("denied-direct-continuation");
        let agent = AgentCore::default();
        let result = agent
            .run_turn_detailed_streaming(
                AgentTurnInput {
                    thread_id: Uuid::new_v4(),
                    user_message_id: Uuid::new_v4(),
                    workspace_root: workspace.clone(),
                    content: "/write denied.txt\nnever written".to_string(),
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
        assert!(!workspace.join("denied.txt").exists());
        let _ = fs::remove_dir_all(workspace);
    }

    #[tokio::test]
    async fn denied_provider_tool_call_is_returned_to_model_as_error() {
        let workspace = test_workspace("denied-provider-continuation");
        let provider = Arc::new(ScriptedProvider::new(vec![
            ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: "call_write".to_string(),
                    name: "write_file".to_string(),
                    arguments: json!({
                        "path": "denied-provider.txt",
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
                    content: "Create denied-provider.txt".to_string(),
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
            AgentTurnOutcome::Completed => panic!("provider write should require approval"),
        };

        let resumed = agent
            .resume_turn_streaming(continuation, false, None, None, None)
            .await
            .expect("provider receives denial result");
        assert!(matches!(resumed.outcome, AgentTurnOutcome::Completed));
        assert!(assistant_text(&resumed.events).contains("approval was denied"));
        assert!(!workspace.join("denied-provider.txt").exists());
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
            "/run powershell -NoProfile -Command \"Start-Sleep -Seconds 30\""
        } else {
            "/run sh -c 'sleep 30'"
        };
        let agent = AgentCore::default();
        let workspace_for_turn = workspace.clone();
        let cancellation_for_turn = cancellation.clone();
        let task = tokio::spawn(async move {
            agent
                .run_turn(AgentTurnInput {
                    thread_id: Uuid::new_v4(),
                    user_message_id: Uuid::new_v4(),
                    workspace_root: workspace_for_turn,
                    content: command.to_string(),
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
    async fn slash_command_does_not_call_provider() {
        let workspace = test_workspace("slash-command");
        fs::write(workspace.join("sample.txt"), "slash command content").unwrap();
        let provider = Arc::new(CountingProvider {
            calls: AtomicUsize::new(0),
        });
        let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());

        let events = agent
            .run_turn(AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "/read sample.txt".to_string(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::FullAccess,
                context_budget: None,
                store: None,
                cancellation: None,
            })
            .await
            .expect("turn succeeds");

        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
        assert!(assistant_text(&events).contains("slash command content"));

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
