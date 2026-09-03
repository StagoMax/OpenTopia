use crate::artifact_runtime::ArtifactRuntime;
use crate::background::BackgroundProcessRegistry;
use crate::browser::BrowserRuntime;
use crate::collaboration::AgentCollaborationInvocation;
use crate::computer::{ComputerAccessPolicy, ComputerRuntime};
use crate::enterprise::{AgentKnowledgeBindingV1, CapabilityProjection};
#[cfg(test)]
use crate::execution::ExecRequest;
#[cfg(test)]
use crate::execution::ShellDialect;
use crate::execution::{
    ExecutionContext, ExecutionEnvironment, FileReadRequest, LocalExecutionEnvironment,
};
#[cfg(test)]
use crate::execution_authorization::{ApprovalEscalation, FilesystemAccess};
use crate::execution_authorization::{ExecutionGrant, ProcessLifetime, ToolExecutionIntent};
#[cfg(test)]
use crate::file_mutation::{read_optional, PreparedFileMutation};
use crate::file_mutation::{
    FileMutationBatch, FileMutationBatchResult, FileMutationObserver, FileMutationScope,
};
use crate::flow_runtime::FlowNodeHarness;
use crate::mcp::McpToolDescriptor;
use crate::mcp_host::McpExtensionHost;
#[cfg(test)]
use crate::model::UserInputRequest;
use crate::model::{
    Artifact, ArtifactStorage, CollaborationMode, ModelContentPart, ToolCall, ToolResult,
};
#[cfg(test)]
use crate::model_context::content_fingerprint;
use crate::model_context::CompiledModelContext;
use crate::policy::{ApprovalRequired, PermissionMode, PolicyDecision, PolicyEngine};
use crate::provider::ModelConversationMessage;
use crate::sandbox::LocalSandboxConfig;
#[cfg(test)]
use crate::shell_analysis::analyze_shell_command;
use crate::tool_state::ToolStateStore;
use crate::work_form::WorkForm;
use crate::ConnectionOperationRuntimeRoute;
use anyhow::Context;
use async_trait::async_trait;
#[cfg(test)]
use futures_util::stream::FuturesUnordered;
#[cfg(test)]
use futures_util::StreamExt;
use schemars::{gen::SchemaSettings, JsonSchema};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone)]
pub struct ToolInvocationContext {
    pub(crate) workspace_root: PathBuf,
    pub(crate) policy: Arc<dyn PolicyEngine>,
    pub(crate) permission_mode: PermissionMode,
    pub(crate) environment: Arc<dyn ExecutionEnvironment>,
    /// Base/effective local sandbox profile. `None` is reserved for injected
    /// execution environments whose authorization is enforced externally.
    pub(crate) sandbox_config: Option<LocalSandboxConfig>,
    /// Narrow persistence capabilities available to tool execution. The broad
    /// product SessionStore never crosses this boundary.
    pub state: Option<ToolStateStore>,
    pub thread_id: Option<Uuid>,
    pub cancel: Option<CancellationToken>,
    /// Caller-bound multi-Agent capability. Session, AgentThread, Turn, and
    /// Runtime Snapshot identity are captured by the runtime and cannot be
    /// supplied or overridden by model tool arguments.
    pub collaboration: Option<AgentCollaborationInvocation>,
    /// Commands that outlive the tool call that started them.
    pub background: Option<BackgroundProcessRegistry>,
    /// Runtime-owned lower bound for keeping ordinary commands in the
    /// foreground. Model arguments may extend this window, but cannot shorten
    /// it and turn quick commands into background jobs.
    pub(crate) minimum_foreground_yield: Duration,
    pub agent_turn_id: Option<Uuid>,
    /// Process-shared sink for exact, committed file mutations. The server uses
    /// it to build per-Turn diffs without scanning the workspace at Turn start.
    pub file_mutation_observer: Option<Arc<dyn FileMutationObserver>>,
    pub agent_depth: u8,
    pub agent_path: String,
    pub browser: Option<Arc<dyn BrowserRuntime>>,
    pub computer: Option<Arc<dyn ComputerRuntime>>,
    /// Application executables explicitly approved for model-driven desktop access.
    pub computer_access_policy: ComputerAccessPolicy,
    /// Shared host runtime for deterministic Office parsing, conversion, and rendering.
    pub artifact_runtime: Arc<ArtifactRuntime>,
    /// MCP tools activated for this thread. Attachment analysis may route image bytes only to
    /// this bounded set, never to an arbitrary or merely cached server.
    pub mcp_host: Option<McpExtensionHost>,
    pub mcp_tools: Vec<McpToolDescriptor>,
    /// Exact structured Connection routes keyed by the model-facing alias.
    /// Legacy MCP contexts leave this empty.
    pub connection_operations: BTreeMap<String, ConnectionOperationRuntimeRoute>,
    /// Runtime-owned Agent knowledge binding. Knowledge tools treat the
    /// provider and provider-specific scope as fail-closed authority; model
    /// arguments cannot select a backend or widen its scope.
    pub knowledge_binding: Option<AgentKnowledgeBindingV1>,
    /// Whether the provider selected for this thread accepts native image input.
    /// `view_attachment` uses this to choose native image delivery or an explicitly
    /// declared MCP attachment-inspection capability.
    pub model_supports_vision: bool,
    pub fork_conversation: Vec<ModelConversationMessage>,
    pub fork_model_context: Option<CompiledModelContext>,
    pub current_work_form: Option<WorkForm>,
    pub collaboration_mode: CollaborationMode,
    pub goal_id: Option<Uuid>,
    /// Set only while replaying a tool call that the user explicitly approved.
    /// Browser navigation uses this as a one-time fallback when a caller does not have a
    /// persistent session store from which it can read the approved domain.
    pub approval_granted: bool,
    /// The same fail-closed projection used to build the provider catalog.
    /// Discovery tools must apply it to their result contents as well.
    pub(crate) capability_projection: CapabilityProjection,
    /// A clone of the currently restricted Agent Harness. Flow nodes use this
    /// instead of constructing a second execution stack with wider visibility.
    pub flow_harness: Option<Arc<dyn FlowNodeHarness>>,
}

impl ToolInvocationContext {
    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn permission_mode(&self) -> PermissionMode {
        self.permission_mode
    }

    pub fn environment(&self) -> &dyn ExecutionEnvironment {
        self.environment.as_ref()
    }

    pub fn sandbox_config(&self) -> Option<&LocalSandboxConfig> {
        self.sandbox_config.as_ref()
    }

    pub fn capability_projection(&self) -> &CapabilityProjection {
        &self.capability_projection
    }

