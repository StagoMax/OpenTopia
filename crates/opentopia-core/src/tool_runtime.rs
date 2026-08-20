//! Provider-neutral tool validation, authorization preflight, scheduling, and execution.
//!
//! The runtime consumes an immutable catalog snapshot. Product code may replace
//! that snapshot between rounds, but a batch is planned and executed against one
//! coherent capability boundary.

use crate::background::{
    BackgroundCompletionSink, BackgroundOutputChunk, BackgroundProcessRegistry, BackgroundScope,
};
use crate::browser::{BrowserRuntime, BrowserRuntimeConfig, LocalBrowserRuntime};
use crate::computer::{
    ComputerAccessPolicy, ComputerRuntime, ComputerRuntimeConfig, LocalComputerRuntime,
};
use crate::effect_journal::{EffectIntent, EffectKind, EffectSideEffectClass, EffectStatus};
use crate::enterprise::CapabilityProjection;
use crate::execution_authority::ExecutionAuthority;
use crate::guardian::{
    GuardianApprovalAction, GuardianApprovalRequest, GuardianReviewContext, GuardianReviewResult,
    GuardianReviewSessionManager,
};
use crate::mcp::McpToolDescriptor;
use crate::mcp_host::McpExtensionHost;
use crate::model::{
    AgentEvent, AgentEventPayload, Message, MessagePart, MessageRole, ModelContentPart, ToolCall,
    ToolResult,
};
use crate::model_context::content_fingerprint;
#[cfg(test)]
use crate::policy::BasicPolicyEngine;
use crate::policy::{approval_required, ApprovalRequired, PermissionMode, PolicyDecision};
use crate::provider::{
    invalid_tool_arguments_json_details, ModelConversationMessage, ProviderToolCall,
    ProviderToolCandidate, ProviderToolResult,
};
use crate::sandbox::LocalSandboxConfig;
#[cfg(test)]
use crate::store::SessionStore;
use crate::tool_error::{insert_classified_anyhow_error_record, insert_tool_error_record};
use crate::tool_result_ingress::{
    normalize_tool_result_at_ingress, provider_tool_result_content, provider_tool_result_metadata,
    provider_tool_result_output, tool_result_is_error,
};
use crate::tool_state::ToolStateStore;
use crate::tools::{
    Tool, ToolClass, ToolInvocationContext, ToolRegistry, ToolSideEffect, ToolSource,
};
use crate::turn_inbox::{TurnInbox, TurnInboxItem};
use crate::work_form::{WorkForm, WorkItemStatus};
use crate::ConnectionOperationRuntimeRoute;
use async_trait::async_trait;
use chrono::Utc;
use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub const MAX_PARALLEL_TOOL_CALLS: usize = 8;
const TOOL_EXECUTION_DURATION_MS_KEY: &str = "durationMs";

/// Owns every concrete tool-side service behind one AgentCore boundary.
///
/// AgentCore coordinates turns; it no longer owns individual catalogs,
/// extension hosts, device runtimes, detached jobs, or schedulers as parallel
/// fields. ToolRuntimeHost is the single composition root for that dependency
/// cone and can later be replaced without changing the model loop.
#[derive(Clone)]
pub struct ToolRuntimeHost {
    pub(crate) catalog: ToolRegistry,
    pub(crate) mcp_host: Option<McpExtensionHost>,
    pub(crate) active_mcp_tools: Vec<McpToolDescriptor>,
    pub(crate) active_connection_operations: BTreeMap<String, ConnectionOperationRuntimeRoute>,
    pub(crate) model_supports_vision: bool,
    pub(crate) sandbox_config: LocalSandboxConfig,
    pub(crate) browser: Arc<dyn BrowserRuntime>,
    pub(crate) computer: Arc<dyn ComputerRuntime>,
    pub(crate) computer_access_policy: ComputerAccessPolicy,
    pub(crate) background: BackgroundProcessRegistry,
}

impl ToolRuntimeHost {
    pub fn new(
        catalog: ToolRegistry,
        model_supports_vision: bool,
        sandbox_config: LocalSandboxConfig,
    ) -> Self {
        Self {
            catalog,
            mcp_host: None,
            active_mcp_tools: Vec::new(),
            active_connection_operations: BTreeMap::new(),
            model_supports_vision,
            sandbox_config,
            browser: Arc::new(LocalBrowserRuntime::new(BrowserRuntimeConfig::default())),
            computer: Arc::new(LocalComputerRuntime::new(ComputerRuntimeConfig::default())),
            computer_access_policy: ComputerAccessPolicy::default(),
            background: BackgroundProcessRegistry::default(),
        }
    }
}

/// The single synchronous response to a provider call that started detached work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptedToolResult {
    pub provider_call_id: String,
    pub tool_name: String,
    pub job_id: Uuid,
}

impl AcceptedToolResult {
    pub fn from_provider_result(result: &ProviderToolResult) -> Option<Self> {
        let background = result
            .metadata
            .get("background")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !background || result.is_error {
            return None;
        }
        let job_id = result
            .metadata
            .get("jobId")
            .and_then(Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())?;
        Some(Self {
            provider_call_id: result.call_id.clone(),
            tool_name: result.name.clone(),
            job_id,
        })
    }
}

/// A terminal background result, correlated by `job_id` rather than by reusing
/// the original provider Tool Call id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AsyncToolResult {
    pub job_id: Uuid,
    pub tool_name: String,
    pub output: String,
    pub is_error: bool,
    #[serde(default)]
    pub metadata: Value,
}

impl AsyncToolResult {
    pub fn from_background_chunk(chunk: &BackgroundOutputChunk) -> Self {
        Self::from_background_chunk_for_tool(chunk, "background_job")
    }

    pub fn from_background_chunk_for_tool(
        chunk: &BackgroundOutputChunk,
        tool_name: impl Into<String>,
    ) -> Self {
        let job = &chunk.job;
        let mut lines = vec![format!(
            "Background job {} finished with status {} (exit {}).",
            job.job_id,
            job.status.as_str(),
            job.exit_code
                .map_or_else(|| "unknown".to_string(), |code| code.to_string())
        )];
        if let Some(error) = job.error.as_deref() {
            lines.push(format!("Error: {error}"));
        }
        if chunk.dropped_bytes > 0 {
            lines.push(format!(
                "{} earlier output bytes were dropped; the retained tail follows.",
                chunk.dropped_bytes
            ));
        }
        if !chunk.stdout.trim().is_empty() {
            lines.push(format!("stdout:\n{}", chunk.stdout.trim()));
        }
        if !chunk.stderr.trim().is_empty() {
            lines.push(format!("stderr:\n{}", chunk.stderr.trim()));
        }
        Self {
            job_id: job.job_id,
            tool_name: tool_name.into(),
            output: lines.join("\n"),
            is_error: !job.success,
            metadata: json!({
                "asyncToolResult": true,
                "jobId": job.job_id,
                "command": job.command,
                "status": job.status,
                "exitCode": job.exit_code,
                "success": job.success,
                "droppedBytes": chunk.dropped_bytes,
                "untrusted": true,
            }),
        }
    }

    pub fn provider_call_id(&self) -> String {
        format!("async_tool_result_{}", self.job_id.simple())
    }

    pub fn into_provider_result(self, runtime_tool_name: &str) -> ProviderToolResult {
        ProviderToolResult {
            call_id: self.provider_call_id(),
            name: runtime_tool_name.to_string(),
            output: self.output.clone(),
            content: vec![ModelContentPart::text(self.output)],
            is_error: self.is_error,
            metadata: self.metadata,
        }
    }
}

/// Bridges a detached job completion into both the durable conversation ledger
/// and the live turn inbox. The background registry depends only on its sink
/// port, while this adapter owns the persistence/protocol translation.
#[derive(Clone)]
pub struct DurableAsyncToolResultSink {
    thread_id: Uuid,
    persisted_turn_id: Uuid,
    inbox_turn_id: Uuid,
    agent_path: String,
    source_tool_name: String,
    store: ToolStateStore,
    inbox: Arc<dyn TurnInbox>,
}

