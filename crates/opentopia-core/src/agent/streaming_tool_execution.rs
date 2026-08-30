use super::{
    approval_required, browser_handoff_required, current_work_form_for_tool, AgentCore,
    CancellationToken, CompiledModelContext, ContextBudget, ExecutionAuthority, ModelContentPart,
    ModelConversationMessage, ModelConversationRole, PathBuf, PermissionMode,
    ProviderToolCall, ProviderToolCandidate, ProviderToolResult, SessionStore, ToolCall, ToolClass,
    ToolStateStore, TurnEvents, TurnRuntimeState, UserInputRequest, Uuid,
};
use crate::tools::ToolInvocationContext;
use crate::policy::PolicyDecision;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

/// Starts committed provider tool items while the model response continues.
/// Calls that need interactive authorization stay on the existing pending-call
/// path; everything admitted here begins immediately in a spawned task.
pub(super) struct StreamingToolExecution {
    agent: AgentCore,
    user_message_id: Uuid,
    permission_mode: PermissionMode,
    base_context: ToolInvocationContext,
    gate: Arc<RwLock<()>>,
    calls_by_id: HashMap<String, ProviderToolCall>,
    committed_calls: Vec<ProviderToolCall>,
    tasks: Vec<JoinHandle<StreamingToolTaskResult>>,
}

struct StreamingToolTaskResult {
    result: anyhow::Result<ProviderToolResult>,
    events: TurnEvents,
}

pub(super) struct StreamingToolExecutionResult {
    pub committed_calls: Vec<ProviderToolCall>,
    pub completed: Vec<(ProviderToolResult, TurnEvents)>,
}

impl StreamingToolExecutionResult {
    pub fn validate_terminal_calls(&self, terminal_calls: &[ProviderToolCall]) -> anyhow::Result<()> {
        for committed in &self.committed_calls {
            let Some(terminal) = terminal_calls.iter().find(|call| call.id == committed.id) else {
                anyhow::bail!(
                    "provider committed tool item `{}` before omitting it from the terminal response",
                    committed.id
                );
            };
            anyhow::ensure!(
                terminal == committed,
                "provider changed committed tool item `{}` before the terminal response",
                committed.id
            );
        }
        Ok(())
    }
}