    #[cfg(test)]
    pub(crate) fn local(workspace_root: PathBuf, policy: Arc<dyn PolicyEngine>) -> Self {
        Self::local_with_sandbox_config(workspace_root, policy, LocalSandboxConfig::from_env())
    }

    pub(crate) fn local_with_sandbox_config(
        workspace_root: PathBuf,
        policy: Arc<dyn PolicyEngine>,
        sandbox_config: LocalSandboxConfig,
    ) -> Self {
        let context_sandbox_config = sandbox_config.clone();
        let environment = Arc::new(LocalExecutionEnvironment::with_sandbox_config(
            workspace_root.clone(),
            sandbox_config,
        ));
        Self {
            workspace_root,
            policy,
            permission_mode: PermissionMode::Chat,
            environment,
            sandbox_config: Some(context_sandbox_config),
            state: None,
            thread_id: None,
            cancel: None,
            collaboration: None,
            background: None,
            minimum_foreground_yield: Duration::from_millis(DEFAULT_FOREGROUND_YIELD_MILLISECONDS),
            agent_turn_id: None,
            file_mutation_observer: None,
            agent_depth: 0,
            agent_path: "/root".to_string(),
            browser: None,
            computer: None,
            computer_access_policy: ComputerAccessPolicy::default(),
            artifact_runtime: ArtifactRuntime::shared(),
            mcp_host: None,
            mcp_tools: Vec::new(),
            connection_operations: BTreeMap::new(),
            knowledge_binding: None,
            model_supports_vision: true,
            fork_conversation: Vec::new(),
            fork_model_context: None,
            current_work_form: None,
            collaboration_mode: CollaborationMode::Default,
            goal_id: None,
            approval_granted: false,
            capability_projection: CapabilityProjection::deny_all(),
            flow_harness: None,
        }
    }

    pub(crate) fn local_with_authority(
        workspace_root: PathBuf,
        policy: Arc<dyn PolicyEngine>,
        permission_mode: PermissionMode,
        sandbox_config: LocalSandboxConfig,
        capability_projection: CapabilityProjection,
    ) -> Self {
        let mut context = Self::local_with_sandbox_config(workspace_root, policy, sandbox_config);
        context.permission_mode = permission_mode;
        context.capability_projection = capability_projection;
        context
    }

    #[cfg(test)]
    pub(crate) fn with_environment(
        workspace_root: PathBuf,
        policy: Arc<dyn PolicyEngine>,
        environment: Arc<dyn ExecutionEnvironment>,
    ) -> Self {
        Self {
            workspace_root,
            policy,
            permission_mode: PermissionMode::Chat,
            environment,
            sandbox_config: None,
            state: None,
            thread_id: None,
            cancel: None,
            collaboration: None,
            background: None,
            minimum_foreground_yield: Duration::from_millis(DEFAULT_FOREGROUND_YIELD_MILLISECONDS),
            agent_turn_id: None,
            file_mutation_observer: None,
            agent_depth: 0,
            agent_path: "/root".to_string(),
            browser: None,
            computer: None,
            computer_access_policy: ComputerAccessPolicy::default(),
            artifact_runtime: ArtifactRuntime::shared(),
            mcp_host: None,
            mcp_tools: Vec::new(),
            connection_operations: BTreeMap::new(),
            knowledge_binding: None,
            model_supports_vision: true,
            fork_conversation: Vec::new(),
            fork_model_context: None,
            current_work_form: None,
            collaboration_mode: CollaborationMode::Default,
            goal_id: None,
            approval_granted: false,
            capability_projection: CapabilityProjection::deny_all(),
            flow_harness: None,
        }
    }

    fn execution_context(&self, timeout: Duration) -> ExecutionContext {
        let context = ExecutionContext::with_timeout(timeout);
        match &self.cancel {
            Some(cancel) => context.with_cancel(cancel.clone()),
            None => context,
        }
    }

    pub(super) async fn commit_file_mutations(
        &self,
        batch: &FileMutationBatch,
    ) -> anyhow::Result<FileMutationBatchResult> {
        let scope = self.file_mutation_scope()?;
        batch
            .commit_observed(
                self.environment.as_ref(),
                self.file_mutation_observer.as_deref(),
                scope.as_ref(),
            )
            .await
    }

    fn file_mutation_scope(&self) -> anyhow::Result<Option<FileMutationScope>> {
        match (
            self.file_mutation_observer.as_deref(),
            self.thread_id,
            self.agent_turn_id,
        ) {
            (Some(_), Some(thread_id), Some(turn_id)) => Ok(Some(FileMutationScope {
                thread_id,
                turn_id,
                agent_path: self.agent_path.clone(),
                workspace_root: self.workspace_root.clone(),
            })),
            (Some(_), _, _) => {
                anyhow::bail!("file mutation journaling requires thread and turn identity")
            }
            (None, _, _) => Ok(None),
        }
    }

    fn apply_execution_intent(
        &mut self,
        intent: &ToolExecutionIntent,
    ) -> anyhow::Result<Option<ExecutionGrant>> {
        let Some(base) = self.sandbox_config.as_ref() else {
            return Ok(None);
        };
        let grant = ExecutionGrant::resolve(
            base,
            &self.workspace_root,
            intent,
            // `approval_granted` lets a tool replay an Ask decision, but it is
            // not itself filesystem authority. The approved ExecutionGrant is
            // materialized by AgentCore before constructing this context.
            false,
        )?;
        self.environment = Arc::new(LocalExecutionEnvironment::with_sandbox_config(
            self.workspace_root.clone(),
            grant.sandbox.clone(),
        ));
        self.sandbox_config = Some(grant.sandbox.clone());
        Ok(Some(grant))
    }
}