impl DurableAsyncToolResultSink {
    pub fn new(
        thread_id: Uuid,
        persisted_turn_id: Uuid,
        inbox_turn_id: Uuid,
        agent_path: impl Into<String>,
        source_tool_name: impl Into<String>,
        store: ToolStateStore,
        inbox: Arc<dyn TurnInbox>,
    ) -> Self {
        Self {
            thread_id,
            persisted_turn_id,
            inbox_turn_id,
            agent_path: agent_path.into(),
            source_tool_name: source_tool_name.into(),
            store,
            inbox,
        }
    }

    fn message_id(job_id: Uuid, discriminator: u128) -> Uuid {
        Uuid::from_u128(job_id.as_u128() ^ discriminator)
    }
}

impl BackgroundCompletionSink for DurableAsyncToolResultSink {
    fn deliver(&self, chunk: BackgroundOutputChunk) -> anyhow::Result<()> {
        const CALL_MESSAGE_DISCRIMINATOR: u128 = 0x4153594e_435f4341_4c4c5f4d_53470001;
        const RESULT_MESSAGE_DISCRIMINATOR: u128 = 0x4153594e_435f5245_53555f4d_53470002;
        const RUNTIME_TOOL_NAME: &str = "runtime_background_completion";

        let mut async_result =
            AsyncToolResult::from_background_chunk_for_tool(&chunk, self.source_tool_name.clone());
        let provider_call_id = async_result.provider_call_id();
        // A persisted Responses cursor represents history only through its last
        // model call. The newly appended async result is a suffix mutation, so
        // the next turn must replay local history unless a later model request
        // incorporates it and saves a fresh cursor.
        let provider_state_cleared = self
            .store
            .clear_provider_conversation_state(self.thread_id, &self.agent_path)?;
        if let Some(metadata) = async_result.metadata.as_object_mut() {
            metadata.insert("durablyAppended".to_string(), json!(true));
            metadata.insert("sourceToolName".to_string(), json!(&self.source_tool_name));
            metadata.insert("agentPath".to_string(), json!(&self.agent_path));
            metadata.insert("runtimeObservation".to_string(), json!("async_tool_result"));
            metadata.insert(
                "providerStateCleared".to_string(),
                json!(provider_state_cleared),
            );
        }

        // The UUID correlation is stable in the local ledger; provider replay
        // uses the deterministic string id stored in result metadata.
        let call = ToolCall {
            id: async_result.job_id,
            name: RUNTIME_TOOL_NAME.to_string(),
            input: json!({
                "source": "runtime",
                "agentPath": &self.agent_path,
                "sourceToolName": &self.source_tool_name,
                "jobId": async_result.job_id,
            }),
        };
        let mut provider_result = async_result.clone().into_provider_result(RUNTIME_TOOL_NAME);
        if let Some(metadata) = provider_result.metadata.as_object_mut() {
            metadata.insert("providerToolCallId".to_string(), json!(provider_call_id));
            metadata.insert("toolName".to_string(), json!(RUNTIME_TOOL_NAME));
            metadata.insert("isError".to_string(), json!(provider_result.is_error));
            metadata.insert("success".to_string(), json!(!provider_result.is_error));
        }
        let tool_result = ToolResult {
            call_id: call.id,
            output: provider_result.output,
            content: provider_result.content,
            metadata: provider_result.metadata,
        };
        let created_at = Utc::now();

        self.store.append_message(Message {
            id: Self::message_id(async_result.job_id, CALL_MESSAGE_DISCRIMINATOR),
            thread_id: self.thread_id,
            role: MessageRole::Tool,
            parts: vec![MessagePart::ToolCall { call: call.clone() }],
            created_at,
        })?;
        self.store.append_message(Message {
            id: Self::message_id(async_result.job_id, RESULT_MESSAGE_DISCRIMINATOR),
            thread_id: self.thread_id,
            role: MessageRole::Tool,
            parts: vec![MessagePart::ToolResult {
                result: tool_result.clone(),
            }],
            created_at,
        })?;
        self.store.append_event(AgentEvent::new(
            self.thread_id,
            Some(self.persisted_turn_id),
            0,
            AgentEventPayload::ToolCallStarted { call },
        ))?;
        self.store.append_event(AgentEvent::new(
            self.thread_id,
            Some(self.persisted_turn_id),
            0,
            AgentEventPayload::ToolCallFinished {
                result: tool_result,
            },
        ))?;

        if self
            .store
            .get_active_turn(self.thread_id)?
            .is_some_and(|turn| turn.turn_id == self.persisted_turn_id)
        {
            self.inbox.push(
                self.inbox_turn_id,
                TurnInboxItem::AsyncToolResult {
                    result: async_result,
                },
            );
        }
        Ok(())
    }
}

/// Immutable capability and schema view used for one scheduling/execution pass.
#[derive(Clone)]
pub struct ToolRuntimeCatalog {
    registry: ToolRegistry,
    provider_candidates: Vec<ProviderToolCandidate>,
    capability_projection: CapabilityProjection,
    allowed_tools: Option<HashSet<String>>,
    denied_tools: HashSet<String>,
    enabled_bundled_plugins: HashSet<String>,
}

impl ToolRuntimeCatalog {
    pub fn new(
        registry: ToolRegistry,
        provider_candidates: Vec<ProviderToolCandidate>,
        capability_projection: CapabilityProjection,
        allowed_tools: Option<HashSet<String>>,
        denied_tools: HashSet<String>,
        enabled_bundled_plugins: HashSet<String>,
    ) -> Self {
        Self {
            registry,
            provider_candidates,
            capability_projection,
            allowed_tools,
            denied_tools,
            enabled_bundled_plugins,
        }
    }

    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.registry.get(name)
    }

    pub fn allows(&self, name: &str) -> bool {
        let plugin_enabled = match self.registry.source(name) {
            Some(ToolSource::BundledPlugin { plugin_name }) => {
                self.enabled_bundled_plugins.contains(&plugin_name)
                    && self.capability_projection.allows_plugin(&plugin_name)
            }
            _ => true,
        };
        plugin_enabled
            && self.capability_projection.allows_tool(name)
            && !self.denied_tools.contains(name)
            && self
                .allowed_tools
                .as_ref()
                .map(|allowed| allowed.contains(name))
                .unwrap_or(true)
    }

    pub fn disabled_message(&self, name: &str) -> String {
        match self.registry.source(name) {
            Some(ToolSource::BundledPlugin { plugin_name })
                if !self.enabled_bundled_plugins.contains(&plugin_name) =>
            {
                format!(
                    "{name} is disabled because bundled plugin {plugin_name} is disabled for this thread"
                )
            }
            _ => format!("{name} is disabled by the active agent profile"),
        }
    }

    pub fn input_error(&self, provider_call: &ProviderToolCall) -> Option<String> {
        if let Some(tool) = self.registry.get(&provider_call.name) {
            return tool.input_error(&provider_call.arguments);
        }
        let schema = self
            .provider_candidates
            .iter()
            .find(|candidate| candidate.name == provider_call.name)?
            .input_schema
            .clone();
        crate::provider::tool_input_schema_error(&schema, &provider_call.arguments, "arguments")
    }

    pub fn insert_source_metadata(&self, name: &str, metadata: &mut Value) {
        let Some(object) = metadata.as_object_mut() else {
            return;
        };
        match self.registry.source(name) {
            Some(ToolSource::Core) => {
                object.insert("toolSource".to_string(), json!("core"));
            }
            Some(ToolSource::BundledPlugin { plugin_name }) => {
                object.insert("toolSource".to_string(), json!("bundled_plugin"));
                object.insert("pluginName".to_string(), json!(plugin_name));
            }
            Some(ToolSource::Mcp) => {
                object.insert("toolSource".to_string(), json!("mcp"));
            }
            None => {}
        }
    }
}