impl StreamingToolExecution {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        agent: &AgentCore,
        thread_id: Uuid,
        user_message_id: Uuid,
        workspace_root: PathBuf,
        permission_mode: PermissionMode,
        runtime_state: &TurnRuntimeState,
        store: Option<Arc<dyn SessionStore>>,
        cancellation: Option<CancellationToken>,
        conversation: &[ModelConversationMessage],
        model_user_message: &str,
        model_user_content: &[ModelContentPart],
        model_context: &CompiledModelContext,
        events: &TurnEvents,
    ) -> anyhow::Result<Self> {
        let sandbox_config =
            runtime_state.sandbox_config_with_path_leases(&agent.tool_host.sandbox_config);
        let authority = ExecutionAuthority::new(
            workspace_root.clone(),
            permission_mode,
            sandbox_config.clone(),
            agent.capability_projection.clone(),
        )?;
        let mut base_context = authority.local_tool_context();
        base_context.state = store.map(ToolStateStore::new);
        base_context.thread_id = Some(thread_id);
        base_context.cancel = cancellation;
        agent.apply_agent_context(&mut base_context, user_message_id);
        base_context.fork_conversation = conversation.to_vec();
        base_context.fork_conversation.push(ModelConversationMessage {
            role: ModelConversationRole::User,
            content: model_user_message.to_string(),
            content_parts: model_user_content.to_vec(),
            tool_calls: Vec::new(),
            tool_results: Vec::new(),
        });
        base_context.fork_model_context = Some(model_context.clone());
        base_context.current_work_form = current_work_form_for_tool(&base_context, events)?;

        Ok(Self {
            agent: agent.clone(),
            user_message_id,
            permission_mode,
            base_context,
            gate: Arc::new(RwLock::new(())),
            calls_by_id: HashMap::new(),
            committed_calls: Vec::new(),
            tasks: Vec::new(),
        })
    }

    pub(super) fn dispatch(&mut self, call: ProviderToolCall) -> anyhow::Result<()> {
        if let Some(previous) = self.calls_by_id.get(&call.id) {
            anyhow::ensure!(
                previous == &call,
                "provider reused tool-call id `{}` for a different committed item",
                call.id
            );
            return Ok(());
        }
        self.calls_by_id.insert(call.id.clone(), call.clone());
        self.committed_calls.push(call.clone());

        let catalog = self.agent.tool_runtime_catalog();
        let logical_call = ToolCall::new(&call.name, call.arguments.clone());
        let tool = catalog.get(&call.name);
        let interactive = catalog.registry().class(&call.name) == Some(ToolClass::StructuredInput);
        let policy_and_authorization = tool.as_ref().map(|tool| {
            let policy = tool.execution_policy(&logical_call);
            let authorization = tool
                .authorization_preflight(&logical_call, &self.base_context)
                .map(|decision| self.permission_mode.resolve_policy_decision(decision));
            (policy, authorization)
        });

        // Unknown/invalid tools still need the ordinary path to produce the
        // canonical provider error. Tools without a pure authorization preview
        // must also stay there: executing them just to discover an approval
        // boundary would violate the existing continuation contract.
        let admitted = call.name == super::TOOL_SEARCH_NAME
            || (!interactive
                && catalog.allows(&call.name)
                && catalog.input_error(&call).is_none()
                && policy_and_authorization
                    .as_ref()
                    .is_some_and(|(_, decision)| matches!(decision, Some(PolicyDecision::Allow))));
        if !admitted {
            return Ok(());
        }

        let parallel = policy_and_authorization
            .as_ref()
            .is_some_and(|(policy, _)| policy.parallel_safe && policy.read_only);
        let agent = self.agent.clone();
        let context = self.base_context.clone();
        let gate = Arc::clone(&self.gate);
        let user_message_id = self.user_message_id;
        self.tasks.push(tokio::spawn(async move {
            let mut events = TurnEvents::new(None);
            let result = if parallel {
                let _guard = gate.read_owned().await;
                agent
                    .execute_provider_tool_call(&call, user_message_id, context, &mut events)
                    .await
            } else {
                let _guard = gate.write_owned().await;
                agent
                    .execute_provider_tool_call(&call, user_message_id, context, &mut events)
                    .await
            };
            StreamingToolTaskResult {
                result,
                events,
            }
        }));
        Ok(())
    }

    pub(super) fn committed_any(&self) -> bool {
        !self.committed_calls.is_empty()
    }

    pub(super) async fn finish(self) -> anyhow::Result<StreamingToolExecutionResult> {
        let mut completed = Vec::with_capacity(self.tasks.len());
        for task in self.tasks {
            let task = task
                .await
                .map_err(|error| anyhow::anyhow!("streaming tool task failed to join: {error}"))?;
            match task.result {
                Ok(result) => completed.push((result, task.events)),
                Err(error)
                    if approval_required(&error).is_some()
                        || browser_handoff_required(&error).is_some() =>
                {}
                Err(error) => return Err(error),
            }
        }
        Ok(StreamingToolExecutionResult {
            committed_calls: self.committed_calls,
            completed,
        })
    }
}

impl AgentCore {
    pub(super) fn commit_streaming_tool_execution(
        &self,
        execution: StreamingToolExecutionResult,
        budget: &mut Option<ContextBudget>,
        tool_candidates: &mut Vec<ProviderToolCandidate>,
        provider_tool_results: &mut Vec<ProviderToolResult>,
        events: &mut TurnEvents,
    ) -> anyhow::Result<HashSet<String>> {
        let mut completed_call_ids = HashSet::new();
        for (result, local_events) in execution.completed {
            for event in local_events.into_vec() {
                events.push(event);
            }
            let user_input_request = result
                .metadata
                .get("userInputRequest")
                .cloned()
                .map(serde_json::from_value::<UserInputRequest>)
                .transpose()?;
            anyhow::ensure!(
                user_input_request.is_none(),
                "streaming tool `{}` unexpectedly requested user input",
                result.name
            );
            if let Some(budget) = budget.as_mut() {
                budget.record_tokens(ContextBudget::estimate_tokens(&result.output));
            }
            self.reveal_tools_from_search_result(&result, tool_candidates);
            completed_call_ids.insert(result.call_id.clone());
            provider_tool_results.push(result);
        }
        Ok(completed_call_ids)
    }
}