fn enforce_policy_decision(
    decision: PolicyDecision,
    context: &ToolInvocationContext,
) -> anyhow::Result<()> {
    match context.permission_mode.resolve_policy_decision(decision) {
        PolicyDecision::Allow => Ok(()),
        PolicyDecision::Deny { reason } => anyhow::bail!("denied: {reason}"),
        PolicyDecision::Ask { .. } if context.approval_granted => Ok(()),
        PolicyDecision::Ask { reason } => Err(ApprovalRequired::new(reason).into()),
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> Value;
    fn input_error(&self, input: &Value) -> Option<String> {
        crate::provider::tool_input_schema_error(&self.schema(), input, "arguments")
    }
    fn has_derived_input_schema(&self) -> bool {
        false
    }
    /// Name of the trusted discovery tool that must load this tool's precise
    /// provider-facing contract before it is exposed to the model.
    fn provider_contract_loader(&self) -> Option<&str> {
        None
    }
    /// Scheduling facts, separate from the model-facing schema. AgentCore may use
    /// these to run independent observations concurrently without guessing from
    /// tool names. The default is intentionally conservative for plugins/MCP.
    fn execution_policy(&self, _call: &ToolCall) -> ToolExecutionPolicy {
        ToolExecutionPolicy::conservative()
    }
    /// Semantic local authority requested by this call. The dispatcher and
    /// policy layer resolve it against the active session profile; tools never
    /// configure platform ACLs themselves.
    fn execution_intent(&self, call: &ToolCall, _workspace_root: &Path) -> ToolExecutionIntent {
        self.execution_policy(call).execution_intent()
    }
    /// Pure authorization preview used only to group calls that are already
    /// known to cross an approval boundary. `None` means the tool cannot make
    /// that decision without its ordinary execution-time validation. The
    /// dispatcher must then keep the existing sequential path; it must never
    /// execute a tool merely to discover whether approval is required.
    fn authorization_preflight(
        &self,
        _call: &ToolCall,
        _ctx: &ToolInvocationContext,
    ) -> Option<PolicyDecision> {
        None
    }
    async fn execute(
        &self,
        call: ToolCall,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult>;
}

/// Static tools declare their input exactly once. The associated Rust type is
/// used both to derive the model-facing JSON Schema and to deserialize the
/// value that reaches policy evaluation and execution. Tools whose contracts
/// are discovered dynamically (currently MCP) continue to implement `Tool`
/// directly.
#[async_trait]
trait TypedTool: Send + Sync {
    type Input: DeserializeOwned + JsonSchema + Send + 'static;

    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn provider_contract_loader(&self) -> Option<&str> {
        None
    }
    fn validate_context(&self, _ctx: &ToolInvocationContext) -> anyhow::Result<()> {
        Ok(())
    }
    fn execution_policy(&self, _input: &Self::Input) -> ToolExecutionPolicy {
        ToolExecutionPolicy::conservative()
    }
    fn execution_intent(&self, input: &Self::Input, _workspace_root: &Path) -> ToolExecutionIntent {
        self.execution_policy(input).execution_intent()
    }
    fn authorization_preflight(
        &self,
        _input: &Self::Input,
        _ctx: &ToolInvocationContext,
    ) -> Option<PolicyDecision> {
        None
    }
    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        _ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult>;
}

fn derived_tool_schema<T: JsonSchema>() -> Value {
    let mut settings = SchemaSettings::draft07();
    settings.inline_subschemas = true;
    settings.meta_schema = None;
    let schema = settings.into_generator().into_root_schema_for::<T>();
    let mut value = serde_json::to_value(schema).expect("derived tool schema must serialize");
    if let Some(object) = value.as_object_mut() {
        object.remove("title");
        object.remove("definitions");
        let object_union = ["anyOf", "oneOf"].iter().any(|keyword| {
            object
                .get(*keyword)
                .and_then(Value::as_array)
                .is_some_and(|branches| {
                    !branches.is_empty()
                        && branches.iter().all(|branch| {
                            branch.get("type").and_then(Value::as_str) == Some("object")
                        })
                })
        });
        if object.get("type").is_none() && object_union {
            // Function-tool transports require an object at the root even when
            // Draft 7 can infer it from an anyOf/oneOf of object variants.
            object.insert("type".to_string(), Value::String("object".to_string()));
        }
    }
    value
}

fn decode_typed_tool_input<T: DeserializeOwned>(
    tool_name: &str,
    input: Value,
) -> anyhow::Result<T> {
    serde_json::from_value(input)
        .with_context(|| format!("invalid arguments for tool `{tool_name}`"))
}

macro_rules! impl_typed_tool {
    ($tool:ty) => {
        #[async_trait]
        impl Tool for $tool {
            fn name(&self) -> &str {
                <Self as TypedTool>::name(self)
            }

            fn description(&self) -> &str {
                <Self as TypedTool>::description(self)
            }

            fn schema(&self) -> Value {
                derived_tool_schema::<<Self as TypedTool>::Input>()
            }

            fn input_error(&self, input: &Value) -> Option<String> {
                crate::provider::tool_input_schema_error(&self.schema(), input, "arguments")
                    .or_else(|| {
                        serde_json::from_value::<<Self as TypedTool>::Input>(input.clone())
                            .err()
                            .map(|error| {
                                format!("arguments do not match the derived input type: {error}")
                            })
                    })
            }

            fn has_derived_input_schema(&self) -> bool {
                true
            }

            fn provider_contract_loader(&self) -> Option<&str> {
                <Self as TypedTool>::provider_contract_loader(self)
            }

            fn execution_policy(&self, call: &ToolCall) -> ToolExecutionPolicy {
                decode_typed_tool_input::<<Self as TypedTool>::Input>(
                    <Self as TypedTool>::name(self),
                    call.input.clone(),
                )
                .map(|input| <Self as TypedTool>::execution_policy(self, &input))
                .unwrap_or_else(|_| ToolExecutionPolicy::conservative())
            }

            fn execution_intent(
                &self,
                call: &ToolCall,
                workspace_root: &Path,
            ) -> ToolExecutionIntent {
                decode_typed_tool_input::<<Self as TypedTool>::Input>(
                    <Self as TypedTool>::name(self),
                    call.input.clone(),
                )
                .map(|input| <Self as TypedTool>::execution_intent(self, &input, workspace_root))
                .unwrap_or_default()
            }

            fn authorization_preflight(
                &self,
                call: &ToolCall,
                ctx: &ToolInvocationContext,
            ) -> Option<PolicyDecision> {
                decode_typed_tool_input::<<Self as TypedTool>::Input>(
                    <Self as TypedTool>::name(self),
                    call.input.clone(),
                )
                .ok()
                .map(|input| {
                    let intent =
                        <Self as TypedTool>::execution_intent(self, &input, &ctx.workspace_root);
                    let intent_decision = ctx
                        .policy
                        .inspect_execution_intent(&intent, &ctx.workspace_root);
                    let tool_decision =
                        <Self as TypedTool>::authorization_preflight(self, &input, ctx)
                            .unwrap_or(PolicyDecision::Allow);
                    PolicyDecision::combine([intent_decision, tool_decision])
                })
            }

            async fn execute(
                &self,
                call: ToolCall,
                mut ctx: ToolInvocationContext,
            ) -> anyhow::Result<ToolResult> {
                <Self as TypedTool>::validate_context(self, &ctx)?;
                let input = decode_typed_tool_input::<<Self as TypedTool>::Input>(
                    <Self as TypedTool>::name(self),
                    call.input,
                )?;
                let intent =
                    <Self as TypedTool>::execution_intent(self, &input, &ctx.workspace_root);
                let intent_decision = ctx
                    .policy
                    .inspect_execution_intent(&intent, &ctx.workspace_root);
                let tool_decision =
                    <Self as TypedTool>::authorization_preflight(self, &input, &ctx)
                        .unwrap_or(PolicyDecision::Allow);
                enforce_policy_decision(
                    PolicyDecision::combine([intent_decision, tool_decision]),
                    &ctx,
                )?;
                ctx.apply_execution_intent(&intent)?;
                <Self as TypedTool>::execute_typed(self, call.id, input, ctx).await
            }
        }
    };
}

mod office_tools;
pub use office_tools::{DocumentTool, PdfTool};
mod filesystem_tool;
pub use filesystem_tool::FilesystemTool;
impl_typed_tool!(FilesystemTool);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSideEffect {
    None,
    WorkspaceWrite,
    Process,
    SessionMutation,
    ControlPlane,
    External,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolExecutionPolicy {
    pub read_only: bool,
    pub idempotent: bool,
    pub parallel_safe: bool,
    pub side_effect: ToolSideEffect,
    /// Logical resources touched by this call. Equal keys conflict; `*` is a
    /// global barrier. Paths remain logical and are not trusted for authorization.
    pub resource_keys: Vec<String>,
}

impl ToolExecutionPolicy {
    pub fn conservative() -> Self {
        Self {
            read_only: false,
            idempotent: false,
            parallel_safe: false,
            side_effect: ToolSideEffect::Unknown,
            resource_keys: vec!["*".to_string()],
        }
    }

    pub fn read_only(resource_keys: Vec<String>) -> Self {
        Self {
            read_only: true,
            idempotent: true,
            parallel_safe: true,
            side_effect: ToolSideEffect::None,
            resource_keys,
        }
    }

    pub fn execution_intent(&self) -> ToolExecutionIntent {
        if self.read_only {
            return ToolExecutionIntent::observation([]);
        }
        match self.side_effect {
            ToolSideEffect::None => ToolExecutionIntent::default(),
            ToolSideEffect::WorkspaceWrite => ToolExecutionIntent::workspace_mutation([]),
            ToolSideEffect::Process => {
                ToolExecutionIntent::session_process(ProcessLifetime::OneShot)
            }
            ToolSideEffect::External => ToolExecutionIntent::external(),
            ToolSideEffect::SessionMutation
            | ToolSideEffect::ControlPlane
            | ToolSideEffect::Unknown => ToolExecutionIntent::default(),
        }
    }
}

mod descriptor;
pub(crate) use descriptor::{RegisteredTool, ToolGovernance};
pub use descriptor::{
    ToolApprovalMode, ToolCapabilityDescriptor, ToolClass, ToolRiskLevel, ToolSource,
};
mod registry;
pub use registry::ToolRegistry;

fn truncate_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    value.chars().take(limit).collect()
}

mod spreadsheet_tool;

mod office_resource;
use office_resource::DocumentResourceRef;
mod document_protocol_tools;
mod document_session;
mod spreadsheet_operations;
pub use document_protocol_tools::{
    DocumentExecuteTool, DocumentGetOperationSchemasTool, DocumentOpenTool,
};
impl_typed_tool!(DocumentOpenTool);
impl_typed_tool!(DocumentGetOperationSchemasTool);
impl_typed_tool!(DocumentExecuteTool);

mod request_user_input_tool;
pub use request_user_input_tool::RequestUserInputTool;

mod work_form_tools;
pub use work_form_tools::UpdatePlanTool;
mod skill_tools;
pub use skill_tools::{CreateSkillTool, ListSkillsTool, ReadSkillTool};
#[cfg(test)]
use skill_tools::{EmptyToolInput, ReadSkillInput};

mod browser_tool;
#[cfg(test)]
use browser_tool::{browser_destination_host, configured_browser_hosts};
pub use browser_tool::{
    browser_handoff_for_node, browser_handoff_required, BrowserHandoffRequired, BrowserTool,
};

mod computer_tool;
#[cfg(test)]
use computer_tool::ComputerInput;
pub use computer_tool::ComputerTool;

mod collaboration_tools;
use collaboration_tools::{await_cancellable, MAX_WAIT_TIMEOUT_MS};
#[cfg(test)]
use collaboration_tools::{
    AgentTargetMessageInput, AgentWorkspaceModeInput, ListAgentsInput, SpawnAgentInput,
};
pub use collaboration_tools::{
    FollowupAgentTaskTool, InterruptAgentTool, ListAgentsTool, SendAgentMessageTool,
    SpawnAgentTool, WaitAgentTool,
};

fn required_typed_string(input: Option<&str>, key: &str) -> anyhow::Result<String> {
    input
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .with_context(|| format!("{key} must be a non-empty string"))
}
#[cfg(test)]
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListFilesInput {
    /// Directory path relative to workspace. Use `.` for the workspace root.
    path: String,
}

#[cfg(test)]
struct ListFilesTool;

#[cfg(test)]
#[async_trait]
impl TypedTool for ListFilesTool {
    type Input = ListFilesInput;

    fn name(&self) -> &str {
        "list_files"
    }

    fn description(&self) -> &str {
        "List direct children of a directory inside the workspace. Use `.` for the workspace root."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        ToolExecutionPolicy::read_only(vec![tool_resource_key("dir", &input.path)])
    }

    fn execution_intent(&self, input: &Self::Input, _workspace_root: &Path) -> ToolExecutionIntent {
        ToolExecutionIntent::observation([PathBuf::from(&input.path)])
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let relative = input.path.trim();
        anyhow::ensure!(
            !relative.is_empty(),
            "list_files requires a path; use `.` for the workspace root"
        );
        let logical_path = normalize_workspace_path(&ctx.workspace_root, relative)?;
        enforce_read_policy(&ctx, &logical_path)?;
        let path = ctx.environment.resolve_read_path(&logical_path)?;

        let entries = tokio::task::spawn_blocking(move || list_dir_entries(&path))
            .await
            .context("list_files task failed")??;
        Ok(ToolResult {
            call_id,
            output: entries.join("\n"),
            content: Vec::new(),
            metadata: json!({ "count": entries.len() }),
        })
    }
}

#[cfg(test)]
impl_typed_tool!(ListFilesTool);

mod attachment_tool;
pub(crate) use attachment_tool::mcp_tool_declares_image_inspection;
use attachment_tool::{insert_attachment_provenance, read_stored_attachment_file};
#[cfg(test)]
use attachment_tool::{
    mcp_image_inspection_arguments, parse_mcp_image_inspection_binding, select_mcp_image_inspector,
    ReadAttachmentInput, ViewAttachmentInput, ATTACHMENT_RESULT_BOUNDARY,
    MCP_IMAGE_INSPECTION_CAPABILITY,
};
pub use attachment_tool::{ReadAttachmentTool, ViewAttachmentTool};
#[cfg(test)]
const READ_FILE_ARTIFACT_THRESHOLD: usize = 64_000;
#[cfg(test)]
const READ_FILE_WINDOW_CHARS: usize = 16_000;

#[cfg(test)]
#[derive(Debug, Clone, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileReadInput {
    /// File path relative to workspace.
    path: String,
    /// Optional typed read window. Omit it to read the first character window.
    #[serde(default)]
    window: Option<FileReadWindow>,
}

#[cfg(test)]
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(tag = "mode", rename_all = "camelCase", deny_unknown_fields)]
enum FileReadWindow {
    /// Read a character window. Offset defaults to 0 and limit defaults to 16000.
    Characters {
        #[serde(default)]
        offset: Option<u64>,
        #[serde(default)]
        #[schemars(range(min = 1, max = 16000))]
        limit: Option<u64>,
    },
    /// Read an exact one-based source-line range.
    Lines {
        #[serde(rename = "startLine", alias = "start_line")]
        #[schemars(range(min = 1))]
        start_line: u64,
        #[serde(default, rename = "endLine", alias = "end_line")]
        #[schemars(range(min = 1))]
        end_line: Option<u64>,
    },
}