pub struct ToolSchedulingInput<'a> {
    pub catalog: &'a ToolRuntimeCatalog,
    pub calls: &'a [ProviderToolCall],
    pub workspace_root: &'a Path,
    pub permission_mode: PermissionMode,
    pub sandbox_config: &'a LocalSandboxConfig,
}

#[derive(Debug, Clone)]
pub struct ToolApprovalCandidate {
    pub call: ProviderToolCall,
    pub reason: String,
    pub action: GuardianApprovalAction,
}

pub struct ToolReviewInput<'a> {
    pub guardian: &'a GuardianReviewSessionManager,
    pub request: &'a GuardianApprovalRequest,
    pub conversation: &'a [ModelConversationMessage],
    pub current_user_message: &'a str,
    pub tool_calls: &'a [ProviderToolCall],
    pub tool_results: &'a [ProviderToolResult],
    pub workspace_root: &'a Path,
    pub sandbox_config: &'a LocalSandboxConfig,
}

/// One canonical tool invocation after the turn runtime has supplied its
/// immutable capability snapshot and invocation-scoped dependencies.
pub struct ToolExecutionInput {
    pub catalog: ToolRuntimeCatalog,
    pub call: ToolCall,
    pub context: ToolInvocationContext,
    pub metadata_overlay: Option<Value>,
}

/// A policy boundary is control flow, not a failed tool execution. The runtime
/// preserves it explicitly so callers can review or suspend the exact call
/// without publishing a synthetic failed result first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolApprovalBoundary {
    pub logical_call_id: Uuid,
    pub reason: String,
}

impl ToolApprovalBoundary {
    fn from_error(logical_call_id: Uuid, error: &anyhow::Error) -> Option<Self> {
        approval_required(error).map(|required| Self {
            logical_call_id,
            reason: required.reason().to_string(),
        })
    }
}

#[derive(Debug)]
pub enum ToolExecutionOutcome<T> {
    Completed(T),
    NeedsApproval(ToolApprovalBoundary),
    Failed(anyhow::Error),
}

impl<T> ToolExecutionOutcome<T> {
    pub fn into_result(self) -> anyhow::Result<T> {
        match self {
            Self::Completed(value) => Ok(value),
            Self::NeedsApproval(boundary) => Err(ApprovalRequired::new(boundary.reason).into()),
            Self::Failed(error) => Err(error),
        }
    }

    #[cfg(test)]
    fn as_completed(&self) -> Option<&T> {
        match self {
            Self::Completed(value) => Some(value),
            Self::NeedsApproval(_) | Self::Failed(_) => None,
        }
    }
}

/// Execution always returns its local protocol events together with the
/// outcome. Callers may delay publication to preserve provider call order even
/// when the runtime executes independent calls concurrently.
pub struct ToolExecutionReport {
    pub outcome: ToolExecutionOutcome<ToolResult>,
    pub events: Vec<AgentEventPayload>,
}

/// Provider-facing invocation envelope. It deliberately contains no model-loop
/// state: the Tool Runtime owns validation, durable effect reconciliation,
/// execution, normalization, and detached-result delivery for this call.
pub struct ProviderToolExecutionInput {
    pub catalog: ToolRuntimeCatalog,
    pub provider_call: ProviderToolCall,
    pub user_message_id: Uuid,
    pub agent_path: String,
    pub context: ToolInvocationContext,
    pub background: BackgroundProcessRegistry,
    pub turn_inbox: Arc<dyn TurnInbox>,
}

pub struct ProviderToolExecutionReport {
    pub provider_call: ProviderToolCall,
    pub outcome: ToolExecutionOutcome<ProviderToolResult>,
    pub events: Vec<AgentEventPayload>,
}

#[async_trait]
pub trait ToolRuntime: Send + Sync {
    /// Reject malformed provider calls before authorization or execution.
    fn validate_provider_call(
        &self,
        catalog: &ToolRuntimeCatalog,
        call: &ProviderToolCall,
    ) -> Option<ProviderToolResult>;