#[cfg(test)]
impl<'de> Deserialize<'de> for FileReadInput {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as _;

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct CurrentInput {
            path: String,
            #[serde(default)]
            window: Option<FileReadWindow>,
        }

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct LegacyInput {
            path: String,
            #[serde(default)]
            offset: Option<u64>,
            #[serde(default)]
            limit: Option<u64>,
            #[serde(default, alias = "start_line")]
            start_line: Option<u64>,
            #[serde(default, alias = "end_line")]
            end_line: Option<u64>,
        }

        let value = Value::deserialize(deserializer)?;
        if value.get("window").is_some() {
            let current = CurrentInput::deserialize(value).map_err(D::Error::custom)?;
            return Ok(Self {
                path: current.path,
                window: current.window,
            });
        }

        let legacy = LegacyInput::deserialize(value).map_err(D::Error::custom)?;
        let line_mode = legacy.start_line.is_some() || legacy.end_line.is_some();
        let character_mode = legacy.offset.is_some() || legacy.limit.is_some();
        if line_mode && character_mode {
            return Err(D::Error::custom(
                "legacy line coordinates cannot be combined with character coordinates; use the typed window field",
            ));
        }
        let window = if line_mode {
            let start_line = legacy
                .start_line
                .ok_or_else(|| D::Error::custom("legacy endLine requires startLine"))?;
            Some(FileReadWindow::Lines {
                start_line,
                end_line: legacy.end_line,
            })
        } else if character_mode {
            Some(FileReadWindow::Characters {
                offset: legacy.offset,
                limit: legacy.limit,
            })
        } else {
            None
        };
        Ok(Self {
            path: legacy.path,
            window,
        })
    }
}

#[cfg(test)]
impl FileReadInput {
    fn is_line_window(&self) -> bool {
        matches!(self.window, Some(FileReadWindow::Lines { .. }))
    }

    fn character_limit(&self) -> Option<u64> {
        match self.window {
            Some(FileReadWindow::Characters { limit, .. }) => limit,
            Some(FileReadWindow::Lines { .. }) | None => None,
        }
    }

    fn set_character_limit(&mut self, limit: u64) {
        match &mut self.window {
            Some(FileReadWindow::Characters { limit: current, .. }) => *current = Some(limit),
            Some(FileReadWindow::Lines { .. }) => {}
            None => {
                self.window = Some(FileReadWindow::Characters {
                    offset: None,
                    limit: Some(limit),
                });
            }
        }
    }
}

#[cfg(test)]
struct ReadFileTool;

#[cfg(test)]
#[async_trait]
impl TypedTool for ReadFileTool {
    type Input = FileReadInput;

    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read a UTF-8 text file inside the workspace. Omit window for the first 16000 characters, or pass exactly one typed window: {mode: \"lines\", startLine, endLine?} for source lines or {mode: \"characters\", offset?, limit?} for a character range. Returns nextLine or nextOffset when more content remains."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        ToolExecutionPolicy::read_only(vec![tool_resource_key("file", &input.path)])
    }

    fn execution_intent(&self, input: &Self::Input, _workspace_root: &Path) -> ToolExecutionIntent {
        ToolExecutionIntent::observation([PathBuf::from(&input.path)])
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        execute_read_file_with_cap(call_id, input, ctx, READ_FILE_WINDOW_CHARS).await
    }
}