    /// Select independent calls that are already authorized to execute.
    fn parallel_call_indices(&self, input: ToolSchedulingInput<'_>) -> Vec<usize>;

    /// Select a contiguous approved batch; authorization has already been granted.
    fn approved_parallel_call_indices(
        &self,
        catalog: &ToolRuntimeCatalog,
        calls: &[ProviderToolCall],
    ) -> Vec<usize>;

    /// Detect the contiguous calls that reach a declared approval boundary.
    /// The caller decides whether the configured reviewer is automatic or the user.
    fn approval_candidates(&self, input: ToolSchedulingInput<'_>) -> Vec<ToolApprovalCandidate>;

    /// Convert the canonical executor result into the provider protocol exactly once.
    fn normalize_provider_result(
        &self,
        catalog: &ToolRuntimeCatalog,
        call: &ProviderToolCall,
        result: anyhow::Result<ToolResult>,
    ) -> anyhow::Result<ProviderToolResult>;

    /// Review an authorization boundary inside the tool-runtime dependency cone.
    async fn review(
        &self,
        input: ToolReviewInput<'_>,
        cancellation: Option<&CancellationToken>,
    ) -> GuardianReviewResult;

    /// Execute one canonical invocation and emit a complete local event pair.
    async fn execute_call(&self, input: ToolExecutionInput) -> ToolExecutionReport;

    /// Execute and normalize one provider call. The report owns all synchronous
    /// protocol output; a detached terminal result is delivered later through
    /// the Turn Inbox and durable ledger, correlated by JobId.
    async fn execute_provider_call(
        &self,
        input: ProviderToolExecutionInput,
    ) -> ProviderToolExecutionReport;

    /// Execute an already scheduled independent batch concurrently while
    /// returning reports in the exact order supplied by the provider.
    async fn execute_provider_batch(
        &self,
        inputs: Vec<ProviderToolExecutionInput>,
    ) -> Vec<ProviderToolExecutionReport>;
}

/// Local implementation of the provider-neutral tool runtime port.
#[derive(Clone, Default)]
pub struct LocalToolRuntime;

/// Constructor-injected executor for the local Tool trait. The wide
/// ToolInvocationContext is captured once at the runtime edge; individual tools
/// cannot reach AgentCore or any mutable turn-loop state during execution.
struct ToolExecutor {
    tool: Arc<dyn Tool>,
    context: ToolInvocationContext,
}

impl ToolExecutor {
    fn new(tool: Arc<dyn Tool>, context: ToolInvocationContext) -> Self {
        Self { tool, context }
    }

    async fn execute(self, call: ToolCall) -> anyhow::Result<ToolResult> {
        self.tool.execute(call, self.context).await
    }
}

impl LocalToolRuntime {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ToolRuntime for LocalToolRuntime {
    fn validate_provider_call(
        &self,
        catalog: &ToolRuntimeCatalog,
        provider_call: &ProviderToolCall,
    ) -> Option<ProviderToolResult> {
        if let Some(details) = invalid_tool_arguments_json_details(&provider_call.arguments) {
            let reason = details
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("invalid JSON syntax");
            let line = details
                .get("errorLine")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let column = details
                .get("errorColumn")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let excerpt = details
                .get("redactedExcerpt")
                .and_then(Value::as_str)
                .unwrap_or("<unavailable>");
            let output = format!(
                "Tool `{}` was not executed because its `function.arguments` payload was not valid JSON: {reason} at line {line}, column {column}. Redacted excerpt: {excerpt}. Submit a new tool call with one valid JSON object that matches the tool schema; quote every string value (for example, use `\"fork_turns\":\"none\"`). Do not repeat the malformed call unchanged.",
                provider_call.name
            );
            let mut metadata = json!({
                "toolName": &provider_call.name,
                "providerToolCallId": &provider_call.id,
                "success": false,
                "invalidToolArguments": true,
                "invalidToolArgumentsJson": true,
                "retryable": true,
                "providerArgumentDiagnostics": details,
            });
            insert_tool_error_record(
                &mut metadata,
                "invalid_tool_arguments_json",
                "provider_protocol_validation",
                false,
                true,
                &output,
            );
            catalog.insert_source_metadata(&provider_call.name, &mut metadata);
            return Some(ProviderToolResult {
                call_id: provider_call.id.clone(),
                name: provider_call.name.clone(),
                output: output.clone(),
                content: vec![ModelContentPart::text(output)],
                is_error: true,
                metadata,
            });
        }

        let validation_error = catalog.input_error(provider_call)?;
        let output = format!(
            "Invalid arguments for tool `{}`: {validation_error}. Do not retry this call unchanged; provide the required fields or choose a different action.",
            provider_call.name
        );
        let mut metadata = json!({
            "toolName": &provider_call.name,
            "providerToolCallId": &provider_call.id,
            "success": false,
            "invalidToolArguments": true,
            "inputSchemaValidationError": validation_error,
        });
        insert_tool_error_record(
            &mut metadata,
            "invalid_tool_arguments",
            "validation",
            false,
            false,
            &output,
        );
        catalog.insert_source_metadata(&provider_call.name, &mut metadata);
        Some(ProviderToolResult {
            call_id: provider_call.id.clone(),
            name: provider_call.name.clone(),
            output: output.clone(),
            content: vec![ModelContentPart::text(output)],
            is_error: true,
            metadata,
        })
    }

    fn parallel_call_indices(&self, input: ToolSchedulingInput<'_>) -> Vec<usize> {
        let ToolSchedulingInput {
            catalog,
            calls,
            workspace_root,
            permission_mode,
            sandbox_config,
        } = input;
        let Ok(authority) = ExecutionAuthority::new(
            workspace_root.to_path_buf(),
            permission_mode,
            sandbox_config.clone(),
            catalog.capability_projection.clone(),
        ) else {
            return Vec::new();
        };
        let authorization_context = authority.local_tool_context();
        let mut resource_keys = HashMap::<String, bool>::new();
        let mut selected = Vec::new();

        for (index, provider_call) in calls.iter().enumerate() {
            if selected.len() >= MAX_PARALLEL_TOOL_CALLS {
                break;
            }
            if !catalog.allows(&provider_call.name) || catalog.input_error(provider_call).is_some()
            {
                continue;
            }
            let call = ToolCall::new(&provider_call.name, provider_call.arguments.clone());
            let Some(tool) = catalog.get(&provider_call.name) else {
                continue;
            };
            let execution_policy = tool.execution_policy(&call);
            if !execution_policy.parallel_safe {
                break;
            }
            let decision = tool
                .authorization_preflight(&call, &authorization_context)
                .map(|decision| permission_mode.resolve_policy_decision(decision));
            if !matches!(decision, Some(PolicyDecision::Allow)) {
                continue;
            }
            if scheduling_conflicts(&mut resource_keys, &execution_policy) {
                continue;
            }
            selected.push(index);
        }
        selected
    }

    fn approved_parallel_call_indices(
        &self,
        catalog: &ToolRuntimeCatalog,
        calls: &[ProviderToolCall],
    ) -> Vec<usize> {
        let mut resource_keys = HashMap::<String, bool>::new();
        let mut selected = Vec::new();
        for (index, provider_call) in calls.iter().enumerate() {
            if selected.len() >= MAX_PARALLEL_TOOL_CALLS {
                break;
            }
            if !catalog.allows(&provider_call.name) || catalog.input_error(provider_call).is_some()
            {
                break;
            }
            let call = ToolCall::new(&provider_call.name, provider_call.arguments.clone());
            let Some(tool) = catalog.get(&provider_call.name) else {
                break;
            };
            let execution_policy = tool.execution_policy(&call);
            if !execution_policy.parallel_safe {
                break;
            }
            if scheduling_conflicts(&mut resource_keys, &execution_policy) {
                continue;
            }
            selected.push(index);
        }
        selected
    }

    fn approval_candidates(&self, input: ToolSchedulingInput<'_>) -> Vec<ToolApprovalCandidate> {
        let ToolSchedulingInput {
            catalog,
            calls,
            workspace_root,
            permission_mode,
            sandbox_config,
        } = input;
        let Ok(authority) = ExecutionAuthority::new(
            workspace_root.to_path_buf(),
            permission_mode,
            sandbox_config.clone(),
            catalog.capability_projection.clone(),
        ) else {
            return Vec::new();
        };
        let context = authority.local_tool_context();
        let mut candidates = Vec::new();
        for provider_call in calls.iter().take(MAX_PARALLEL_TOOL_CALLS) {
            if !catalog.allows(&provider_call.name) || catalog.input_error(provider_call).is_some()
            {
                break;
            }
            let Some(tool) = catalog.get(&provider_call.name) else {
                break;
            };
            let call = ToolCall::new(&provider_call.name, provider_call.arguments.clone());
            let action = GuardianApprovalAction::from_provider_call(provider_call, workspace_root);
            if action.reviewability_error().is_some() {
                break;
            }
            let decision = tool
                .authorization_preflight(&call, &context)
                .map(|decision| permission_mode.resolve_policy_decision(decision));
            match decision {
                Some(PolicyDecision::Ask { reason }) => candidates.push(ToolApprovalCandidate {
                    call: provider_call.clone(),
                    reason,
                    action,
                }),
                Some(PolicyDecision::Allow | PolicyDecision::Deny { .. }) | None => break,
            }
        }
        candidates
    }

    fn normalize_provider_result(
        &self,
        catalog: &ToolRuntimeCatalog,
        provider_call: &ProviderToolCall,
        result: anyhow::Result<ToolResult>,
    ) -> anyhow::Result<ProviderToolResult> {
        match result {
            Ok(result) => {
                let content = provider_tool_result_content(&result);
                let is_error = tool_result_is_error(&result);
                let metadata = provider_tool_result_metadata(&provider_call.name, &result.metadata);
                Ok(ProviderToolResult {
                    call_id: provider_call.id.clone(),
                    name: provider_call.name.clone(),
                    output: provider_tool_result_output(&result),
                    content,
                    is_error,
                    metadata,
                })
            }
            Err(error) if approval_required(&error).is_some() => Err(error),
            Err(error) if error.to_string().contains("cancelled") => Err(error),
            Err(error) => {
                let error_message = format!("{error:#}");
                let mut metadata = json!({
                    "toolName": &provider_call.name,
                    "providerToolCallId": &provider_call.id,
                    "success": false,
                    "error": &error_message
                });
                insert_classified_anyhow_error_record(&mut metadata, &error);
                catalog.insert_source_metadata(&provider_call.name, &mut metadata);
                Ok(ProviderToolResult {
                    call_id: provider_call.id.clone(),
                    name: provider_call.name.clone(),
                    output: error_message.clone(),
                    content: vec![ModelContentPart::text(error_message)],
                    is_error: true,
                    metadata,
                })
            }
        }
    }

    async fn review(
        &self,
        input: ToolReviewInput<'_>,
        cancellation: Option<&CancellationToken>,
    ) -> GuardianReviewResult {
        input
            .guardian
            .review(
                input.request,
                GuardianReviewContext {
                    conversation: input.conversation,
                    current_user_message: input.current_user_message,
                    tool_calls: input.tool_calls,
                    tool_results: input.tool_results,
                    workspace_root: input.workspace_root,
                    sandbox_config: input.sandbox_config,
                },
                cancellation,
            )
            .await
    }

    async fn execute_call(&self, input: ToolExecutionInput) -> ToolExecutionReport {
        let execution_started_at = Instant::now();
        let ToolExecutionInput {
            catalog,
            call,
            mut context,
            metadata_overlay,
        } = input;
        let name = call.name.clone();
        let tool_class = catalog
            .registry()
            .class(&name)
            .unwrap_or(ToolClass::Standard);
        let result_store = context.state.clone();
        let result_thread_id = context.thread_id;
        let approval_granted = context.approval_granted;
        let active_work_item_id = context.current_work_form.as_ref().and_then(|form| {
            form.items
                .iter()
                .find(|item| item.status == WorkItemStatus::InProgress)
                .map(|item| item.id.clone())
        });
        let mut events = vec![AgentEventPayload::ToolCallStarted { call: call.clone() }];

        if !catalog.allows(&name) {
            let error = anyhow::anyhow!(catalog.disabled_message(&name));
            let mut metadata = json!({
                "toolName": &name,
                "success": false,
                "error": error.to_string(),
            });
            insert_tool_error_record(
                &mut metadata,
                "tool_disabled",
                "dispatch",
                false,
                false,
                &error.to_string(),
            );
            catalog.insert_source_metadata(&name, &mut metadata);
            insert_approval_execution_metadata(&mut metadata, approval_granted, Some(&error));
            merge_metadata_overlay(&mut metadata, metadata_overlay.as_ref());
            insert_work_item_metadata(&mut metadata, active_work_item_id.as_deref());
            insert_tool_execution_duration(&mut metadata, &execution_started_at);
            events.push(AgentEventPayload::ToolCallFinished {
                result: ToolResult {
                    call_id: call.id,
                    output: error.to_string(),
                    content: vec![ModelContentPart::text(error.to_string())],
                    metadata,
                },
            });
            return ToolExecutionReport {
                outcome: ToolExecutionOutcome::Failed(error),
                events,
            };
        }

        let Some(tool) = catalog.get(&name) else {
            let error = anyhow::anyhow!("{} tool not registered", name);
            let mut metadata = json!({
                "toolName": &name,
                "success": false,
                "error": error.to_string(),
            });
            insert_tool_error_record(
                &mut metadata,
                "tool_not_registered",
                "dispatch",
                false,
                false,
                &error.to_string(),
            );
            catalog.insert_source_metadata(&name, &mut metadata);
            insert_approval_execution_metadata(&mut metadata, approval_granted, Some(&error));
            merge_metadata_overlay(&mut metadata, metadata_overlay.as_ref());
            insert_work_item_metadata(&mut metadata, active_work_item_id.as_deref());
            insert_tool_execution_duration(&mut metadata, &execution_started_at);
            events.push(AgentEventPayload::ToolCallFinished {
                result: ToolResult {
                    call_id: call.id,
                    output: error.to_string(),
                    content: vec![ModelContentPart::text(error.to_string())],
                    metadata,
                },
            });
            return ToolExecutionReport {
                outcome: ToolExecutionOutcome::Failed(error),
                events,
            };
        };

        // Executors consume only the immutable services captured for this
        // invocation and cannot reach back into AgentCore's mutable loop state.
        context.current_work_form = context.current_work_form.clone();
        let executor = ToolExecutor::new(tool, context);
        let mut result = match executor.execute(call.clone()).await {
            Ok(result) => result,
            Err(error) => {
                if let Some(boundary) = ToolApprovalBoundary::from_error(call.id, &error) {
                    return ToolExecutionReport {
                        outcome: ToolExecutionOutcome::NeedsApproval(boundary),
                        events: Vec::new(),
                    };
                }
                let error_message = format!("{error:#}");
                let mut metadata = json!({
                    "toolName": &name,
                    "success": false,
                    "error": &error_message,
                });
                insert_classified_anyhow_error_record(&mut metadata, &error);
                catalog.insert_source_metadata(&name, &mut metadata);
                insert_approval_execution_metadata(&mut metadata, approval_granted, Some(&error));
                merge_metadata_overlay(&mut metadata, metadata_overlay.as_ref());
                insert_work_item_metadata(&mut metadata, active_work_item_id.as_deref());
                insert_tool_execution_duration(&mut metadata, &execution_started_at);
                events.push(AgentEventPayload::ToolCallFinished {
                    result: ToolResult {
                        call_id: call.id,
                        output: error_message.clone(),
                        content: vec![ModelContentPart::text(error_message)],
                        metadata,
                    },
                });
                return ToolExecutionReport {
                    outcome: ToolExecutionOutcome::Failed(error),
                    events,
                };
            }
        };
        if let Some(object) = result.metadata.as_object_mut() {
            object.insert("toolName".to_string(), json!(&name));
        }
        catalog.insert_source_metadata(&name, &mut result.metadata);
        insert_approval_execution_metadata(&mut result.metadata, approval_granted, None);
        merge_metadata_overlay(&mut result.metadata, metadata_overlay.as_ref());
        insert_work_item_metadata(&mut result.metadata, active_work_item_id.as_deref());
        result = normalize_tool_result_at_ingress(
            &name,
            result,
            result_store.as_ref(),
            result_thread_id,
        );
        if tool_class == ToolClass::WorkForm {
            let persistence = (|| -> anyhow::Result<()> {
                if let (Some(store), Some(form)) = (
                    result_store.as_ref(),
                    result
                        .metadata
                        .get("workForm")
                        .and_then(|value| serde_json::from_value::<WorkForm>(value.clone()).ok()),
                ) {
                    store.upsert_work_form(&form)?;
                }
                Ok(())
            })();
            if let Err(error) = persistence {
                result.output = format!("failed to persist WorkForm: {error:#}");
                insert_classified_anyhow_error_record(&mut result.metadata, &error);
                if let Some(metadata) = result.metadata.as_object_mut() {
                    metadata.insert("success".to_string(), json!(false));
                    metadata.insert("error".to_string(), json!(result.output));
                }
            }
        }
        insert_tool_execution_duration(&mut result.metadata, &execution_started_at);
        crate::tool_error::ensure_tool_error_record(&mut result);
        events.push(AgentEventPayload::ToolCallFinished {
            result: result.clone(),
        });
        if tool_class == ToolClass::WorkForm {
            if let Some(form) = result
                .metadata
                .get("workForm")
                .and_then(|value| serde_json::from_value::<WorkForm>(value.clone()).ok())
            {
                events.push(AgentEventPayload::WorkFormUpdated { form });
            }
        }
        ToolExecutionReport {
            outcome: ToolExecutionOutcome::Completed(result),
            events,
        }
    }

    async fn execute_provider_call(
        &self,
        input: ProviderToolExecutionInput,
    ) -> ProviderToolExecutionReport {
        let ProviderToolExecutionInput {
            catalog,
            provider_call,
            user_message_id,
            agent_path,
            context,
            background,
            turn_inbox,
        } = input;
        let call = provider_event_call(&provider_call, user_message_id, &agent_path);
        if let Some(result) = self.validate_provider_call(&catalog, &provider_call) {
            return ProviderToolExecutionReport {
                provider_call,
                events: provider_result_events(call, &result),
                outcome: ToolExecutionOutcome::Completed(result),
            };
        }

        let active_turn = match (context.state.as_ref(), context.thread_id) {
            (Some(store), Some(thread_id)) => match store.get_active_turn(thread_id) {
                Ok(turn) => turn.filter(|turn| {
                    turn.turn_id == user_message_id || turn.user_message_id == user_message_id
                }),
                Err(error) => {
                    return ProviderToolExecutionReport {
                        provider_call,
                        outcome: ToolExecutionOutcome::Failed(error),
                        events: Vec::new(),
                    };
                }
            },
            _ => None,
        };
        // Conversation effects use the persisted Turn id. Flow Agent nodes do
        // not create synthetic conversation Turns; their prepared context
        // carries the durable FlowRun id instead. The journal accepts either
        // logical execution scope after migration v28.
        let durable_effect_scope = if let Some(turn) = active_turn.as_ref() {
            Some(turn.turn_id)
        } else if let (Some(store), Some(flow_run_id)) =
            (context.state.as_ref(), context.agent_turn_id)
        {
            match store.flow_session_store().get_flow_run(flow_run_id) {
                Ok(Some(_)) => Some(flow_run_id),
                Ok(None) => None,
                Err(error) => {
                    return ProviderToolExecutionReport {
                        provider_call,
                        outcome: ToolExecutionOutcome::Failed(error),
                        events: Vec::new(),
                    };
                }
            }
        } else {
            None
        };
        let completion_sink = match (
            context.state.as_ref(),
            context.thread_id,
            active_turn.as_ref(),
        ) {
            (Some(store), Some(thread_id), Some(turn)) => {
                Some(Arc::new(DurableAsyncToolResultSink::new(
                    thread_id,
                    turn.turn_id,
                    user_message_id,
                    agent_path.clone(),
                    provider_call.name.clone(),
                    store.clone(),
                    turn_inbox,
                )) as Arc<dyn BackgroundCompletionSink>)
            }
            _ => None,
        };
        let completion_scope = context.thread_id.map(|thread_id| BackgroundScope {
            thread_id,
            agent_path: agent_path.clone(),
        });
        let execution_policy = catalog
            .get(&provider_call.name)
            .map(|tool| tool.execution_policy(&call));
        let mut journal = None;
        if let (Some(store), Some(thread_id), Some(policy), Some(effect_scope_id)) = (
            context.state.as_ref(),
            context.thread_id,
            execution_policy.as_ref(),
            durable_effect_scope,
        ) {
            let input_hash = content_fingerprint(
                serde_json::to_vec(&provider_call.arguments)
                    .unwrap_or_default()
                    .as_slice(),
            );
            let intent = EffectIntent {
                thread_id,
                turn_id: effect_scope_id,
                agent_path: agent_path.clone(),
                idempotency_key: format!(
                    "{}/{}/{}/{}",
                    effect_scope_id, agent_path, provider_call.name, provider_call.id
                ),
                kind: EffectKind::ToolCall,
                operation: provider_call.name.clone(),
                input_hash,
                input: provider_call.arguments.clone(),
                side_effect_class: effect_side_effect_class(policy.side_effect),
                idempotent: policy.idempotent,
            };
            let prepared = match store.prepare_effect(&intent) {
                Ok(prepared) => prepared,
                Err(error) => {
                    return ProviderToolExecutionReport {
                        provider_call,
                        outcome: ToolExecutionOutcome::Failed(error),
                        events: Vec::new(),
                    };
                }
            };
            if prepared.status == EffectStatus::Succeeded {
                let replayed = prepared
                    .result
                    .ok_or_else(|| {
                        anyhow::anyhow!("succeeded tool effect is missing its replayable result")
                    })
                    .and_then(|value| {
                        serde_json::from_value::<ProviderToolResult>(value)
                            .map_err(anyhow::Error::from)
                    });
                return match replayed {
                    Ok(mut replayed) => {
                        if let Some(metadata) = replayed.metadata.as_object_mut() {
                            metadata.insert("effectJournalReplay".to_string(), json!(true));
                            metadata.insert("effectId".to_string(), json!(prepared.effect_id));
                        }
                        ProviderToolExecutionReport {
                            provider_call,
                            events: provider_result_events(call, &replayed),
                            outcome: ToolExecutionOutcome::Completed(replayed),
                        }
                    }
                    Err(error) => ProviderToolExecutionReport {
                        provider_call,
                        outcome: ToolExecutionOutcome::Failed(error),
                        events: Vec::new(),
                    },
                };
            }
            if prepared.requires_reconciliation() || prepared.status == EffectStatus::Running {
                let reason = if prepared.status == EffectStatus::Running {
                    "the same durable tool effect is still marked running"
                } else {
                    "a previous attempt may have produced a non-idempotent side effect"
                };
                let output = format!(
                    "Tool execution requires reconciliation: {reason}. Inspect the target state or ask the user before attempting the action again."
                );
                let mut metadata = json!({
                    "success": false,
                    "effectId": prepared.effect_id,
                    "effectStatus": prepared.status,
                    "reconciliationRequired": true,
                });
                insert_tool_error_record(
                    &mut metadata,
                    "effect_reconciliation_required",
                    "preflight",
                    false,
                    true,
                    &output,
                );
                let blocked = ProviderToolResult {
                    call_id: provider_call.id.clone(),
                    name: provider_call.name.clone(),
                    output: output.clone(),
                    content: vec![ModelContentPart::text(output)],
                    is_error: true,
                    metadata,
                };
                return ProviderToolExecutionReport {
                    provider_call,
                    events: provider_result_events(call, &blocked),
                    outcome: ToolExecutionOutcome::Completed(blocked),
                };
            }
            match store.start_effect(prepared.effect_id) {
                Ok(running) => {
                    journal = Some((store.clone(), running.effect_id, policy.clone()));
                }
                Err(error) => {
                    return ProviderToolExecutionReport {
                        provider_call,
                        outcome: ToolExecutionOutcome::Failed(error),
                        events: Vec::new(),
                    };
                }
            }
        }

        let execution = self
            .execute_call(ToolExecutionInput {
                catalog: catalog.clone(),
                call,
                context,
                metadata_overlay: Some(json!({ "providerToolCallId": &provider_call.id })),
            })
            .await;
        let events = execution.events;
        let mut provider_outcome = match execution.outcome {
            ToolExecutionOutcome::Completed(result) => self
                .normalize_provider_result(&catalog, &provider_call, Ok(result))
                .map_or_else(
                    ToolExecutionOutcome::Failed,
                    ToolExecutionOutcome::Completed,
                ),
            ToolExecutionOutcome::NeedsApproval(boundary) => {
                ToolExecutionOutcome::NeedsApproval(boundary)
            }
            ToolExecutionOutcome::Failed(error) => self
                .normalize_provider_result(&catalog, &provider_call, Err(error))
                .map_or_else(
                    ToolExecutionOutcome::Failed,
                    ToolExecutionOutcome::Completed,
                ),
        };

        if let (ToolExecutionOutcome::Completed(result), Some(sink), Some(scope)) =
            (&mut provider_outcome, completion_sink, completion_scope)
        {
            if let Some(accepted) = AcceptedToolResult::from_provider_result(result) {
                let delivery =
                    match background.attach_completion_sink(&scope, accepted.job_id, sink) {
                        Ok(()) => json!("durable"),
                        Err(error) => json!({
                            "mode": "registry_fallback",
                            "error": error.to_string(),
                        }),
                    };
                if let Some(metadata) = result.metadata.as_object_mut() {
                    metadata.insert("asyncCompletionDelivery".to_string(), delivery);
                }
            }
        }

        if let Some((store, effect_id, policy)) = journal {
            let finish = match &provider_outcome {
                ToolExecutionOutcome::Completed(result) if result.is_error => {
                    let executed = result
                        .metadata
                        .get("errorRecord")
                        .and_then(|record| record.get("executed"))
                        .and_then(Value::as_bool)
                        .unwrap_or(true);
                    let status = if !executed
                        || policy.side_effect == ToolSideEffect::None
                        || policy.idempotent
                    {
                        EffectStatus::Failed
                    } else {
                        EffectStatus::Indeterminate
                    };
                    let error = result
                        .metadata
                        .get("errorRecord")
                        .and_then(|record| record.get("message"))
                        .or_else(|| result.metadata.get("error"))
                        .and_then(Value::as_str)
                        .unwrap_or(&result.output)
                        .to_string();
                    serde_json::to_value(result)
                        .map_err(anyhow::Error::from)
                        .and_then(|value| {
                            store.finish_effect(effect_id, status, Some(value), Some(error))
                        })
                }
                ToolExecutionOutcome::Completed(result) => serde_json::to_value(result)
                    .map_err(anyhow::Error::from)
                    .and_then(|value| {
                        store.finish_effect(effect_id, EffectStatus::Succeeded, Some(value), None)
                    }),
                ToolExecutionOutcome::NeedsApproval(boundary) => store.finish_effect(
                    effect_id,
                    EffectStatus::Failed,
                    None,
                    Some(format!("approval required: {}", boundary.reason)),
                ),
                ToolExecutionOutcome::Failed(error) => {
                    let status = if policy.side_effect == ToolSideEffect::None || policy.idempotent
                    {
                        EffectStatus::Failed
                    } else {
                        EffectStatus::Indeterminate
                    };
                    store.finish_effect(effect_id, status, None, Some(error.to_string()))
                }
            };
            if let Err(error) = finish {
                provider_outcome = ToolExecutionOutcome::Failed(error);
            }
        }
        ProviderToolExecutionReport {
            provider_call,
            outcome: provider_outcome,
            events,
        }
    }

    async fn execute_provider_batch(
        &self,
        inputs: Vec<ProviderToolExecutionInput>,
    ) -> Vec<ProviderToolExecutionReport> {
        // join_all retains input order, so concurrency never changes provider
        // result order or the order in which callers publish local events.
        join_all(
            inputs
                .into_iter()
                .map(|input| self.execute_provider_call(input)),
        )
        .await
    }
}

fn insert_work_item_metadata(metadata: &mut Value, item_id: Option<&str>) {
    let Some(item_id) = item_id else {
        return;
    };
    if let Some(object) = metadata.as_object_mut() {
        object.insert("workItemId".to_string(), json!(item_id));
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
    let denied = error.is_some_and(|error| approval_required(error).is_some());
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

fn merge_metadata_overlay(metadata: &mut Value, overlay: Option<&Value>) {
    let Some(Value::Object(overlay)) = overlay else {
        return;
    };
    if !metadata.is_object() {
        *metadata = json!({});
    }
    if let Some(object) = metadata.as_object_mut() {
        for (key, value) in overlay {
            object.insert(key.clone(), value.clone());
        }
    }
}

fn effect_side_effect_class(side_effect: ToolSideEffect) -> EffectSideEffectClass {
    match side_effect {
        ToolSideEffect::None => EffectSideEffectClass::None,
        ToolSideEffect::WorkspaceWrite => EffectSideEffectClass::Workspace,
        ToolSideEffect::External
        | ToolSideEffect::Process
        | ToolSideEffect::SessionMutation
        | ToolSideEffect::ControlPlane => EffectSideEffectClass::External,
        ToolSideEffect::Unknown => EffectSideEffectClass::Unknown,
    }
}

fn provider_event_call(
    provider_call: &ProviderToolCall,
    turn_id: Uuid,
    agent_path: &str,
) -> ToolCall {
    let logical_name = format!("{agent_path}\0{}", provider_call.id);
    ToolCall {
        id: Uuid::new_v5(&turn_id, logical_name.as_bytes()),
        name: provider_call.name.clone(),
        input: provider_call.arguments.clone(),
    }
}

fn provider_result_events(call: ToolCall, result: &ProviderToolResult) -> Vec<AgentEventPayload> {
    let mut metadata = result.metadata.clone();
    if !metadata.is_object() {
        metadata = json!({});
    }
    if let Some(object) = metadata.as_object_mut() {
        object.insert("providerToolCallId".to_string(), json!(&result.call_id));
        object.insert("toolName".to_string(), json!(&result.name));
        object.insert("isError".to_string(), json!(result.is_error));
        object.insert("success".to_string(), json!(!result.is_error));
    }
    vec![
        AgentEventPayload::ToolCallStarted { call: call.clone() },
        AgentEventPayload::ToolCallFinished {
            result: ToolResult {
                call_id: call.id,
                output: result.output.clone(),
                content: result.content.clone(),
                metadata,
            },
        },
    ]
}

fn insert_tool_execution_duration(metadata: &mut Value, started_at: &Instant) {
    if !metadata.is_object() {
        *metadata = json!({});
    }
    let duration_ms = u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
    if let Some(object) = metadata.as_object_mut() {
        object.insert(
            TOOL_EXECUTION_DURATION_MS_KEY.to_string(),
            json!(duration_ms),
        );
    }
}

fn scheduling_conflicts(
    resource_keys: &mut HashMap<String, bool>,
    policy: &crate::tools::ToolExecutionPolicy,
) -> bool {
    let writes_resource = !policy.read_only;
    let conflicts = policy.resource_keys.iter().any(|key| {
        resource_keys.iter().any(|(selected_key, selected_writes)| {
            let same_resource = key == selected_key
                || key == "*"
                || key == "workspace:*"
                || selected_key == "*"
                || selected_key == "workspace:*";
            same_resource && (writes_resource || *selected_writes)
        })
    });
    if conflicts {
        return true;
    }
    for key in &policy.resource_keys {
        resource_keys
            .entry(key.clone())
            .and_modify(|selected_writes| *selected_writes |= writes_resource)
            .or_insert(writes_resource);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::background::{BackgroundJobSnapshot, BackgroundJobStatus};
    use crate::model::TurnRecord;
    use crate::store::SqliteSessionStore;
    use crate::turn_inbox::BufferedTurnInbox;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use std::time::Duration;

    fn accepts_object_safe_runtime(_runtime: &dyn ToolRuntime) {}

    #[test]
    fn default_runtime_implements_the_object_safe_tool_port() {
        accepts_object_safe_runtime(&LocalToolRuntime::new());
    }

    #[test]
    fn background_acceptance_and_terminal_result_use_different_correlation_ids() {
        let job_id = Uuid::new_v4();
        let accepted_result = ProviderToolResult {
            call_id: "provider_call_1".to_string(),
            name: "shell".to_string(),
            output: "running".to_string(),
            content: Vec::new(),
            is_error: false,
            metadata: json!({
                "background": true,
                "jobId": job_id,
            }),
        };
        let accepted = AcceptedToolResult::from_provider_result(&accepted_result)
            .expect("background result is an acceptance");
        let terminal = AsyncToolResult {
            job_id,
            tool_name: "background_job".to_string(),
            output: "finished".to_string(),
            is_error: false,
            metadata: json!({ "jobId": job_id, "asyncToolResult": true }),
        }
        .into_provider_result("runtime_background_completion");

        assert_eq!(accepted.provider_call_id, "provider_call_1");
        assert_eq!(accepted.job_id, job_id);
        assert_ne!(terminal.call_id, accepted.provider_call_id);
        assert_eq!(terminal.metadata["jobId"], json!(job_id));
    }

    struct DelayedTool {
        name: String,
        delay: Duration,
        completions: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl Tool for DelayedTool {
        fn name(&self) -> &str {
            &self.name
        }

        fn description(&self) -> &str {
            "test delayed tool"
        }

        fn schema(&self) -> Value {
            json!({"type": "object", "additionalProperties": false})
        }

        async fn execute(
            &self,
            call: ToolCall,
            _context: ToolInvocationContext,
        ) -> anyhow::Result<ToolResult> {
            tokio::time::sleep(self.delay).await;
            self.completions.lock().unwrap().push(self.name.clone());
            Ok(ToolResult::text(
                call.id,
                self.name.clone(),
                json!({"success": true}),
            ))
        }
    }

    struct ApprovalBoundaryTool {
        executions: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Tool for ApprovalBoundaryTool {
        fn name(&self) -> &str {
            "approval_boundary"
        }

        fn description(&self) -> &str {
            "test approval boundary"
        }

        fn schema(&self) -> Value {
            json!({"type": "object", "additionalProperties": false})
        }

        async fn execute(
            &self,
            call: ToolCall,
            context: ToolInvocationContext,
        ) -> anyhow::Result<ToolResult> {
            if !context.approval_granted {
                return Err(ApprovalRequired::new("approve the exact test call").into());
            }
            self.executions.fetch_add(1, Ordering::SeqCst);
            Ok(ToolResult::text(
                call.id,
                "executed",
                json!({"success": true}),
            ))
        }
    }

    #[tokio::test]
    async fn approval_boundary_is_not_published_as_failure_and_replay_keeps_logical_id() {
        let executions = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::default();
        registry.insert(
            "approval_boundary".to_string(),
            Arc::new(ApprovalBoundaryTool {
                executions: Arc::clone(&executions),
            }),
        );
        let catalog = ToolRuntimeCatalog::new(
            registry,
            vec![ProviderToolCandidate::direct(
                "approval_boundary",
                "test approval boundary",
                json!({"type": "object", "additionalProperties": false}),
            )],
            CapabilityProjection::unrestricted(),
            None,
            HashSet::new(),
            HashSet::new(),
        );
        let sandbox = LocalSandboxConfig::danger_full_access();
        let policy = Arc::new(BasicPolicyEngine::new_with_sandbox_config(
            std::env::temp_dir(),
            PermissionMode::Approve,
            &sandbox,
        ));
        let context =
            ToolInvocationContext::local_with_sandbox_config(std::env::temp_dir(), policy, sandbox);
        let runtime = LocalToolRuntime::new();
        let turn_id = Uuid::new_v4();
        let provider_call = ProviderToolCall {
            id: "provider-approval-1".to_string(),
            name: "approval_boundary".to_string(),
            arguments: json!({}),
        };
        let input = |context: ToolInvocationContext| ProviderToolExecutionInput {
            catalog: catalog.clone(),
            provider_call: provider_call.clone(),
            user_message_id: turn_id,
            agent_path: "/root".to_string(),
            context,
            background: BackgroundProcessRegistry::default(),
            turn_inbox: Arc::new(BufferedTurnInbox::default()),
        };

        let blocked = runtime.execute_provider_call(input(context.clone())).await;
        let logical_call_id = match blocked.outcome {
            ToolExecutionOutcome::NeedsApproval(boundary) => boundary.logical_call_id,
            other => panic!("expected approval boundary, got {other:?}"),
        };
        assert!(blocked.events.is_empty());
        assert_eq!(executions.load(Ordering::SeqCst), 0);

        let mut approved_context = context;
        approved_context.approval_granted = true;
        let approved = runtime.execute_provider_call(input(approved_context)).await;
        assert!(matches!(
            approved.outcome,
            ToolExecutionOutcome::Completed(_)
        ));
        assert_eq!(approved.events.len(), 2);
        assert!(matches!(
            &approved.events[0],
            AgentEventPayload::ToolCallStarted { call } if call.id == logical_call_id
        ));
        assert!(matches!(
            &approved.events[1],
            AgentEventPayload::ToolCallFinished { result } if result.call_id == logical_call_id
        ));
        assert_eq!(executions.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn runtime_executes_concurrently_but_returns_provider_results_in_order() {
        let completions = Arc::new(Mutex::new(Vec::new()));
        let mut registry = ToolRegistry::default();
        registry.insert(
            "slow".to_string(),
            Arc::new(DelayedTool {
                name: "slow".to_string(),
                delay: Duration::from_millis(30),
                completions: Arc::clone(&completions),
            }),
        );
        registry.insert(
            "fast".to_string(),
            Arc::new(DelayedTool {
                name: "fast".to_string(),
                delay: Duration::from_millis(1),
                completions: Arc::clone(&completions),
            }),
        );
        let catalog = ToolRuntimeCatalog::new(
            registry,
            vec![
                ProviderToolCandidate::direct("slow", "slow", json!({"type":"object"})),
                ProviderToolCandidate::direct("fast", "fast", json!({"type":"object"})),
            ],
            CapabilityProjection::unrestricted(),
            None,
            HashSet::new(),
            HashSet::new(),
        );
        let sandbox = LocalSandboxConfig::danger_full_access();
        let policy = Arc::new(BasicPolicyEngine::new_with_sandbox_config(
            std::env::temp_dir(),
            PermissionMode::FullAccess,
            &sandbox,
        ));
        let context =
            ToolInvocationContext::local_with_sandbox_config(std::env::temp_dir(), policy, sandbox);
        let inbox: Arc<dyn TurnInbox> = Arc::new(BufferedTurnInbox::default());
        let background = BackgroundProcessRegistry::default();
        let input = |id: &str, name: &str| ProviderToolExecutionInput {
            catalog: catalog.clone(),
            provider_call: ProviderToolCall {
                id: id.to_string(),
                name: name.to_string(),
                arguments: json!({}),
            },
            user_message_id: Uuid::new_v4(),
            agent_path: "/root".to_string(),
            context: context.clone(),
            background: background.clone(),
            turn_inbox: Arc::clone(&inbox),
        };
        let runtime = LocalToolRuntime::new();

        let reports = runtime
            .execute_provider_batch(vec![
                input("provider-1", "slow"),
                input("provider-2", "fast"),
            ])
            .await;

        assert_eq!(*completions.lock().unwrap(), vec!["fast", "slow"]);
        assert_eq!(reports.len(), 2);
        assert_eq!(reports[0].provider_call.id, "provider-1");
        assert_eq!(reports[1].provider_call.id, "provider-2");
        assert_eq!(
            reports[0].outcome.as_completed().unwrap().call_id,
            "provider-1"
        );
        assert_eq!(
            reports[1].outcome.as_completed().unwrap().call_id,
            "provider-2"
        );
        assert_eq!(reports[0].events.len(), 2);
        assert_eq!(reports[1].events.len(), 2);
        let recorded_duration_ms = |report: &ProviderToolExecutionReport| {
            report
                .events
                .iter()
                .find_map(|event| match event {
                    AgentEventPayload::ToolCallFinished { result } => result
                        .metadata
                        .get(TOOL_EXECUTION_DURATION_MS_KEY)
                        .and_then(Value::as_u64),
                    _ => None,
                })
                .expect("finished tool event records its execution duration")
        };
        assert!(recorded_duration_ms(&reports[0]) >= 25);
        assert!(recorded_duration_ms(&reports[1]) >= 1);
    }

    #[test]
    fn durable_async_result_is_appended_and_delivered_to_the_live_turn() {
        let store: Arc<dyn SessionStore> =
            Arc::new(SqliteSessionStore::open(":memory:").expect("open store"));
        let thread = store
            .create_thread(Some("async result".to_string()), std::env::temp_dir())
            .expect("create thread");
        let inbox_turn_id = Uuid::new_v4();
        let turn = store
            .insert_turn(TurnRecord::running(thread.id, inbox_turn_id))
            .expect("insert turn");
        let inbox: Arc<dyn TurnInbox> = Arc::new(BufferedTurnInbox::default());
        let sink = DurableAsyncToolResultSink::new(
            thread.id,
            turn.turn_id,
            inbox_turn_id,
            "/root",
            "shell",
            ToolStateStore::new(Arc::clone(&store)),
            Arc::clone(&inbox),
        );
        let job_id = Uuid::new_v4();

        sink.deliver(BackgroundOutputChunk {
            job: BackgroundJobSnapshot {
                job_id,
                agent_path: "/root".to_string(),
                command: "build".to_string(),
                interactive: false,
                status: BackgroundJobStatus::Exited,
                exit_code: Some(0),
                success: true,
                started_at: Utc::now(),
                finished_at: Some(Utc::now()),
                error: None,
                approval_required: None,
                truncated: false,
                sandbox: None,
                stdout_bytes: 4,
                stderr_bytes: 0,
                dropped_bytes: 0,
                unread_bytes: 4,
            },
            stdout: "done".to_string(),
            stderr: String::new(),
            dropped_bytes: 0,
        })
        .expect("deliver completion");

        let messages = store.list_messages(thread.id).expect("list messages");
        assert_eq!(messages.len(), 2);
        assert!(matches!(
            messages[0].parts.as_slice(),
            [MessagePart::ToolCall { call }] if call.id == job_id
        ));
        assert!(matches!(
            messages[1].parts.as_slice(),
            [MessagePart::ToolResult { result }]
                if result.call_id == job_id
                    && result.metadata["durablyAppended"] == json!(true)
                    && result.metadata["providerToolCallId"]
                        == json!(format!("async_tool_result_{}", job_id.simple()))
        ));
        assert_eq!(
            store
                .list_events(thread.id, None)
                .expect("list events")
                .len(),
            2
        );
        assert!(matches!(
            inbox.drain(inbox_turn_id).as_slice(),
            [TurnInboxItem::AsyncToolResult { result }]
                if result.job_id == job_id
                    && result.tool_name == "shell"
                    && result.metadata["durablyAppended"] == json!(true)
        ));
    }
}