#[cfg(test)]
async fn execute_read_file_with_cap(
    call_id: Uuid,
    input: FileReadInput,
    ctx: ToolInvocationContext,
    max_chars: usize,
) -> anyhow::Result<ToolResult> {
    let relative = input.path.trim();
    anyhow::ensure!(!relative.is_empty(), "read_file requires a path");
    let logical_path = normalize_workspace_path(&ctx.workspace_root, relative)?;
    enforce_read_policy(&ctx, &logical_path)?;
    let path = ctx.environment.resolve_read_path(&logical_path)?;

    let read = ctx
        .environment
        .read_file(FileReadRequest::new(&path))
        .await?;
    let contents = String::from_utf8(read.bytes)
        .with_context(|| format!("failed to read {} as UTF-8", read.path.display()))?;
    let content_hash = content_fingerprint(contents.as_bytes());
    let bytes = contents.len();
    let total_chars = contents.chars().count();
    let cap = max_chars.clamp(1, READ_FILE_WINDOW_CHARS);

    let (mut output, mut metadata) = match input.window {
        Some(FileReadWindow::Lines {
            start_line,
            end_line,
        }) => {
            anyhow::ensure!(start_line > 0, "read_file startLine must be at least 1");
            if let Some(end_line) = end_line {
                anyhow::ensure!(
                    end_line >= start_line,
                    "read_file endLine must be greater than or equal to startLine"
                );
            }
            let lines = contents.split_inclusive('\n').collect::<Vec<_>>();
            let total_lines = lines.len();
            let start = usize::try_from(start_line).context("read_file startLine is too large")?;
            if total_lines == 0 {
                anyhow::ensure!(start == 1, "read_file startLine {start} exceeds empty file");
            } else {
                anyhow::ensure!(
                    start <= total_lines,
                    "read_file startLine {start} exceeds total lines {total_lines}"
                );
            }
            let requested_end = end_line
                .map(usize::try_from)
                .transpose()
                .context("read_file endLine is too large")?
                .unwrap_or(total_lines)
                .min(total_lines);
            let start_index = start.saturating_sub(1);
            let start_offset = lines[..start_index]
                .iter()
                .map(|line| line.chars().count())
                .sum::<usize>();
            let mut selected = String::new();
            let mut actual_end = None;
            for (index, line) in lines
                .iter()
                .enumerate()
                .take(requested_end)
                .skip(start_index)
            {
                let line_chars = line.chars().count();
                anyhow::ensure!(
                    !selected.is_empty() || line_chars <= cap,
                    "read_file line {} contains {line_chars} characters, exceeding the {cap}-character line-mode cap; use offset/limit character mode for this line",
                    index + 1
                );
                if selected.chars().count().saturating_add(line_chars) > cap {
                    break;
                }
                selected.push_str(line);
                actual_end = Some(index + 1);
            }
            let next_line = actual_end
                .filter(|end| *end < requested_end)
                .map(|end| end + 1);
            let next_offset = next_line.map(|line| {
                lines[..line.saturating_sub(1)]
                    .iter()
                    .map(|value| value.chars().count())
                    .sum::<usize>()
            });
            if let Some(next) = next_line {
                selected.push_str(&format!(
                    "\n\n[lines {start}-{} of {total_lines}; call read_file again with window {{\"mode\":\"lines\",\"startLine\":{next}{}}}]",
                    actual_end.unwrap_or(start.saturating_sub(1)),
                    end_line
                        .map(|end| format!(",\"endLine\":{end}"))
                        .unwrap_or_default()
                ));
            }
            (
                selected,
                json!({
                    "path": read.path.display().to_string(),
                    "bytes": bytes,
                    "mode": "lines",
                    "startLine": start,
                    "endLine": actual_end,
                    "requestedEndLine": end_line,
                    "nextLine": next_line,
                    "totalLines": total_lines,
                    "startOffset": start_offset,
                    "nextOffset": next_offset,
                    "totalChars": total_chars
                }),
            )
        }
        character_window => {
            // A window rather than a bare cap: before this, everything past the
            // first 16000 characters of a file was simply unreachable through this
            // tool, and the model could not tell that from a short file.
            let (requested_offset, requested_limit) = match character_window {
                Some(FileReadWindow::Characters { offset, limit }) => (offset, limit),
                Some(FileReadWindow::Lines { .. }) => unreachable!("line window handled above"),
                None => (None, None),
            };
            let offset = requested_offset
                .map(usize::try_from)
                .transpose()
                .context("read_file offset is too large")?
                .unwrap_or(0);
            let limit = requested_limit.map_or(cap, |value| {
                usize::try_from(value).unwrap_or(usize::MAX).clamp(1, cap)
            });
            let window: String = contents.chars().skip(offset).take(limit).collect();
            let read_to = offset.saturating_add(window.chars().count());
            let next_offset = (read_to < total_chars).then_some(read_to);
            let mut selected = window;
            if let Some(next) = next_offset {
                selected.push_str(&format!(
                    "\n\n[characters {offset}-{} of {total_chars}; call read_file again with window {{\"mode\":\"characters\",\"offset\":{next}}} for the rest]",
                    read_to.saturating_sub(1)
                ));
            }
            (
                selected,
                json!({
                    "path": read.path.display().to_string(),
                    "bytes": bytes,
                    "mode": "characters",
                    "offset": offset,
                    "nextOffset": next_offset,
                    "totalChars": total_chars
                }),
            )
        }
    };
    if let Some(object) = metadata.as_object_mut() {
        object.insert("contentHash".to_string(), json!(content_hash));
    }

    if bytes > READ_FILE_ARTIFACT_THRESHOLD {
        if let Some(ref store) = ctx.state {
            if let Some(thread_id) = ctx.thread_id {
                let tool_result = ToolResult {
                    call_id,
                    output: contents,
                    content: Vec::new(),
                    metadata: metadata.clone(),
                };
                if let Ok(Some(artifact)) = store.insert_large_tool_output_artifact(
                    thread_id,
                    &tool_result,
                    READ_FILE_ARTIFACT_THRESHOLD,
                ) {
                    if let Some(obj) = metadata.as_object_mut() {
                        obj.insert("artifactId".to_string(), json!(artifact.id));
                        obj.insert("artifactKind".to_string(), json!("file_content"));
                        obj.insert(
                            "artifact".to_string(),
                            json!({
                                "id": artifact.id,
                                "kind": "file_content",
                                "bytes": bytes
                            }),
                        );
                    }
                    output.push_str(&format!("\n\n[Artifact: {}]", artifact.id));
                }
            }
        }
    }

    Ok(ToolResult {
        call_id,
        output,
        content: Vec::new(),
        metadata,
    })
}

#[cfg(test)]
impl_typed_tool!(ReadFileTool);

#[cfg(test)]
fn verify_write_precondition(
    path: &Path,
    original: Option<&[u8]>,
    expected_hash: Option<&str>,
) -> anyhow::Result<()> {
    let Some(expected_hash) = expected_hash
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        anyhow::ensure!(
            original.is_none(),
            "write_file requires expectedHash when replacing existing file {}; reread it and retry",
            path.display()
        );
        return Ok(());
    };
    let actual = original
        .map(content_fingerprint)
        .unwrap_or_else(|| "missing".to_string());
    anyhow::ensure!(
        actual.eq_ignore_ascii_case(expected_hash),
        "write_file precondition failed for {}: expected hash {}, actual {}; reread and retry",
        path.display(),
        expected_hash,
        actual
    );
    Ok(())
}

const READ_ARTIFACT_WINDOW_CHARS: usize = 16_000;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReadArtifactInput {
    /// Artifact UUID returned by a previous tool result.
    artifact_id: String,
    /// Zero-based character offset. Omit for the first window.
    #[serde(default)]
    offset: Option<u64>,
    /// Number of characters to return, capped at 16000.
    #[serde(default)]
    limit: Option<u64>,
}

pub struct ReadArtifactTool;

#[async_trait]
impl TypedTool for ReadArtifactTool {
    type Input = ReadArtifactInput;

    fn name(&self) -> &str {
        "read_artifact"
    }

    fn description(&self) -> &str {
        "Read a bounded character window from a text artifact produced earlier in this task. Use artifactId from a tool result, then continue with nextOffset when more content remains."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        ToolExecutionPolicy::read_only(vec![tool_resource_key("artifact", &input.artifact_id)])
    }

    fn execution_intent(
        &self,
        _input: &Self::Input,
        _workspace_root: &Path,
    ) -> ToolExecutionIntent {
        ToolExecutionIntent::observation(std::iter::empty::<PathBuf>())
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let artifact_id = Uuid::parse_str(input.artifact_id.trim())
            .context("read_artifact artifactId must be a UUID")?;
        let thread_id = ctx
            .thread_id
            .context("read_artifact requires an active task")?;
        let store = ctx
            .state
            .as_ref()
            .context("read_artifact requires artifact storage")?;
        let artifact = store
            .get_artifact(thread_id, artifact_id)?
            .with_context(|| format!("artifact {artifact_id} was not found in this task"))?;
        let content = match artifact.storage {
            ArtifactStorage::Inline { content } => content,
            ArtifactStorage::Path { .. } => anyhow::bail!(
                "artifact {artifact_id} is file-backed; use its preview or the corresponding file tool"
            ),
        };
        let total_chars = content.chars().count();
        let offset = input
            .offset
            .map(usize::try_from)
            .transpose()
            .context("read_artifact offset is too large")?
            .unwrap_or(0);
        anyhow::ensure!(
            offset <= total_chars,
            "read_artifact offset {offset} exceeds total characters {total_chars}"
        );
        let limit = input.limit.map_or(READ_ARTIFACT_WINDOW_CHARS, |limit| {
            usize::try_from(limit)
                .unwrap_or(usize::MAX)
                .clamp(1, READ_ARTIFACT_WINDOW_CHARS)
        });
        let mut output = content.chars().skip(offset).take(limit).collect::<String>();
        let read_to = offset.saturating_add(output.chars().count());
        let next_offset = (read_to < total_chars).then_some(read_to);
        if let Some(next_offset) = next_offset {
            output.push_str(&format!(
                "\n\n[characters {offset}-{} of {total_chars}; call read_artifact again with artifactId {artifact_id} and offset {next_offset}]",
                read_to.saturating_sub(1)
            ));
        }
        Ok(ToolResult::text(
            call_id,
            output,
            json!({
                "toolName": "read_artifact",
                "success": true,
                "artifactId": artifact_id,
                "artifactKind": artifact.kind,
                "contentType": artifact.content_type,
                "offset": offset,
                "nextOffset": next_offset,
                "totalChars": total_chars
            }),
        ))
    }
}

impl_typed_tool!(ReadArtifactTool);

#[cfg(test)]
struct ReadFilesTool;

#[cfg(test)]
const READ_FILES_MAX_ITEMS: usize = 8;
#[cfg(test)]
const READ_FILES_TOTAL_CHARS: usize = 64_000;

#[cfg(test)]
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReadFilesInput {
    #[schemars(length(min = 1, max = 8))]
    files: Vec<FileReadInput>,
}

#[cfg(test)]
#[async_trait]
impl TypedTool for ReadFilesTool {
    type Input = ReadFilesInput;

    fn name(&self) -> &str {
        "read_files"
    }

    fn description(&self) -> &str {
        "Read up to 8 independent UTF-8 files concurrently. Each item uses the same typed window contract as read_file; the combined file-content response is capped at 64000 characters."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        let keys = input
            .files
            .iter()
            .map(|item| tool_resource_key("file", &item.path))
            .collect();
        ToolExecutionPolicy::read_only(keys)
    }

    fn execution_intent(&self, input: &Self::Input, _workspace_root: &Path) -> ToolExecutionIntent {
        ToolExecutionIntent::observation(input.files.iter().map(|item| PathBuf::from(&item.path)))
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        if input.files.is_empty() || input.files.len() > READ_FILES_MAX_ITEMS {
            anyhow::bail!("read_files accepts between 1 and {READ_FILES_MAX_ITEMS} files per call");
        }

        // Validate every authorization boundary before starting concurrent I/O.
        // This ensures one denied path suspends the whole tool call for approval
        // instead of being hidden inside an ordinary per-file error.
        for item in &input.files {
            let relative = item.path.trim();
            anyhow::ensure!(!relative.is_empty(), "each read_files item requires a path");
            let logical_path = normalize_workspace_path(&ctx.workspace_root, relative)?;
            enforce_read_policy(&ctx, &logical_path)?;
        }

        let file_count = input.files.len();
        let per_file_cap = (READ_FILES_TOTAL_CHARS / file_count).min(READ_FILE_WINDOW_CHARS);
        let mut pending = FuturesUnordered::new();
        for (index, mut item) in input.files.into_iter().enumerate() {
            if !item.is_line_window() {
                let requested = item
                    .character_limit()
                    .map(|value| value as usize)
                    .unwrap_or(per_file_cap)
                    .clamp(1, per_file_cap);
                item.set_character_limit(requested as u64);
            }
            let item_ctx = ctx.clone();
            pending.push(async move {
                let path = item.path.clone();
                let result =
                    execute_read_file_with_cap(Uuid::new_v4(), item, item_ctx, per_file_cap).await;
                (index, path, result)
            });
        }

        let mut ordered = vec![None; file_count];
        while let Some((index, path, result)) = pending.next().await {
            ordered[index] = Some(match result {
                Ok(result) => json!({
                    "path": path,
                    "ok": true,
                    "content": result.output,
                    "metadata": result.metadata
                }),
                Err(error) => json!({
                    "path": path,
                    "ok": false,
                    "error": error.to_string()
                }),
            });
        }
        let results = ordered.into_iter().flatten().collect::<Vec<_>>();
        let succeeded = results
            .iter()
            .filter(|result| result.get("ok").and_then(Value::as_bool) == Some(true))
            .count();
        let value = json!({ "files": results });
        Ok(ToolResult {
            call_id,
            output: serde_json::to_string_pretty(&value)?,
            content: Vec::new(),
            metadata: json!({
                "count": file_count,
                "succeeded": succeeded,
                "failed": file_count.saturating_sub(succeeded),
                "perFileLimit": per_file_cap,
                "success": succeeded == file_count
            }),
        })
    }
}

#[cfg(test)]
impl_typed_tool!(ReadFilesTool);

#[cfg(test)]
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WriteFileInput {
    /// File path relative to workspace.
    path: String,
    /// Full file contents to write.
    content: String,
    /// Optional content hash returned by read_file. Use `missing` when the
    /// target must not already exist.
    #[serde(default, rename = "expectedHash")]
    #[schemars(rename = "expectedHash")]
    expected_hash: Option<String>,
}

#[cfg(test)]
struct WriteFileTool;

#[cfg(test)]
#[async_trait]
impl TypedTool for WriteFileTool {
    type Input = WriteFileInput;

    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write a UTF-8 text file inside the workspace. For a file read earlier, pass its contentHash as expectedHash; use `missing` when creating a path that must not already exist. A stale precondition is rejected before writing."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        ToolExecutionPolicy {
            read_only: false,
            idempotent: true,
            parallel_safe: true,
            side_effect: ToolSideEffect::WorkspaceWrite,
            resource_keys: vec![tool_resource_key("file", &input.path)],
        }
    }

    fn execution_intent(&self, input: &Self::Input, _workspace_root: &Path) -> ToolExecutionIntent {
        ToolExecutionIntent::workspace_mutation([PathBuf::from(&input.path)])
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let relative = input.path.trim();
        anyhow::ensure!(!relative.is_empty(), "write_file requires a path");
        let path = normalize_workspace_path(&ctx.workspace_root, relative)?;
        enforce_policy_decision(ctx.policy.inspect_write(&path), &ctx)?;

        let original = read_optional(ctx.environment.as_ref(), &path).await?;
        verify_write_precondition(&path, original.as_deref(), input.expected_hash.as_deref())?;
        let contents = input.content.into_bytes();
        let bytes_written = contents.len();
        let batch = FileMutationBatch::new(vec![PreparedFileMutation::write(
            path.clone(),
            original,
            contents,
        )])?;
        ctx.commit_file_mutations(&batch).await?;
        Ok(ToolResult {
            call_id,
            output: format!("Wrote {} bytes to {}", bytes_written, path.display()),
            content: Vec::new(),
            metadata: json!({
                "changedPath": path.display().to_string(),
                "bytes": bytes_written
            }),
        })
    }
}

#[cfg(test)]
impl_typed_tool!(WriteFileTool);

mod workspace_search_tool;
pub use workspace_search_tool::WorkspaceSearchTool;
#[cfg(test)]
use workspace_search_tool::{find_literal_match, run_fallback_search};

mod shell_tool;
use shell_tool::{
    background_scope, effective_foreground_yield_milliseconds, DEFAULT_BACKGROUND_TIMEOUT_SECONDS,
    DEFAULT_FOREGROUND_YIELD_MILLISECONDS, MAX_BACKGROUND_TIMEOUT_SECONDS,
};
#[cfg(test)]
use shell_tool::{shell_execution_intent, BackgroundOutputInput};
pub use shell_tool::{BackgroundOutputTool, ShellTool};

#[cfg(test)]
struct GitDiffTool;

#[cfg(test)]
#[async_trait]
impl TypedTool for GitDiffTool {
    type Input = EmptyToolInput;

    fn name(&self) -> &str {
        "git_diff"
    }

    fn description(&self) -> &str {
        "Show the current git diff for the workspace."
    }

    fn execution_policy(&self, _input: &Self::Input) -> ToolExecutionPolicy {
        ToolExecutionPolicy::read_only(vec!["git:index-and-worktree".to_string()])
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        _input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let output = ctx
            .environment
            .exec(
                ExecRequest::new("git").args(["diff", "--no-ext-diff", "--no-color", "--"]),
                ctx.execution_context(Duration::from_secs(20)),
            )
            .await
            .map_err(|error| anyhow::anyhow!("git diff execution failed: {error:#}"))?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let text = if stdout.trim().is_empty() {
            format!(
                "[stdout]\n(no diff)\n\n[stderr]\n{}",
                truncate(&stderr, 8_000)
            )
        } else {
            truncate(&stdout, 32_000)
        };
        Ok(ToolResult {
            call_id,
            output: text,
            content: Vec::new(),
            metadata: json!({
                "exitCode": output.exit_code,
                "success": output.success,
                "sandbox": output.sandbox
            }),
        })
    }
}

#[cfg(test)]
impl_typed_tool!(GitDiffTool);

mod apply_patch_tool;
#[cfg(test)]
use apply_patch_tool::{apply_text_patch, execute_portable_patch};
pub use apply_patch_tool::{
    execute_native_patch_operation, native_patch_operation_to_unified_diff, ApplyPatchTool,
    NativePatchOperation,
};

fn normalize_workspace_path(workspace_root: &Path, path: &str) -> anyhow::Result<PathBuf> {
    let candidate = PathBuf::from(path);
    if candidate
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        anyhow::bail!(
            "workspace path cannot contain '..': {}",
            candidate.display()
        );
    }
    if candidate.is_absolute() {
        Ok(candidate)
    } else {
        Ok(workspace_root.join(candidate))
    }
}

fn tool_resource_key(kind: &str, path: &str) -> String {
    let normalized = path.trim().replace('\\', "/");
    format!(
        "{kind}:{}",
        if normalized.is_empty() {
            "*"
        } else {
            normalized.as_str()
        }
    )
}

fn enforce_read_policy(ctx: &ToolInvocationContext, path: &Path) -> anyhow::Result<()> {
    enforce_policy_decision(ctx.policy.inspect_read(path), ctx)
}

fn looks_like_sandbox_denial(stderr: &str) -> bool {
    let stderr = stderr.to_ascii_lowercase();
    [
        "access is denied",
        "access denied",
        "access to the path",
        "permissiondenied",
        "permission denied",
        "operation not permitted",
        "read-only file system",
        "unauthorized",
        "unauthorizedaccessexception",
        "network is unreachable",
        "network access is denied",
        "blocked by sandbox",
    ]
    .iter()
    .any(|pattern| stderr.contains(pattern))
}

#[cfg(test)]
fn truncate_bytes(value: &str, max_bytes: usize) -> (String, bool) {
    if value.len() <= max_bytes {
        return (value.to_string(), false);
    }

    let head_target = max_bytes / 2;
    let tail_target = max_bytes.saturating_sub(head_target);
    let mut head_end = head_target;
    while head_end > 0 && !value.is_char_boundary(head_end) {
        head_end -= 1;
    }
    let mut tail_start = value.len().saturating_sub(tail_target);
    while tail_start < value.len() && !value.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    let omitted = tail_start.saturating_sub(head_end);
    let mut truncated = value[..head_end].to_string();
    truncated.push_str(&format!("\n\n[{omitted} bytes omitted]\n\n"));
    truncated.push_str(&value[tail_start..]);
    (truncated, true)
}

#[cfg(test)]
fn list_dir_entries(path: &Path) -> anyhow::Result<Vec<String>> {
    let mut entries = Vec::new();
    for entry in
        std::fs::read_dir(path).with_context(|| format!("failed to list {}", path.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let marker = if file_type.is_dir() { "/" } else { "" };
        entries.push(format!("{}{}", entry.file_name().to_string_lossy(), marker));
    }
    entries.sort();
    Ok(entries)
}

fn truncate(value: &str, max_chars: usize) -> String {
    let total_chars = value.chars().count();
    if total_chars <= max_chars {
        return value.to_string();
    }

    let head_chars = max_chars / 2;
    let tail_chars = max_chars.saturating_sub(head_chars);
    let mut truncated: String = value.chars().take(head_chars).collect();
    truncated.push_str(&format!(
        "\n\n[{} characters omitted]\n\n",
        total_chars.saturating_sub(max_chars)
    ));
    truncated.extend(value.chars().skip(total_chars.saturating_sub(tail_chars)));
    truncated
}

mod mcp_tool;
#[cfg(test)]
use mcp_tool::decode_mcp_base64;
use mcp_tool::mcp_content_parts;
pub use mcp_tool::McpToolWrapper;

#[cfg(test)]
mod tests;
