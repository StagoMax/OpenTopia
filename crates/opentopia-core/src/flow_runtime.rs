use crate::enterprise::{CapabilityProjection, ENTERPRISE_SCHEMA_VERSION_V1};
use crate::flow::{
    compile_flow, FlowBudgetV1, FlowDefinitionV1, GraphDefinitionV1, GraphEdgeV1,
    GraphLoopPolicyV1, GraphNodeKindV1, GraphNodeV1, LoopExhaustionActionV1,
};
use crate::human_task::{HumanTaskActionV1, HumanTaskV1};
use crate::model::UserInputResponse;
use crate::tools::ToolInvocationContext;
use crate::workflow_interrupt::{
    FlowNodeInterruptV1, FlowResumeCommandV1, WorkflowInterruptKindV1, WorkflowInterruptRequestV1,
};
use crate::workflow_state::{apply_state_writes, parse_state_writes};
use crate::{
    CompiledWorkflowV1, DeploymentSnapshotV1, RuntimeConnectionAuthorityV1, WorkflowAgentSpecV1,
    WorkflowDeploymentStatusV1, WorkflowDeploymentV1, WorkflowOutputReviewPolicyV1,
    WorkflowOutputSpecV1, WorkflowTriggerSpecV1,
};
use anyhow::Context;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures_util::stream::{self, StreamExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::time::Duration;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlowRunStatusV1 {
    Queued,
    Running,
    PauseRequested,
    Paused,
    WaitingApproval,
    WaitingHuman,
    Resuming,
    Succeeded,
    Failed,
    CancelRequested,
    Cancelled,
}

impl FlowRunStatusV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::PauseRequested => "pause_requested",
            Self::Paused => "paused",
            Self::WaitingApproval => "waiting_approval",
            // The indexed v19 compatibility column groups all human waits and
            // active execution states. The precise state remains in document_json.
            Self::WaitingHuman => "waiting_approval",
            Self::Resuming => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::CancelRequested => "cancel_requested",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlowNodeRunStatusV1 {
    Running,
    WaitingApproval,
    WaitingHuman,
    Resuming,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlowTranscriptEntryKindV1 {
    Input,
    ToolCall,
    ToolResult,
    Output,
    Approval,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowTranscriptEntryV1 {
    pub id: Uuid,
    pub kind: FlowTranscriptEntryKindV1,
    pub title: String,
    pub content: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<Uuid>,
    #[serde(default)]
    pub is_error: bool,
    pub created_at: DateTime<Utc>,
}

impl FlowTranscriptEntryV1 {
    pub fn new(kind: FlowTranscriptEntryKindV1, title: impl Into<String>, content: Value) -> Self {
        Self {
            id: Uuid::new_v4(),
            kind,
            title: title.into(),
            content,
            tool_name: None,
            call_id: None,
            is_error: false,
            created_at: Utc::now(),
        }
    }

    pub fn tool(
        kind: FlowTranscriptEntryKindV1,
        tool_name: impl Into<String>,
        call_id: Uuid,
        content: Value,
        is_error: bool,
    ) -> Self {
        let tool_name = tool_name.into();
        Self {
            id: Uuid::new_v4(),
            kind,
            title: tool_name.clone(),
            content,
            tool_name: Some(tool_name),
            call_id: Some(call_id),
            is_error,
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowNodeRunV1 {
    pub id: Uuid,
    pub node_id: String,
    pub attempt: u32,
    pub status: FlowNodeRunStatusV1,
    pub input: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub tool_calls: u32,
    #[serde(default)]
    pub transcript: Vec<FlowTranscriptEntryV1>,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowCheckpointStatusV1 {
    Running,
    Committed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowSuperstepNodeV1 {
    pub node_id: String,
    pub node_run_id: Uuid,
    pub attempt: u32,
    pub input: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowPendingWriteV1 {
    pub node_id: String,
    pub node_run_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<FlowNodeExecutionResultV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interrupt: Option<WorkflowInterruptRequestV1>,
    /// The command is retained after execution as an immutable audit record.
    /// It only applies when its interrupt id/revision matches `interrupt`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_command: Option<FlowResumeCommandV1>,
    pub completed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowCheckpointV1 {
    pub id: Uuid,
    pub superstep: u32,
    pub status: WorkflowCheckpointStatusV1,
    pub nodes: Vec<WorkflowSuperstepNodeV1>,
    #[serde(default)]
    pub pending_writes: Vec<WorkflowPendingWriteV1>,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowCheckpointSummaryV1 {
    pub id: Uuid,
    pub superstep: u32,
    pub status: WorkflowCheckpointStatusV1,
    pub node_ids: Vec<String>,
    pub pending_write_count: u32,
    pub created_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowRunV1 {
    pub schema_version: u16,
    pub id: Uuid,
    pub thread_id: Uuid,
    pub flow_id: String,
    pub flow_version: u32,
    pub definition_id: Uuid,
    pub definition_content_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment_snapshot: Option<DeploymentSnapshotV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_draft_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_draft_revision: Option<u32>,
    pub revision: u32,
    pub status: FlowRunStatusV1,
    pub input: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    pub graph: GraphDefinitionV1,
    pub effective_capabilities: CapabilityProjection,
    /// Immutable operation-level authority captured when the Run starts.
    /// `None` is reserved for persisted runs created before this field existed;
    /// those are restored through explicit legacy-projection inference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_authority: Option<RuntimeConnectionAuthorityV1>,
    pub budget: FlowBudgetV1,
    #[serde(default)]
    pub ready_nodes: Vec<String>,
    #[serde(default)]
    pub node_runs: Vec<FlowNodeRunV1>,
    #[serde(default)]
    pub node_outputs: BTreeMap<String, Value>,
    /// Reducer-owned shared state. Node outputs remain the immutable routing
    /// source; state channels are applied only at a committed superstep.
    #[serde(default)]
    pub state: BTreeMap<String, Value>,
    #[serde(default)]
    pub superstep: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_checkpoint: Option<WorkflowCheckpointV1>,
    #[serde(default)]
    pub checkpoint_history: Vec<WorkflowCheckpointSummaryV1>,
    #[serde(default)]
    pub loop_counts: BTreeMap<String, u32>,
    pub node_executions: u32,
    pub tool_calls: u32,
    /// Production deployments pause after terminal output until a HumanTask
    /// records the review decision. Trial/manual compatibility runs may opt out.
    #[serde(default)]
    pub output_review_required: bool,
    #[serde(default)]
    pub output_reviewed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_human_task_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

impl FlowRunV1 {
    pub fn new(
        thread_id: Uuid,
        definition: &FlowDefinitionV1,
        input: Value,
        available_capabilities: &CapabilityProjection,
    ) -> anyhow::Result<Self> {
        let connection_authority =
            RuntimeConnectionAuthorityV1::inferred_from_projection(available_capabilities);
        Self::new_with_connection_authority(
            thread_id,
            definition,
            input,
            available_capabilities,
            connection_authority,
        )
    }

    pub fn new_with_connection_authority(
        thread_id: Uuid,
        definition: &FlowDefinitionV1,
        input: Value,
        available_capabilities: &CapabilityProjection,
        connection_authority: RuntimeConnectionAuthorityV1,
    ) -> anyhow::Result<Self> {
        let effective_capabilities = definition.capabilities.intersect(available_capabilities);
        anyhow::ensure!(
            definition.capabilities.is_subset_of(available_capabilities),
            "the current ExecutionContext is narrower than the published Flow capability snapshot"
        );
        let spec = definition.to_spec();
        compile_flow(&spec, &effective_capabilities).map_err(|report| {
            anyhow::anyhow!(
                "published Flow no longer compiles in this ExecutionContext: {} validation error(s)",
                report.error_count()
            )
        })?;
        let now = Utc::now();
        Ok(Self {
            schema_version: ENTERPRISE_SCHEMA_VERSION_V1,
            id: Uuid::new_v4(),
            thread_id,
            flow_id: definition.flow_id.clone(),
            flow_version: definition.version,
            definition_id: definition.id,
            definition_content_hash: definition.content_hash.clone(),
            deployment_id: None,
            deployment_snapshot: None,
            test_draft_id: None,
            test_draft_revision: None,
            revision: 1,
            status: FlowRunStatusV1::Queued,
            input,
            output: None,
            graph: definition.graph.clone(),
            connection_authority: Some(connection_authority.attenuate(&effective_capabilities)),
            effective_capabilities,
            budget: definition.budget.clone(),
            ready_nodes: vec![definition.graph.entry_node_id.clone()],
            node_runs: Vec::new(),
            node_outputs: BTreeMap::new(),
            state: BTreeMap::new(),
            superstep: 0,
            active_checkpoint: None,
            checkpoint_history: Vec::new(),
            loop_counts: BTreeMap::new(),
            node_executions: 0,
            tool_calls: 0,
            output_review_required: false,
            output_reviewed: false,
            waiting_node_id: None,
            active_human_task_id: None,
            error: None,
            created_at: now,
            started_at: None,
            completed_at: None,
            updated_at: now,
        })
    }

    pub fn new_from_deployment(
        thread_id: Uuid,
        deployment: &WorkflowDeploymentV1,
        input: Value,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            deployment.status == WorkflowDeploymentStatusV1::Active,
            "Workflow deployment is not active"
        );
        let workflow = &deployment.snapshot.compiled_workflow;
        anyhow::ensure!(
            workflow.schema_version == ENTERPRISE_SCHEMA_VERSION_V1
                && deployment.snapshot.schema_version == ENTERPRISE_SCHEMA_VERSION_V1,
            "Workflow deployment snapshot uses an unsupported schema version"
        );
        let now = Utc::now();
        Ok(Self {
            schema_version: ENTERPRISE_SCHEMA_VERSION_V1,
            id: Uuid::new_v4(),
            thread_id,
            flow_id: workflow.flow_id.clone(),
            flow_version: workflow.flow_version,
            definition_id: workflow.definition_id,
            definition_content_hash: workflow.definition_content_hash.clone(),
            deployment_id: Some(deployment.id),
            deployment_snapshot: Some(deployment.snapshot.clone()),
            test_draft_id: None,
            test_draft_revision: None,
            revision: 1,
            status: FlowRunStatusV1::Queued,
            input,
            output: None,
            graph: workflow.graph.clone(),
            // Production Agent-node authority is owned by the compiled
            // workflow. The root graph itself has no implicit account access.
            connection_authority: Some(RuntimeConnectionAuthorityV1::DenyAll),
            effective_capabilities: workflow.root_capabilities.clone(),
            budget: workflow.budget.clone(),
            ready_nodes: vec![workflow.graph.entry_node_id.clone()],
            node_runs: Vec::new(),
            node_outputs: BTreeMap::new(),
            state: BTreeMap::new(),
            superstep: 0,
            active_checkpoint: None,
            checkpoint_history: Vec::new(),
            loop_counts: BTreeMap::new(),
            node_executions: 0,
            tool_calls: 0,
            output_review_required: deployment.snapshot.output_review_policy
                == WorkflowOutputReviewPolicyV1::AlwaysReviewOutput,
            output_reviewed: false,
            waiting_node_id: None,
            active_human_task_id: None,
            error: None,
            created_at: now,
            started_at: None,
            completed_at: None,
            updated_at: now,
        })
    }

    pub fn new_for_test_run(
        thread_id: Uuid,
        draft_id: Uuid,
        draft_revision: u32,
        compiled_workflow: CompiledWorkflowV1,
        input: Value,
    ) -> anyhow::Result<Self> {
        let deployment = WorkflowDeploymentV1::new_with_options(
            "Workflow test run",
            "test",
            compiled_workflow,
            WorkflowTriggerSpecV1::Manual,
            WorkflowOutputSpecV1::Inbox,
            WorkflowOutputReviewPolicyV1::ExplicitNodesOnly,
            "workflow-test-runner",
        )?;
        let mut run = Self::new_from_deployment(thread_id, &deployment, input)?;
        run.deployment_id = None;
        run.test_draft_id = Some(draft_id);
        run.test_draft_revision = Some(draft_revision);
        run.definition_id = draft_id;
        run.output_review_required = false;
        Ok(run)
    }

    pub fn effective_connection_authority(&self) -> RuntimeConnectionAuthorityV1 {
        self.connection_authority.clone().unwrap_or_else(|| {
            RuntimeConnectionAuthorityV1::inferred_from_projection(&self.effective_capabilities)
        })
    }

    pub fn harness_capabilities(&self) -> CapabilityProjection {
        self.deployment_snapshot
            .as_ref()
            .map(|snapshot| snapshot.compiled_workflow.harness_capabilities.clone())
            .unwrap_or_else(|| self.effective_capabilities.clone())
    }

    pub fn harness_connection_authority(&self) -> RuntimeConnectionAuthorityV1 {
        self.deployment_snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .compiled_workflow
                    .harness_connection_authority
                    .clone()
            })
            .unwrap_or_else(|| self.effective_connection_authority())
    }

    pub fn workflow_agent_spec(&self, node_id: &str) -> Option<&WorkflowAgentSpecV1> {
        self.deployment_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.compiled_workflow.agent_spec(node_id))
    }

    pub fn workflow_agent_specs(&self) -> Vec<WorkflowAgentSpecV1> {
        self.deployment_snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .compiled_workflow
                    .agent_specs
                    .values()
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn effective_capabilities_for_node(&self, node_id: &str) -> CapabilityProjection {
        self.workflow_agent_spec(node_id)
            .map(|spec| spec.capabilities.clone())
            .unwrap_or_else(|| self.effective_capabilities.clone())
    }

    pub fn effective_connection_authority_for_node(
        &self,
        node_id: &str,
    ) -> RuntimeConnectionAuthorityV1 {
        self.workflow_agent_spec(node_id)
            .map(|spec| spec.connection_authority.clone())
            .unwrap_or_else(|| self.effective_connection_authority())
    }

    pub fn active_node_run_mut(&mut self, node_id: &str) -> Option<&mut FlowNodeRunV1> {
        self.node_runs
            .iter_mut()
            .rev()
            .find(|node| node.node_id == node_id && node.completed_at.is_none())
    }

    pub fn pending_resume_command_id(&self) -> Option<Uuid> {
        self.active_checkpoint
            .as_ref()?
            .pending_writes
            .iter()
            .find_map(|write| {
                let interrupt = write.interrupt.as_ref()?;
                let command = write.resume_command.as_ref()?;
                command.validates(interrupt).then_some(command.id)
            })
    }

    pub fn next_attempt(&self, node_id: &str) -> u32 {
        self.node_runs
            .iter()
            .filter(|run| run.node_id == node_id)
            .count()
            .saturating_add(1) as u32
    }

    pub fn touch(&mut self) {
        self.revision = self.revision.saturating_add(1);
        self.updated_at = Utc::now();
    }

    pub fn fail(&mut self, error: impl Into<String>) {
        self.status = FlowRunStatusV1::Failed;
        self.error = Some(error.into());
        self.completed_at = Some(Utc::now());
        self.touch();
    }
}

#[derive(Clone)]
pub struct FlowNodeExecutionRequestV1 {
    pub flow_run_id: Uuid,
    pub node_run_id: Uuid,
    pub node: GraphNodeV1,
    pub input: Value,
    pub effective_capabilities: CapabilityProjection,
    pub workflow_agent_spec: Option<WorkflowAgentSpecV1>,
    pub remaining_tool_calls: u32,
    pub context: ToolInvocationContext,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowNodeExecutionResultV1 {
    pub output: Value,
    pub tool_calls: u32,
    #[serde(default)]
    pub transcript: Vec<FlowTranscriptEntryV1>,
}

#[derive(Debug, Clone)]
pub enum FlowNodeExecutionOutcomeV1 {
    Completed(FlowNodeExecutionResultV1),
    Interrupted(FlowNodeInterruptV1),
}

#[derive(Clone)]
pub struct FlowNodeResumeRequestV1 {
    pub flow_run_id: Uuid,
    pub node_run_id: Uuid,
    pub node: GraphNodeV1,
    pub input: Value,
    pub effective_capabilities: CapabilityProjection,
    pub workflow_agent_spec: Option<WorkflowAgentSpecV1>,
    pub remaining_tool_calls: u32,
    pub interrupt: WorkflowInterruptRequestV1,
    pub command: FlowResumeCommandV1,
    pub context: ToolInvocationContext,
}

#[async_trait]
pub trait FlowNodeHarness: Send + Sync {
    async fn execute_flow_node(
        &self,
        request: FlowNodeExecutionRequestV1,
    ) -> anyhow::Result<FlowNodeExecutionOutcomeV1>;

    async fn resume_flow_node(
        &self,
        _request: FlowNodeResumeRequestV1,
    ) -> anyhow::Result<FlowNodeExecutionOutcomeV1> {
        anyhow::bail!("this Flow Agent Harness does not support resumable node continuations")
    }
}

fn restrict_flow_node_connection_context(
    context: &mut ToolInvocationContext,
    authority: &RuntimeConnectionAuthorityV1,
) -> anyhow::Result<()> {
    match authority {
        RuntimeConnectionAuthorityV1::DenyAll => {
            context.mcp_tools.clear();
            context.connection_operations.clear();
        }
        RuntimeConnectionAuthorityV1::LegacyMcp => {}
        RuntimeConnectionAuthorityV1::Structured { operations } => {
            let expected = operations
                .iter()
                .map(|operation| (operation.model_tool_name.as_str(), operation))
                .collect::<BTreeMap<_, _>>();
            for (name, operation) in &expected {
                let route = context.connection_operations.get(*name).with_context(|| {
                    format!("Workflow Agent node Connection route {name} is unavailable")
                })?;
                anyhow::ensure!(
                    route.operation() == *operation,
                    "Workflow Agent node Connection route {name} differs from its DeploymentSnapshot"
                );
            }
            context
                .mcp_tools
                .retain(|descriptor| expected.contains_key(descriptor.public_name.as_str()));
            context.connection_operations.retain(|name, route| {
                expected
                    .get(name.as_str())
                    .is_some_and(|operation| route.operation() == *operation)
            });
        }
    }
    Ok(())
}

pub fn spawn_flow_run(run_id: Uuid, context: ToolInvocationContext) -> anyhow::Result<()> {
    anyhow::ensure!(
        context.state.is_some(),
        "Flow Runtime requires a persistent SessionStore"
    );
    anyhow::ensure!(
        context.flow_harness.is_some(),
        "Flow Runtime requires the Agent Harness"
    );
    tokio::spawn(async move {
        if let Err(error) = drive_flow_run(run_id, context.clone()).await {
            let Some(state) = context.state.as_ref() else {
                return;
            };
            let store = state.flow_session_store();
            let Ok(Some(mut run)) = store.get_flow_run(run_id) else {
                return;
            };
            if !run.status.is_terminal() {
                let expected = run.revision;
                run.fail(error.to_string());
                let _ = store.update_flow_run(&run, expected);
            }
        }
    });
    Ok(())
}

pub fn resolve_flow_approval(
    run: &mut FlowRunV1,
    approved: bool,
    note: Option<&str>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        run.status == FlowRunStatusV1::WaitingApproval,
        "Flow run is not waiting for approval"
    );
    let node_id = run
        .waiting_node_id
        .clone()
        .context("Flow run is missing its waiting approval node")?;
    let output = json!({
        "approved": approved,
        "note": note.unwrap_or_default(),
    });
    let node_run = run
        .active_node_run_mut(&node_id)
        .context("Flow approval node run is missing")?;
    node_run.status = if approved {
        FlowNodeRunStatusV1::Succeeded
    } else {
        FlowNodeRunStatusV1::Cancelled
    };
    node_run.output = Some(output.clone());
    node_run.transcript.push(FlowTranscriptEntryV1::new(
        FlowTranscriptEntryKindV1::Approval,
        if approved {
            "Approval granted"
        } else {
            "Approval rejected"
        },
        output.clone(),
    ));
    node_run.completed_at = Some(Utc::now());
    run.waiting_node_id = None;
    run.active_human_task_id = None;
    if approved {
        run.node_outputs.insert(node_id.clone(), output.clone());
        run.status = FlowRunStatusV1::Running;
        run.error = None;
        route_after_node(run, &node_id, &output)?;
    } else {
        run.status = FlowRunStatusV1::Cancelled;
        run.error = Some(note.unwrap_or("approval denied").to_string());
        run.completed_at = Some(Utc::now());
    }
    run.touch();
    Ok(())
}

pub fn prepare_flow_interrupt_resume(
    run: &mut FlowRunV1,
    task: &HumanTaskV1,
    action: HumanTaskActionV1,
    response: Option<Value>,
    note: Option<&str>,
    actor: &str,
    idempotency_key: &str,
) -> anyhow::Result<FlowResumeCommandV1> {
    anyhow::ensure!(
        run.status == FlowRunStatusV1::WaitingHuman,
        "Flow run is not waiting on an Agent continuation"
    );
    anyhow::ensure!(
        run.active_human_task_id == Some(task.id),
        "Flow run is no longer waiting on this Human task"
    );
    let continuation_id = task
        .continuation_id
        .context("Human task is missing its Agent continuation identity")?;
    let write = run
        .active_checkpoint
        .as_mut()
        .context("waiting Flow run is missing its checkpoint")?
        .pending_writes
        .iter_mut()
        .find(|write| {
            write
                .interrupt
                .as_ref()
                .is_some_and(|interrupt| interrupt.continuation.id == continuation_id)
        })
        .context("Human task continuation is no longer present in the checkpoint")?;
    let interrupt = write
        .interrupt
        .as_ref()
        .context("checkpoint write is missing its interrupt")?;
    let signal = match interrupt.kind {
        WorkflowInterruptKindV1::Approval => {
            anyhow::ensure!(
                matches!(
                    action,
                    HumanTaskActionV1::Approve | HumanTaskActionV1::Reject
                ),
                "approval interrupt only accepts approve or reject"
            );
            crate::workflow_interrupt::FlowResumeSignalV1::Approval {
                approval_id: interrupt
                    .payload
                    .get("approvalId")
                    .and_then(Value::as_str)
                    .and_then(|value| Uuid::parse_str(value).ok()),
                approved: action == HumanTaskActionV1::Approve,
            }
        }
        WorkflowInterruptKindV1::InputRequest => {
            anyhow::ensure!(
                action == HumanTaskActionV1::Submit,
                "input interrupt only accepts submit"
            );
            let response = response.context("input interrupt requires a structured response")?;
            let response = serde_json::from_value::<UserInputResponse>(response)?;
            let request_id = interrupt
                .payload
                .get("request")
                .and_then(|request| request.get("requestId"))
                .and_then(Value::as_str)
                .and_then(|value| Uuid::parse_str(value).ok())
                .context("input interrupt is missing requestId")?;
            crate::workflow_interrupt::FlowResumeSignalV1::UserInput {
                request_id,
                response,
            }
        }
        WorkflowInterruptKindV1::ExternalAction | WorkflowInterruptKindV1::EffectReconciliation => {
            anyhow::ensure!(
                matches!(
                    action,
                    HumanTaskActionV1::Resume
                        | HumanTaskActionV1::Reconnect
                        | HumanTaskActionV1::Acknowledge
                ),
                "external interrupt requires resume, reconnect, or acknowledge"
            );
            let observation = response
                .as_ref()
                .and_then(|value| value.get("observation"))
                .and_then(Value::as_str)
                .or(note)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .context("external action requires an observation")?
                .to_string();
            crate::workflow_interrupt::FlowResumeSignalV1::ExternalAction { observation }
        }
        WorkflowInterruptKindV1::ResumeRetry => {
            anyhow::ensure!(
                action == HumanTaskActionV1::Retry,
                "resume retry interrupt only accepts retry"
            );
            write
                .resume_command
                .as_ref()
                .context("resume retry interrupt is missing its previous command")?
                .signal
                .clone()
        }
    };
    let command =
        FlowResumeCommandV1::new(task.id, idempotency_key, interrupt, signal, note, actor)?;
    write.resume_command = Some(command.clone());
    if let Some(node_run) = run
        .node_runs
        .iter_mut()
        .find(|node| node.id == interrupt.node_run_id)
    {
        node_run.status = FlowNodeRunStatusV1::Resuming;
    }
    run.status = FlowRunStatusV1::Resuming;
    run.waiting_node_id = None;
    run.active_human_task_id = None;
    run.error = None;
    run.touch();
    Ok(command)
}

pub fn prepare_flow_resume(
    run: &mut FlowRunV1,
    retry_interrupted_node: bool,
) -> anyhow::Result<()> {
    if let Some(mut checkpoint) = run.active_checkpoint.take() {
        anyhow::ensure!(
            retry_interrupted_node,
            "superstep {} was interrupted; inspect external state, then explicitly set retryInterruptedNode or cancel the Run",
            checkpoint.superstep
        );
        let completed = checkpoint
            .pending_writes
            .iter()
            .filter(|write| write.result.is_some())
            .map(|write| write.node_id.clone())
            .collect::<std::collections::BTreeSet<_>>();
        let retry_count = checkpoint
            .nodes
            .iter()
            .filter(|item| !completed.contains(&item.node_id))
            .count() as u32;
        anyhow::ensure!(
            run.node_executions.saturating_add(retry_count) <= run.budget.max_node_executions,
            "Flow node execution budget exhausted ({})",
            run.budget.max_node_executions
        );
        checkpoint
            .pending_writes
            .retain(|write| completed.contains(&write.node_id));
        for item in &mut checkpoint.nodes {
            if completed.contains(&item.node_id) {
                continue;
            }
            if let Some(node_run) = run
                .node_runs
                .iter_mut()
                .find(|candidate| candidate.id == item.node_run_id)
            {
                node_run.status = FlowNodeRunStatusV1::Cancelled;
                node_run.error = Some(
                    "process stopped during superstep execution; operator explicitly requested a fresh attempt"
                        .to_string(),
                );
                node_run.completed_at = Some(Utc::now());
            }
            let node_run_id = Uuid::new_v4();
            item.node_run_id = node_run_id;
            item.attempt = item.attempt.saturating_add(1);
            run.node_runs.push(FlowNodeRunV1 {
                id: node_run_id,
                node_id: item.node_id.clone(),
                attempt: item.attempt,
                status: FlowNodeRunStatusV1::Running,
                input: item.input.clone(),
                output: None,
                error: None,
                tool_calls: 0,
                transcript: vec![FlowTranscriptEntryV1::new(
                    FlowTranscriptEntryKindV1::Input,
                    "Node input",
                    item.input.clone(),
                )],
                started_at: Utc::now(),
                completed_at: None,
            });
            run.node_executions = run.node_executions.saturating_add(1);
        }
        run.active_checkpoint = Some(checkpoint);
        run.active_human_task_id = None;
        return Ok(());
    }
    let Some(index) = run
        .node_runs
        .iter()
        .rposition(|node| node.status == FlowNodeRunStatusV1::Running)
    else {
        return Ok(());
    };
    anyhow::ensure!(
        retry_interrupted_node,
        "the previous process stopped during node {}; inspect external state, then explicitly set retryInterruptedNode or cancel the Run",
        run.node_runs[index].node_id
    );
    let node_id = run.node_runs[index].node_id.clone();
    run.node_runs[index].status = FlowNodeRunStatusV1::Cancelled;
    run.node_runs[index].error = Some(
        "process stopped during execution; operator explicitly requested a fresh attempt"
            .to_string(),
    );
    run.node_runs[index].completed_at = Some(Utc::now());
    remove_first(&mut run.ready_nodes, &node_id);
    run.ready_nodes.insert(0, node_id);
    run.active_human_task_id = None;
    Ok(())
}

async fn drive_flow_run(run_id: Uuid, context: ToolInvocationContext) -> anyhow::Result<()> {
    let store = context
        .state
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Flow Runtime requires a persistent SessionStore"))?;
    let store = store.flow_session_store();
    let harness = context
        .flow_harness
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Flow Runtime requires the Agent Harness"))?
        .clone();

    loop {
        let mut run = store
            .get_flow_run(run_id)?
            .ok_or_else(|| anyhow::anyhow!("Flow run not found: {run_id}"))?;
        if run.status.is_terminal()
            || matches!(
                run.status,
                FlowRunStatusV1::WaitingApproval | FlowRunStatusV1::WaitingHuman
            )
        {
            return Ok(());
        }
        if run.status == FlowRunStatusV1::CancelRequested {
            let expected = run.revision;
            cancel_active_checkpoint(&mut run);
            run.status = FlowRunStatusV1::Cancelled;
            run.completed_at = Some(Utc::now());
            run.touch();
            store.update_flow_run(&run, expected)?;
            return Ok(());
        }
        if run.status == FlowRunStatusV1::PauseRequested && run.active_checkpoint.is_none() {
            let expected = run.revision;
            run.status = FlowRunStatusV1::Paused;
            run.touch();
            store.update_flow_run(&run, expected)?;
            return Ok(());
        }
        if run.status == FlowRunStatusV1::Paused {
            return Ok(());
        }

        enforce_run_budget(&run)?;
        if let Some(checkpoint) = run.active_checkpoint.clone() {
            if run.status == FlowRunStatusV1::Resuming {
                let resumable = checkpoint.pending_writes.iter().find_map(|write| {
                    let interrupt = write.interrupt.as_ref()?;
                    let command = write.resume_command.as_ref()?;
                    command
                        .validates(interrupt)
                        .then(|| (write.clone(), interrupt.clone(), command.clone()))
                });
                let (write, interrupt, command) = resumable
                    .context("Flow run is resuming without a matching persisted ResumeCommand")?;
                let replacement = execute_resume_checkpoint_node(
                    run.clone(),
                    write,
                    interrupt,
                    command,
                    context.clone(),
                    harness.clone(),
                )
                .await;
                replace_pending_write(store.as_ref(), run_id, checkpoint.id, replacement)?;
                continue;
            }
            let completed = checkpoint
                .pending_writes
                .iter()
                .map(|write| write.node_id.as_str())
                .collect::<std::collections::BTreeSet<_>>();
            let pending = checkpoint
                .nodes
                .iter()
                .filter(|item| !completed.contains(item.node_id.as_str()))
                .cloned()
                .collect::<Vec<_>>();

            let mut parallel = Vec::new();
            let mut serial = Vec::new();
            for item in pending {
                let node = run
                    .graph
                    .nodes
                    .iter()
                    .find(|node| node.id == item.node_id)
                    .context("Flow checkpoint references a missing node")?;
                if node_is_parallel_safe(node) {
                    parallel.push(item);
                } else {
                    serial.push(item);
                }
            }

            let results = stream::iter(parallel.into_iter().map(|item| {
                execute_checkpoint_node(run.clone(), item, context.clone(), harness.clone())
            }))
            .buffer_unordered(MAX_SUPERSTEP_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;
            for write in results {
                persist_pending_write(store.as_ref(), run_id, checkpoint.id, write)?;
            }
            for item in serial {
                let write =
                    execute_checkpoint_node(run.clone(), item, context.clone(), harness.clone())
                        .await;
                persist_pending_write(store.as_ref(), run_id, checkpoint.id, write)?;
            }
            commit_active_checkpoint(store.as_ref(), run_id, checkpoint.id)?;
            continue;
        }

        let ready_nodes = ready_superstep_nodes(&run);
        if ready_nodes.is_empty() {
            let expected = run.revision;
            if run.ready_nodes.is_empty() {
                run.status = FlowRunStatusV1::Succeeded;
                run.output = terminal_output(&run);
                run.completed_at = Some(Utc::now());
            } else {
                run.status = FlowRunStatusV1::Failed;
                run.error = Some(format!(
                    "Flow graph is deadlocked; waiting nodes: {}",
                    run.ready_nodes.join(", ")
                ));
                run.completed_at = Some(Utc::now());
            }
            run.touch();
            store.update_flow_run(&run, expected)?;
            return Ok(());
        }
        let approval_node = ready_nodes.iter().find_map(|ready_node_id| {
            run.graph
                .nodes
                .iter()
                .find(|node| node.id == *ready_node_id && node.kind == GraphNodeKindV1::Approval)
                .cloned()
        });
        if let Some(node) = approval_node {
            pause_for_approval(store.as_ref(), &mut run, &node)?;
            return Ok(());
        }
        begin_superstep(store.as_ref(), &mut run, &ready_nodes)?;
    }
}

const MAX_SUPERSTEP_CONCURRENCY: usize = 8;
const MAX_CHECKPOINT_HISTORY: usize = 100;

fn ready_superstep_nodes(run: &FlowRunV1) -> Vec<String> {
    run.ready_nodes
        .iter()
        .filter(|node_id| {
            let node_id = node_id.as_str();
            run.graph
                .nodes
                .iter()
                .find(|node| node.id == node_id)
                .is_some_and(|node| node.kind != GraphNodeKindV1::Join || join_ready(run, node_id))
        })
        .cloned()
        .collect()
}

fn node_is_parallel_safe(node: &GraphNodeV1) -> bool {
    matches!(
        node.kind,
        GraphNodeKindV1::Condition
            | GraphNodeKindV1::Validator
            | GraphNodeKindV1::Join
            | GraphNodeKindV1::Loop
            | GraphNodeKindV1::Output
    ) || node.config.get("parallelSafe").and_then(Value::as_bool) == Some(true)
}

fn begin_superstep(
    store: &dyn crate::SessionStore,
    run: &mut FlowRunV1,
    node_ids: &[String],
) -> anyhow::Result<()> {
    anyhow::ensure!(!node_ids.is_empty(), "cannot begin an empty superstep");
    anyhow::ensure!(
        run.node_executions.saturating_add(node_ids.len() as u32) <= run.budget.max_node_executions,
        "Flow node execution budget exhausted ({})",
        run.budget.max_node_executions
    );
    let expected = run.revision;
    let mut nodes = Vec::with_capacity(node_ids.len());
    for node_id in node_ids {
        let input = node_input(run, node_id);
        let node_run_id = Uuid::new_v4();
        let attempt = run.next_attempt(node_id);
        run.node_runs.push(FlowNodeRunV1 {
            id: node_run_id,
            node_id: node_id.clone(),
            attempt,
            status: FlowNodeRunStatusV1::Running,
            input: input.clone(),
            output: None,
            error: None,
            tool_calls: 0,
            transcript: vec![FlowTranscriptEntryV1::new(
                FlowTranscriptEntryKindV1::Input,
                "Node input",
                input.clone(),
            )],
            started_at: Utc::now(),
            completed_at: None,
        });
        nodes.push(WorkflowSuperstepNodeV1 {
            node_id: node_id.clone(),
            node_run_id,
            attempt,
            input,
        });
        remove_first(&mut run.ready_nodes, node_id);
    }
    run.node_executions = run
        .node_executions
        .saturating_add(u32::try_from(nodes.len()).unwrap_or(u32::MAX));
    run.status = FlowRunStatusV1::Running;
    run.started_at.get_or_insert_with(Utc::now);
    run.active_checkpoint = Some(WorkflowCheckpointV1 {
        id: Uuid::new_v4(),
        superstep: run.superstep.saturating_add(1),
        status: WorkflowCheckpointStatusV1::Running,
        nodes,
        pending_writes: Vec::new(),
        created_at: Utc::now(),
        completed_at: None,
    });
    run.touch();
    store.update_flow_run(run, expected)?;
    Ok(())
}

async fn execute_checkpoint_node(
    run: FlowRunV1,
    item: WorkflowSuperstepNodeV1,
    context: ToolInvocationContext,
    harness: std::sync::Arc<dyn FlowNodeHarness>,
) -> WorkflowPendingWriteV1 {
    let result = async {
        let node = run
            .graph
            .nodes
            .iter()
            .find(|node| node.id == item.node_id)
            .cloned()
            .with_context(|| format!("Flow node disappeared from checkpoint: {}", item.node_id))?;
        match node.kind {
            GraphNodeKindV1::Condition
            | GraphNodeKindV1::Validator
            | GraphNodeKindV1::Join
            | GraphNodeKindV1::Loop
            | GraphNodeKindV1::Output => execute_runtime_node(&node, item.input.clone())
                .map(FlowNodeExecutionOutcomeV1::Completed),
            GraphNodeKindV1::Agent | GraphNodeKindV1::Skill | GraphNodeKindV1::Tool => {
                let node_capabilities = run.effective_capabilities_for_node(&item.node_id);
                let authority = run.effective_connection_authority_for_node(&item.node_id);
                let mut node_context = context;
                restrict_flow_node_connection_context(&mut node_context, &authority)?;
                node_context.capability_projection = node_capabilities.clone();
                harness
                    .execute_flow_node(FlowNodeExecutionRequestV1 {
                        flow_run_id: run.id,
                        node_run_id: item.node_run_id,
                        node,
                        input: item.input.clone(),
                        effective_capabilities: node_capabilities,
                        workflow_agent_spec: run.workflow_agent_spec(&item.node_id).cloned(),
                        remaining_tool_calls: run
                            .budget
                            .max_tool_calls
                            .saturating_sub(run.tool_calls),
                        context: node_context,
                    })
                    .await
            }
            GraphNodeKindV1::Approval => {
                anyhow::bail!("approval nodes cannot execute inside a superstep")
            }
        }
    }
    .await;
    match result {
        Ok(FlowNodeExecutionOutcomeV1::Completed(result)) => WorkflowPendingWriteV1 {
            node_id: item.node_id,
            node_run_id: item.node_run_id,
            result: Some(result),
            error: None,
            interrupt: None,
            resume_command: None,
            completed_at: Utc::now(),
        },
        Ok(FlowNodeExecutionOutcomeV1::Interrupted(interrupt)) => {
            let checkpoint = run
                .active_checkpoint
                .as_ref()
                .expect("checkpoint node execution requires an active checkpoint");
            WorkflowPendingWriteV1 {
                node_id: item.node_id.clone(),
                node_run_id: item.node_run_id,
                result: None,
                error: None,
                interrupt: Some(WorkflowInterruptRequestV1::at_checkpoint(
                    checkpoint.id,
                    checkpoint.superstep,
                    item.node_id,
                    item.node_run_id,
                    interrupt,
                )),
                resume_command: None,
                completed_at: Utc::now(),
            }
        }
        Err(error) => WorkflowPendingWriteV1 {
            node_id: item.node_id,
            node_run_id: item.node_run_id,
            result: None,
            error: Some(error.to_string()),
            interrupt: None,
            resume_command: None,
            completed_at: Utc::now(),
        },
    }
}

async fn execute_resume_checkpoint_node(
    run: FlowRunV1,
    previous_write: WorkflowPendingWriteV1,
    interrupt: WorkflowInterruptRequestV1,
    command: FlowResumeCommandV1,
    context: ToolInvocationContext,
    harness: std::sync::Arc<dyn FlowNodeHarness>,
) -> WorkflowPendingWriteV1 {
    let result = async {
        anyhow::ensure!(
            command.validates(&interrupt),
            "ResumeCommand does not match the active interrupt revision"
        );
        let node = run
            .graph
            .nodes
            .iter()
            .find(|node| node.id == previous_write.node_id)
            .cloned()
            .context("resuming Flow node disappeared from the graph")?;
        anyhow::ensure!(
            matches!(node.kind, GraphNodeKindV1::Agent | GraphNodeKindV1::Skill),
            "only Agent and Skill nodes have resumable continuations"
        );
        let node_capabilities = run.effective_capabilities_for_node(&previous_write.node_id);
        let authority = run.effective_connection_authority_for_node(&previous_write.node_id);
        let mut node_context = context;
        restrict_flow_node_connection_context(&mut node_context, &authority)?;
        node_context.capability_projection = node_capabilities.clone();
        harness
            .resume_flow_node(FlowNodeResumeRequestV1 {
                flow_run_id: run.id,
                node_run_id: previous_write.node_run_id,
                node,
                input: run
                    .active_checkpoint
                    .as_ref()
                    .and_then(|checkpoint| {
                        checkpoint
                            .nodes
                            .iter()
                            .find(|item| item.node_run_id == previous_write.node_run_id)
                    })
                    .map(|item| item.input.clone())
                    .unwrap_or(Value::Null),
                effective_capabilities: node_capabilities,
                workflow_agent_spec: run.workflow_agent_spec(&previous_write.node_id).cloned(),
                remaining_tool_calls: run.budget.max_tool_calls.saturating_sub(run.tool_calls),
                interrupt: interrupt.clone(),
                command: command.clone(),
                context: node_context,
            })
            .await
    }
    .await;

    match result {
        Ok(FlowNodeExecutionOutcomeV1::Completed(result)) => WorkflowPendingWriteV1 {
            result: Some(result),
            error: None,
            interrupt: None,
            resume_command: Some(command),
            completed_at: Utc::now(),
            ..previous_write
        },
        Ok(FlowNodeExecutionOutcomeV1::Interrupted(next_interrupt)) => {
            let request = WorkflowInterruptRequestV1::at_checkpoint(
                interrupt.checkpoint_id,
                interrupt.superstep,
                previous_write.node_id.clone(),
                previous_write.node_run_id,
                next_interrupt,
            );
            WorkflowPendingWriteV1 {
                result: None,
                error: None,
                interrupt: Some(request),
                resume_command: Some(command),
                completed_at: Utc::now(),
                ..previous_write
            }
        }
        Err(error) => WorkflowPendingWriteV1 {
            result: None,
            error: None,
            interrupt: Some(WorkflowInterruptRequestV1::resume_retry(
                &interrupt,
                &command,
                error.to_string(),
            )),
            resume_command: Some(command),
            completed_at: Utc::now(),
            ..previous_write
        },
    }
}

fn persist_pending_write(
    store: &dyn crate::SessionStore,
    run_id: Uuid,
    checkpoint_id: Uuid,
    write: WorkflowPendingWriteV1,
) -> anyhow::Result<()> {
    for _ in 0..8 {
        let mut run = store
            .get_flow_run(run_id)?
            .context("Flow run disappeared while recording a pending write")?;
        let expected = run.revision;
        let checkpoint = run
            .active_checkpoint
            .as_mut()
            .filter(|checkpoint| checkpoint.id == checkpoint_id)
            .context("Flow checkpoint changed while a node was executing")?;
        if checkpoint
            .pending_writes
            .iter()
            .any(|existing| existing.node_id == write.node_id)
        {
            return Ok(());
        }
        checkpoint.pending_writes.push(write.clone());
        checkpoint
            .pending_writes
            .sort_by(|left, right| left.node_id.cmp(&right.node_id));
        run.touch();
        match store.update_flow_run(&run, expected) {
            Ok(_) => return Ok(()),
            Err(error)
                if matches!(
                    error.downcast_ref::<crate::FlowStoreError>(),
                    Some(crate::FlowStoreError::RunRevisionConflict(_))
                ) => {}
            Err(error) => return Err(error),
        }
    }
    anyhow::bail!("Flow checkpoint could not persist a pending write after concurrent updates")
}

fn replace_pending_write(
    store: &dyn crate::SessionStore,
    run_id: Uuid,
    checkpoint_id: Uuid,
    replacement: WorkflowPendingWriteV1,
) -> anyhow::Result<()> {
    for _ in 0..8 {
        let mut run = store
            .get_flow_run(run_id)?
            .context("Flow run disappeared while recording a resumed write")?;
        let expected = run.revision;
        let checkpoint = run
            .active_checkpoint
            .as_mut()
            .filter(|checkpoint| checkpoint.id == checkpoint_id)
            .context("Flow checkpoint changed while a node continuation was resuming")?;
        let current = checkpoint
            .pending_writes
            .iter_mut()
            .find(|write| write.node_run_id == replacement.node_run_id)
            .context("resumed Flow node pending write disappeared")?;
        let expected_command_id = current.resume_command.as_ref().map(|command| command.id);
        anyhow::ensure!(
            expected_command_id
                == replacement
                    .resume_command
                    .as_ref()
                    .map(|command| command.id),
            "resumed Flow node command changed while it was executing"
        );
        *current = replacement.clone();
        run.status = FlowRunStatusV1::Running;
        run.active_human_task_id = None;
        run.touch();
        match store.update_flow_run(&run, expected) {
            Ok(_) => return Ok(()),
            Err(error)
                if matches!(
                    error.downcast_ref::<crate::FlowStoreError>(),
                    Some(crate::FlowStoreError::RunRevisionConflict(_))
                ) => {}
            Err(error) => return Err(error),
        }
    }
    anyhow::bail!("Flow continuation result could not be persisted after concurrent updates")
}

fn commit_active_checkpoint(
    store: &dyn crate::SessionStore,
    run_id: Uuid,
    checkpoint_id: Uuid,
) -> anyhow::Result<()> {
    let mut run = store
        .get_flow_run(run_id)?
        .context("Flow run disappeared before checkpoint commit")?;
    let expected = run.revision;
    let mut checkpoint = run
        .active_checkpoint
        .take()
        .filter(|checkpoint| checkpoint.id == checkpoint_id)
        .context("Flow checkpoint changed before commit")?;
    if run.status == FlowRunStatusV1::CancelRequested {
        run.active_checkpoint = Some(checkpoint);
        cancel_active_checkpoint(&mut run);
        run.status = FlowRunStatusV1::Cancelled;
        run.completed_at = Some(Utc::now());
        run.touch();
        store.update_flow_run(&run, expected)?;
        return Ok(());
    }
    anyhow::ensure!(
        checkpoint.pending_writes.len() == checkpoint.nodes.len(),
        "Flow checkpoint has incomplete pending writes"
    );
    checkpoint
        .pending_writes
        .sort_by(|left, right| left.node_id.cmp(&right.node_id));
    if let Some(interrupt) = checkpoint
        .pending_writes
        .iter()
        .find_map(|write| write.interrupt.clone())
    {
        return pause_for_interrupt(store, &mut run, expected, checkpoint, interrupt);
    }
    let mut succeeded = Vec::new();
    let mut routed_errors = Vec::new();
    let mut unhandled_error = None;
    let mut output_values = Vec::new();
    for write in &checkpoint.pending_writes {
        let node = run
            .graph
            .nodes
            .iter()
            .find(|node| node.id == write.node_id)
            .cloned()
            .context("Flow checkpoint references a missing graph node")?;
        let node_run = run
            .node_runs
            .iter_mut()
            .find(|candidate| candidate.id == write.node_run_id)
            .context("Flow checkpoint node run disappeared")?;
        node_run.completed_at = Some(write.completed_at);
        if let Some(result) = &write.result {
            node_run.status = FlowNodeRunStatusV1::Succeeded;
            node_run.output = Some(result.output.clone());
            node_run.tool_calls = result.tool_calls;
            extend_transcript_unique(&mut node_run.transcript, &result.transcript);
            node_run.transcript.push(FlowTranscriptEntryV1::new(
                FlowTranscriptEntryKindV1::Output,
                "Node output",
                result.output.clone(),
            ));
            run.tool_calls = run.tool_calls.saturating_add(result.tool_calls);
            succeeded.push((node, result.output.clone()));
        } else {
            let error = write.error.as_deref().unwrap_or("node execution failed");
            node_run.status = FlowNodeRunStatusV1::Failed;
            node_run.error = Some(error.to_string());
            node_run.transcript.push(FlowTranscriptEntryV1::new(
                FlowTranscriptEntryKindV1::Error,
                "Node failed",
                json!({"message": error}),
            ));
            if let Some(target) = error_route(&run.graph, &write.node_id) {
                routed_errors.push(target);
            } else if unhandled_error.is_none() {
                unhandled_error = Some(format!("node {} failed: {error}", write.node_id));
            }
        }
    }

    if run.tool_calls > run.budget.max_tool_calls && unhandled_error.is_none() {
        unhandled_error = Some(format!(
            "Flow tool-call budget exhausted ({})",
            run.budget.max_tool_calls
        ));
    }
    if unhandled_error.is_none() {
        for (node, output) in &succeeded {
            run.node_outputs.insert(node.id.clone(), output.clone());
            let writes = parse_state_writes(&node.config).map_err(anyhow::Error::msg)?;
            apply_state_writes(&mut run.state, &writes, output).map_err(anyhow::Error::msg)?;
        }
        for (node, output) in &succeeded {
            if node.kind == GraphNodeKindV1::Output {
                output_values.push((node.id.clone(), output.clone()));
            } else {
                route_after_node(&mut run, &node.id, output)?;
            }
        }
        for target in routed_errors {
            enqueue_unique(&mut run.ready_nodes, target);
        }
    }

    let completed_at = Utc::now();
    checkpoint.completed_at = Some(completed_at);
    checkpoint.status = if unhandled_error.is_some() {
        WorkflowCheckpointStatusV1::Failed
    } else {
        WorkflowCheckpointStatusV1::Committed
    };
    run.superstep = checkpoint.superstep;
    run.checkpoint_history.push(WorkflowCheckpointSummaryV1 {
        id: checkpoint.id,
        superstep: checkpoint.superstep,
        status: checkpoint.status,
        node_ids: checkpoint
            .nodes
            .iter()
            .map(|node| node.node_id.clone())
            .collect(),
        pending_write_count: u32::try_from(checkpoint.pending_writes.len()).unwrap_or(u32::MAX),
        created_at: checkpoint.created_at,
        completed_at,
    });
    if run.checkpoint_history.len() > MAX_CHECKPOINT_HISTORY {
        let overflow = run.checkpoint_history.len() - MAX_CHECKPOINT_HISTORY;
        run.checkpoint_history.drain(0..overflow);
    }
    if let Some(error) = unhandled_error {
        run.status = FlowRunStatusV1::Failed;
        run.error = Some(error);
        run.completed_at = Some(completed_at);
    } else if !output_values.is_empty() {
        let review_node_id = output_values.first().map(|(node_id, _)| node_id.clone());
        run.output = if output_values.len() == 1 {
            output_values.pop().map(|(_, output)| output)
        } else {
            Some(Value::Object(output_values.into_iter().collect()))
        };
        if run.output_review_required && !run.output_reviewed {
            let node_run = review_node_id.as_ref().and_then(|node_id| {
                run.node_runs
                    .iter()
                    .rev()
                    .find(|node_run| node_run.node_id == *node_id)
            });
            let task = HumanTaskV1::flow_output_review(
                run.thread_id,
                run.id,
                node_run.map(|node_run| node_run.id),
                review_node_id,
                checkpoint.id,
                json!({
                    "flowId": run.flow_id,
                    "flowVersion": run.flow_version,
                    "checkpointId": checkpoint.id,
                    "output": run.output,
                }),
            );
            run.status = FlowRunStatusV1::WaitingHuman;
            run.active_human_task_id = Some(task.id);
            run.touch();
            store.update_flow_run_and_human_task(&run, expected, &task, None)?;
            return Ok(());
        }
        run.status = FlowRunStatusV1::Succeeded;
        run.completed_at = Some(completed_at);
    } else if run.status == FlowRunStatusV1::PauseRequested {
        run.status = FlowRunStatusV1::Paused;
    } else {
        run.status = FlowRunStatusV1::Running;
    }
    run.touch();
    store.update_flow_run(&run, expected)?;
    Ok(())
}

fn pause_for_interrupt(
    store: &dyn crate::SessionStore,
    run: &mut FlowRunV1,
    expected_revision: u32,
    checkpoint: WorkflowCheckpointV1,
    interrupt: WorkflowInterruptRequestV1,
) -> anyhow::Result<()> {
    let node_run = run
        .node_runs
        .iter_mut()
        .find(|candidate| candidate.id == interrupt.node_run_id)
        .context("interrupted Flow node run disappeared")?;
    node_run.status = FlowNodeRunStatusV1::WaitingHuman;
    extend_transcript_unique(&mut node_run.transcript, &interrupt.transcript);
    node_run.transcript.push(FlowTranscriptEntryV1::new(
        match interrupt.kind {
            WorkflowInterruptKindV1::Approval => FlowTranscriptEntryKindV1::Approval,
            WorkflowInterruptKindV1::InputRequest
            | WorkflowInterruptKindV1::ExternalAction
            | WorkflowInterruptKindV1::EffectReconciliation
            | WorkflowInterruptKindV1::ResumeRetry => FlowTranscriptEntryKindV1::Input,
        },
        "Waiting for human input",
        json!({
            "interruptId": interrupt.id,
            "kind": interrupt.kind,
            "checkpointId": interrupt.checkpoint_id,
            "continuationId": interrupt.continuation.id,
        }),
    ));

    let mut payload = match interrupt.payload.clone() {
        Value::Object(payload) => payload,
        value => {
            let mut payload = Map::new();
            payload.insert("value".to_string(), value);
            payload
        }
    };
    payload.insert("flowId".to_string(), json!(run.flow_id));
    payload.insert("flowVersion".to_string(), json!(run.flow_version));
    payload.insert("interruptId".to_string(), json!(interrupt.id));
    payload.insert("interruptRevision".to_string(), json!(interrupt.revision));
    payload.insert("checkpointId".to_string(), json!(interrupt.checkpoint_id));
    payload.insert("superstep".to_string(), json!(interrupt.superstep));
    payload.insert("nodeId".to_string(), json!(interrupt.node_id));
    payload.insert(
        "continuationId".to_string(),
        json!(interrupt.continuation.id),
    );
    if let Some(effect_id) = payload
        .get("effectId")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
    {
        if let Some(receipt) = store.get_effect(effect_id)? {
            payload.insert(
                "activityReceipt".to_string(),
                serde_json::to_value(receipt)?,
            );
        }
    }
    let task =
        HumanTaskV1::flow_interrupt(run.thread_id, run.id, &interrupt, Value::Object(payload));
    run.status = FlowRunStatusV1::WaitingHuman;
    run.waiting_node_id = Some(interrupt.node_id.clone());
    run.active_human_task_id = Some(task.id);
    run.active_checkpoint = Some(checkpoint);
    run.touch();
    store.update_flow_run_and_human_task(run, expected_revision, &task, None)?;
    Ok(())
}

fn extend_transcript_unique(
    target: &mut Vec<FlowTranscriptEntryV1>,
    additions: &[FlowTranscriptEntryV1],
) {
    let existing = target
        .iter()
        .map(|entry| entry.id)
        .collect::<std::collections::HashSet<_>>();
    target.extend(
        additions
            .iter()
            .filter(|entry| !existing.contains(&entry.id))
            .cloned(),
    );
}

fn cancel_active_checkpoint(run: &mut FlowRunV1) {
    let Some(mut checkpoint) = run.active_checkpoint.take() else {
        return;
    };
    let completed_at = Utc::now();
    for item in &checkpoint.nodes {
        if let Some(node_run) = run
            .node_runs
            .iter_mut()
            .find(|candidate| candidate.id == item.node_run_id && candidate.completed_at.is_none())
        {
            node_run.status = FlowNodeRunStatusV1::Cancelled;
            node_run.error = Some("Flow run cancelled before superstep commit".to_string());
            node_run.completed_at = Some(completed_at);
        }
    }
    checkpoint.status = WorkflowCheckpointStatusV1::Cancelled;
    checkpoint.completed_at = Some(completed_at);
    run.checkpoint_history.push(WorkflowCheckpointSummaryV1 {
        id: checkpoint.id,
        superstep: checkpoint.superstep,
        status: checkpoint.status,
        node_ids: checkpoint
            .nodes
            .iter()
            .map(|node| node.node_id.clone())
            .collect(),
        pending_write_count: u32::try_from(checkpoint.pending_writes.len()).unwrap_or(u32::MAX),
        created_at: checkpoint.created_at,
        completed_at,
    });
}

fn pause_for_approval(
    store: &dyn crate::SessionStore,
    run: &mut FlowRunV1,
    node: &GraphNodeV1,
) -> anyhow::Result<()> {
    let node_id = node.id.clone();
    let input = node_input(run, &node_id);
    let expected = run.revision;
    remove_first(&mut run.ready_nodes, &node_id);
    let node_run_id = Uuid::new_v4();
    let attempt = run.next_attempt(&node_id);
    let task_input = input.clone();
    run.node_runs.push(FlowNodeRunV1 {
        id: node_run_id,
        node_id: node_id.clone(),
        attempt,
        status: FlowNodeRunStatusV1::WaitingApproval,
        input: input.clone(),
        output: None,
        error: None,
        tool_calls: 0,
        transcript: vec![
            FlowTranscriptEntryV1::new(FlowTranscriptEntryKindV1::Input, "Node input", input),
            FlowTranscriptEntryV1::new(
                FlowTranscriptEntryKindV1::Approval,
                "Waiting for human approval",
                json!({"status": "pending"}),
            ),
        ],
        started_at: Utc::now(),
        completed_at: None,
    });
    run.node_executions = run.node_executions.saturating_add(1);
    run.started_at.get_or_insert_with(Utc::now);
    run.waiting_node_id = Some(node_id.clone());
    run.status = FlowRunStatusV1::WaitingApproval;
    let task = HumanTaskV1::flow_approval(
        run.thread_id,
        run.id,
        node_run_id,
        node_id.clone(),
        &node.label,
        json!({
            "flowId": run.flow_id,
            "flowVersion": run.flow_version,
            "nodeId": node_id,
            "nodeLabel": node.label,
            "input": task_input,
        }),
    );
    run.active_human_task_id = Some(task.id);
    run.touch();
    store.update_flow_run_and_human_task(run, expected, &task, None)?;
    Ok(())
}

fn execute_runtime_node(
    node: &GraphNodeV1,
    input: Value,
) -> anyhow::Result<FlowNodeExecutionResultV1> {
    let output = match node.kind {
        GraphNodeKindV1::Validator => {
            let required = node
                .config
                .get("requiredFields")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let missing = required
                .iter()
                .filter_map(Value::as_str)
                .filter(|field| value_at_path(&input, field).is_none())
                .map(str::to_string)
                .collect::<Vec<_>>();
            let expression_passed = node
                .config
                .get("expression")
                .and_then(Value::as_str)
                .map(|expression| evaluate_condition(expression, &input))
                .unwrap_or(true);
            json!({
                "passed": missing.is_empty() && expression_passed,
                "missingFields": missing,
                "value": input,
            })
        }
        GraphNodeKindV1::Condition => json!({
            "matched": node.config.get("expression").and_then(Value::as_str)
                .map(|expression| evaluate_condition(expression, &input)).unwrap_or(true),
            "value": input,
        }),
        GraphNodeKindV1::Join | GraphNodeKindV1::Loop | GraphNodeKindV1::Output => input,
        _ => anyhow::bail!(
            "node kind {:?} is not a deterministic runtime node",
            node.kind
        ),
    };
    Ok(FlowNodeExecutionResultV1 {
        output,
        tool_calls: 0,
        transcript: Vec::new(),
    })
}

fn enforce_run_budget(run: &FlowRunV1) -> anyhow::Result<()> {
    anyhow::ensure!(
        run.node_executions < run.budget.max_node_executions,
        "Flow node execution budget exhausted ({})",
        run.budget.max_node_executions
    );
    anyhow::ensure!(
        run.tool_calls <= run.budget.max_tool_calls,
        "Flow tool-call budget exhausted ({})",
        run.budget.max_tool_calls
    );
    let elapsed = Utc::now()
        .signed_duration_since(run.started_at.unwrap_or(run.created_at))
        .to_std()
        .unwrap_or(Duration::ZERO);
    anyhow::ensure!(
        elapsed.as_secs() <= run.budget.max_duration_seconds,
        "Flow duration budget exhausted ({} seconds)",
        run.budget.max_duration_seconds
    );
    Ok(())
}

fn join_ready(run: &FlowRunV1, node_id: &str) -> bool {
    run.graph
        .edges
        .iter()
        .filter(|edge| edge.to == node_id && edge.loop_policy.is_none())
        .all(|edge| run.node_outputs.contains_key(&edge.from))
}

fn node_input(run: &FlowRunV1, node_id: &str) -> Value {
    let incoming = run
        .graph
        .edges
        .iter()
        .filter(|edge| edge.to == node_id)
        .filter_map(|edge| {
            run.node_outputs
                .get(&edge.from)
                .map(|value| (edge.from.clone(), project_edge_value(value, edge)))
        })
        .collect::<Vec<_>>();
    if incoming.is_empty() {
        return run.input.clone();
    }
    if incoming.len() == 1 {
        return incoming[0].1.clone();
    }
    Value::Object(incoming.into_iter().collect::<Map<_, _>>())
}

fn project_edge_value(value: &Value, edge: &GraphEdgeV1) -> Value {
    if edge.allowed_fields.is_empty() {
        return value.clone();
    }
    let Some(object) = value.as_object() else {
        return value.clone();
    };
    Value::Object(
        object
            .iter()
            .filter(|(key, _)| edge.allowed_fields.contains(*key))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    )
}

fn route_after_node(run: &mut FlowRunV1, node_id: &str, output: &Value) -> anyhow::Result<()> {
    let edges = run
        .graph
        .edges
        .iter()
        .enumerate()
        .filter(|(_, edge)| edge.from == node_id)
        .map(|(index, edge)| (index, edge.clone()))
        .collect::<Vec<_>>();
    for (index, edge) in edges {
        let condition_matches = edge
            .condition
            .as_deref()
            .map(|condition| evaluate_condition(condition, output))
            .unwrap_or(true);
        if !condition_matches {
            continue;
        }
        if let Some(policy) = edge.loop_policy.as_ref() {
            if !evaluate_condition(&policy.continue_condition, output) {
                continue;
            }
            if !consume_loop_budget(run, index, policy)? {
                continue;
            }
        }
        enqueue_unique(&mut run.ready_nodes, edge.to);
    }
    Ok(())
}

fn consume_loop_budget(
    run: &mut FlowRunV1,
    edge_index: usize,
    policy: &GraphLoopPolicyV1,
) -> anyhow::Result<bool> {
    let key = edge_index.to_string();
    let count = run.loop_counts.entry(key).or_default();
    let maximum = policy.max_iterations.min(run.budget.max_loop_iterations);
    if *count < maximum {
        *count = count.saturating_add(1);
        return Ok(true);
    }
    match policy.on_exhausted {
        LoopExhaustionActionV1::Fail => {
            anyhow::bail!("Flow loop budget exhausted after {maximum} iteration(s)")
        }
        LoopExhaustionActionV1::RequireHuman => {
            run.status = FlowRunStatusV1::Paused;
            run.error = Some(format!(
                "loop budget exhausted after {maximum} iteration(s); human decision required"
            ));
            Ok(false)
        }
        LoopExhaustionActionV1::ReturnPartial => {
            run.status = FlowRunStatusV1::Succeeded;
            run.output = terminal_output(run);
            run.completed_at = Some(Utc::now());
            Ok(false)
        }
    }
}

fn error_route(graph: &GraphDefinitionV1, node_id: &str) -> Option<String> {
    graph
        .edges
        .iter()
        .find(|edge| edge.from == node_id && edge.on_error.is_some())
        .and_then(|edge| edge.on_error.clone())
}

fn terminal_output(run: &FlowRunV1) -> Option<Value> {
    run.node_runs
        .iter()
        .rev()
        .find_map(|node| node.output.clone())
        .or_else(|| Some(run.input.clone()))
}

fn enqueue_unique(queue: &mut Vec<String>, node_id: String) {
    if !queue.iter().any(|queued| queued == &node_id) {
        queue.push(node_id);
    }
}

fn remove_first(queue: &mut Vec<String>, node_id: &str) {
    if let Some(index) = queue.iter().position(|queued| queued == node_id) {
        queue.remove(index);
    }
}

pub fn evaluate_condition(expression: &str, value: &Value) -> bool {
    let expression = expression.trim();
    if expression.eq_ignore_ascii_case("true") {
        return true;
    }
    if expression.eq_ignore_ascii_case("false") {
        return false;
    }
    if let Some(path) = expression.strip_prefix('!') {
        return !value_truthy(value_at_path(value, path.trim()).unwrap_or(&Value::Null));
    }
    for operator in ["==", "!="] {
        if let Some((left, right)) = expression.split_once(operator) {
            let actual = value_at_path(value, left.trim()).unwrap_or(&Value::Null);
            let expected = parse_condition_literal(right.trim());
            return if operator == "==" {
                actual == &expected
            } else {
                actual != &expected
            };
        }
    }
    value_at_path(value, expression)
        .map(value_truthy)
        .unwrap_or(false)
}

fn value_at_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let path = path
        .trim()
        .trim_start_matches("$.")
        .trim_start_matches("output.");
    if path.is_empty() || path == "$" || path == "output" {
        return Some(value);
    }
    path.split('.')
        .try_fold(value, |current, segment| current.as_object()?.get(segment))
}

fn parse_condition_literal(value: &str) -> Value {
    serde_json::from_str(value).unwrap_or_else(|_| {
        Value::String(
            value
                .trim_matches(|character| character == '\'' || character == '"')
                .to_string(),
        )
    })
}

fn value_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enterprise::{AgentRiskClassV1, DataClassification};
    use crate::flow::{FlowSourceV1, FlowSpecV1};
    use crate::model::ExperienceMode;
    use crate::policy::{BasicPolicyEngine, PermissionMode};
    use crate::store::{SessionStore, SqliteSessionStore};
    use async_trait::async_trait;
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    struct NeverCalledHarness;

    struct ParallelHarness {
        active: AtomicUsize,
        max_active: AtomicUsize,
    }

    struct InterruptHarness {
        starts: AtomicUsize,
        resumes: AtomicUsize,
    }

    #[derive(Default)]
    struct SpecRecordingHarness {
        specs: Mutex<Vec<WorkflowAgentSpecV1>>,
    }

    #[async_trait]
    impl FlowNodeHarness for SpecRecordingHarness {
        async fn execute_flow_node(
            &self,
            request: FlowNodeExecutionRequestV1,
        ) -> anyhow::Result<FlowNodeExecutionOutcomeV1> {
            let spec = request
                .workflow_agent_spec
                .context("Agent execution must receive its frozen WorkflowAgentSpec")?;
            anyhow::ensure!(request.effective_capabilities == spec.capabilities);
            self.specs.lock().expect("spec mutex").push(spec);
            Ok(FlowNodeExecutionOutcomeV1::Completed(
                FlowNodeExecutionResultV1 {
                    output: json!({"reviewed": true}),
                    tool_calls: 0,
                    transcript: Vec::new(),
                },
            ))
        }
    }

    impl InterruptHarness {
        fn new() -> Self {
            Self {
                starts: AtomicUsize::new(0),
                resumes: AtomicUsize::new(0),
            }
        }
    }

    impl ParallelHarness {
        fn new() -> Self {
            Self {
                active: AtomicUsize::new(0),
                max_active: AtomicUsize::new(0),
            }
        }

        fn max_active(&self) -> usize {
            self.max_active.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl FlowNodeHarness for NeverCalledHarness {
        async fn execute_flow_node(
            &self,
            _request: FlowNodeExecutionRequestV1,
        ) -> anyhow::Result<FlowNodeExecutionOutcomeV1> {
            anyhow::bail!("deterministic runtime test unexpectedly called the Agent Harness")
        }
    }

    #[async_trait]
    impl FlowNodeHarness for ParallelHarness {
        async fn execute_flow_node(
            &self,
            request: FlowNodeExecutionRequestV1,
        ) -> anyhow::Result<FlowNodeExecutionOutcomeV1> {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(active, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(30)).await;
            self.active.fetch_sub(1, Ordering::SeqCst);
            Ok(FlowNodeExecutionOutcomeV1::Completed(
                FlowNodeExecutionResultV1 {
                    output: json!({"node": request.node.id}),
                    tool_calls: 0,
                    transcript: Vec::new(),
                },
            ))
        }
    }

    #[async_trait]
    impl FlowNodeHarness for InterruptHarness {
        async fn execute_flow_node(
            &self,
            request: FlowNodeExecutionRequestV1,
        ) -> anyhow::Result<FlowNodeExecutionOutcomeV1> {
            self.starts.fetch_add(1, Ordering::SeqCst);
            let continuation = crate::AgentContinuation {
                thread_id: request.context.thread_id.expect("thread"),
                turn_id: request.flow_run_id,
                invocation_id: 1,
                user_message_id: request.node_run_id,
                workspace_root: request.context.workspace_root.clone(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: request.context.permission_mode,
                execution_authority: None,
                context_budget: None,
                rollout_budget: None,
                model_context: Default::default(),
                collaboration_mode: Default::default(),
                goal: None,
                state: crate::AgentContinuationState::Provider {
                    model_user_message: "continue".to_string(),
                    model_user_content: Vec::new(),
                    tool_candidates: Vec::new(),
                    provider_tool_calls: Vec::new(),
                    provider_tool_results: Vec::new(),
                    pending_tool_calls: Vec::new(),
                    compacted_tool_history: String::new(),
                    provider_response_items: Vec::new(),
                    model_rounds: 1,
                    rollout_reviews: 0,
                    runtime_state: Default::default(),
                    branch_developer_instructions: None,
                    provider_compatibility_hash: String::new(),
                },
            };
            Ok(FlowNodeExecutionOutcomeV1::Interrupted(
                FlowNodeInterruptV1::new(
                    WorkflowInterruptKindV1::Approval,
                    "Approve dynamic action",
                    "Resume the same node",
                    json!({ "approvalId": Uuid::new_v4() }),
                    &continuation,
                    0,
                    Vec::new(),
                )?,
            ))
        }

        async fn resume_flow_node(
            &self,
            request: FlowNodeResumeRequestV1,
        ) -> anyhow::Result<FlowNodeExecutionOutcomeV1> {
            self.resumes.fetch_add(1, Ordering::SeqCst);
            anyhow::ensure!(request.command.validates(&request.interrupt));
            Ok(FlowNodeExecutionOutcomeV1::Completed(
                FlowNodeExecutionResultV1 {
                    output: json!({ "resumed": true }),
                    tool_calls: 0,
                    transcript: request.interrupt.transcript,
                },
            ))
        }
    }

    fn runtime_node(id: &str, kind: GraphNodeKindV1, config: Value) -> GraphNodeV1 {
        GraphNodeV1 {
            id: id.to_string(),
            label: id.to_string(),
            kind,
            config,
            input_schema: json!({"type": "object"}),
            output_schema: json!({"type": "object"}),
        }
    }

    fn edge(from: &str, to: &str) -> GraphEdgeV1 {
        GraphEdgeV1 {
            from: from.to_string(),
            to: to.to_string(),
            condition: None,
            allowed_fields: BTreeSet::new(),
            data_classification: DataClassification::Internal,
            on_error: None,
            loop_policy: None,
        }
    }

    fn definition(nodes: Vec<GraphNodeV1>, edges: Vec<GraphEdgeV1>) -> FlowDefinitionV1 {
        let graph = GraphDefinitionV1 {
            schema_version: ENTERPRISE_SCHEMA_VERSION_V1,
            entry_node_id: nodes[0].id.clone(),
            nodes,
            edges,
        };
        let spec = FlowSpecV1 {
            flow_id: "runtime-test".to_string(),
            name: "Runtime test".to_string(),
            description: "Exercise the durable coordinator".to_string(),
            owner: "test".to_string(),
            categories: BTreeSet::new(),
            source: FlowSourceV1::NaturalLanguage {
                description: "test".to_string(),
            },
            input_schema: json!({"type": "object"}),
            output_schema: json!({"type": "object"}),
            graph: graph.clone(),
            requested_capabilities: CapabilityProjection::unrestricted(),
            budget: FlowBudgetV1::default(),
            risk_class: AgentRiskClassV1::Low,
            pending_decisions: Vec::new(),
        };
        FlowDefinitionV1 {
            schema_version: ENTERPRISE_SCHEMA_VERSION_V1,
            id: Uuid::new_v4(),
            flow_id: spec.flow_id,
            name: spec.name,
            version: 1,
            owner: spec.owner,
            description: spec.description,
            categories: spec.categories,
            source: spec.source,
            graph,
            input_schema: spec.input_schema,
            output_schema: spec.output_schema,
            capabilities: spec.requested_capabilities,
            budget: spec.budget,
            risk_class: spec.risk_class,
            content_hash: "runtime-test-hash".to_string(),
            published_at: Utc::now(),
            published_by: "test".to_string(),
        }
    }

    fn runtime_context(store: Arc<SqliteSessionStore>, thread_id: Uuid) -> ToolInvocationContext {
        let workspace = PathBuf::from(".");
        let policy = Arc::new(BasicPolicyEngine::new(
            workspace.clone(),
            PermissionMode::FullAccess,
        ));
        let mut context = ToolInvocationContext::local(workspace, policy);
        context.state = Some(crate::tool_state::ToolStateStore::new(store));
        context.thread_id = Some(thread_id);
        context.flow_harness = Some(Arc::new(NeverCalledHarness));
        context
    }

    async fn wait_for_status(
        store: &SqliteSessionStore,
        run_id: Uuid,
        status: FlowRunStatusV1,
    ) -> FlowRunV1 {
        for _ in 0..100 {
            let run = store
                .get_flow_run(run_id)
                .expect("read Flow run")
                .expect("Flow run exists");
            if run.status == status {
                return run;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("Flow run did not reach {status:?}");
    }

    #[test]
    fn conditions_are_small_and_deterministic() {
        let value = json!({"passed": true, "score": 3, "nested": {"ready": false}});
        assert!(evaluate_condition("passed == true", &value));
        assert!(evaluate_condition("score != 2", &value));
        assert!(evaluate_condition("!nested.ready", &value));
        assert!(!evaluate_condition("missing", &value));
        assert!(evaluate_condition(
            "value.ready == true",
            &json!({"value": {"ready": true}})
        ));
    }

    #[test]
    fn edge_projection_only_forwards_allowed_fields() {
        let edge = GraphEdgeV1 {
            from: "a".to_string(),
            to: "b".to_string(),
            condition: None,
            allowed_fields: ["safe".to_string()].into_iter().collect(),
            data_classification: crate::enterprise::DataClassification::Internal,
            on_error: None,
            loop_policy: None,
        };
        assert_eq!(
            project_edge_value(&json!({"safe": 1, "secret": 2}), &edge),
            json!({"safe": 1})
        );
    }

    #[test]
    fn old_flow_runs_only_infer_legacy_authority_from_their_frozen_projection() {
        let definition = definition(
            vec![runtime_node("output", GraphNodeKindV1::Output, json!({}))],
            Vec::new(),
        );
        let run = FlowRunV1::new(
            Uuid::new_v4(),
            &definition,
            json!({}),
            &CapabilityProjection::unrestricted(),
        )
        .expect("create run");
        let mut persisted = serde_json::to_value(run).expect("serialize run");
        persisted
            .as_object_mut()
            .expect("run object")
            .remove("connectionAuthority");
        let restored: FlowRunV1 = serde_json::from_value(persisted).expect("restore old run");

        assert_eq!(
            restored.effective_connection_authority(),
            RuntimeConnectionAuthorityV1::LegacyMcp
        );
    }

    #[test]
    fn flow_run_freezes_only_structured_operations_inside_its_definition_ceiling() {
        fn operation(name: &str) -> crate::ExecutionConnectionOperationV1 {
            crate::ExecutionConnectionOperationV1 {
                connection_id: Uuid::new_v4(),
                capability_revision: 1,
                operation_id: format!("connection:test:tool:{name}"),
                mcp_server_id: Uuid::new_v4(),
                provider_tool_name: name.to_string(),
                model_tool_name: format!("{name}__account"),
                pinned_operation_fingerprint: format!("sha256:{name}"),
            }
        }

        let allowed = operation("allowed");
        let excluded = operation("excluded");
        let mut definition = definition(
            vec![runtime_node("output", GraphNodeKindV1::Output, json!({}))],
            Vec::new(),
        );
        definition.capabilities = CapabilityProjection::deny_all();
        definition
            .capabilities
            .tools
            .insert(allowed.model_tool_name.clone());
        definition
            .capabilities
            .mcp_servers
            .insert(allowed.mcp_server_id.to_string());
        let mut available = definition.capabilities.clone();
        available.tools.insert(excluded.model_tool_name.clone());
        available
            .mcp_servers
            .insert(excluded.mcp_server_id.to_string());

        let run = FlowRunV1::new_with_connection_authority(
            Uuid::new_v4(),
            &definition,
            json!({}),
            &available,
            RuntimeConnectionAuthorityV1::Structured {
                operations: vec![allowed.clone(), excluded],
            },
        )
        .expect("create structured run");

        assert_eq!(
            run.effective_connection_authority(),
            RuntimeConnectionAuthorityV1::Structured {
                operations: vec![allowed]
            }
        );
    }

    #[test]
    fn deployed_flow_run_owns_the_immutable_deployment_snapshot() {
        let mut definition = definition(
            vec![runtime_node("output", GraphNodeKindV1::Output, json!({}))],
            Vec::new(),
        );
        definition.capabilities = CapabilityProjection::deny_all();
        let compiled =
            crate::CompiledWorkflowV1::compile(&definition, Vec::new()).expect("compile workflow");
        let deployment = crate::WorkflowDeploymentV1::new(
            "Runtime production",
            "production",
            compiled,
            "release-manager",
        )
        .expect("deployment");

        let run = FlowRunV1::new_from_deployment(
            Uuid::new_v4(),
            &deployment,
            json!({"requestId": "lead-1"}),
        )
        .expect("deployed run");
        let restored: FlowRunV1 =
            serde_json::from_str(&serde_json::to_string(&run).expect("serialize deployed run"))
                .expect("restore deployed run");

        assert_eq!(restored.deployment_id, Some(deployment.id));
        assert_eq!(
            restored
                .deployment_snapshot
                .as_ref()
                .map(|snapshot| snapshot.content_hash.as_str()),
            Some(deployment.snapshot.content_hash.as_str())
        );
        assert_eq!(
            restored.harness_connection_authority(),
            RuntimeConnectionAuthorityV1::Structured {
                operations: Vec::new()
            }
        );
        assert!(restored.output_review_required);
        assert!(!restored.output_reviewed);
    }

    #[test]
    fn deployment_can_limit_review_to_explicit_approval_nodes() {
        let definition = definition(
            vec![runtime_node("output", GraphNodeKindV1::Output, json!({}))],
            Vec::new(),
        );
        let compiled =
            crate::CompiledWorkflowV1::compile(&definition, Vec::new()).expect("compile workflow");
        let deployment = crate::WorkflowDeploymentV1::new_with_options(
            "Unattended runtime",
            "production",
            compiled,
            crate::WorkflowTriggerSpecV1::Manual,
            crate::WorkflowOutputSpecV1::Inbox,
            crate::WorkflowOutputReviewPolicyV1::ExplicitNodesOnly,
            "release-manager",
        )
        .expect("deployment");

        let run = FlowRunV1::new_from_deployment(Uuid::new_v4(), &deployment, json!({}))
            .expect("deployed run");

        assert!(!run.output_review_required);
    }

    #[tokio::test]
    async fn deployed_agent_node_executes_only_its_frozen_workflow_agent_spec() {
        let store = Arc::new(SqliteSessionStore::open(":memory:").expect("open store"));
        let thread = store
            .create_thread_with_mode(
                Some("Frozen workflow Agent".to_string()),
                PathBuf::from("."),
                ExperienceMode::Flow,
            )
            .expect("create Flow thread");
        let definition = definition(
            vec![
                runtime_node(
                    "agent",
                    GraphNodeKindV1::Agent,
                    json!({"reference": "reviewer", "templateVersion": 7}),
                ),
                runtime_node("output", GraphNodeKindV1::Output, json!({})),
            ],
            vec![edge("agent", "output")],
        );
        let agent_spec = WorkflowAgentSpecV1 {
            node_id: "agent".to_string(),
            template_id: "reviewer".to_string(),
            template_version: 7,
            template_content_hash: "sha256:frozen-reviewer".to_string(),
            name: "Frozen reviewer".to_string(),
            owner: "risk-team".to_string(),
            instructions: "Use only the frozen reviewer policy".to_string(),
            capabilities: CapabilityProjection::deny_all(),
            resource_grants: Vec::new(),
            model_policy: crate::AgentModelPolicyV1::unrestricted(),
            state_schema: json!({"type": "object"}),
            output_schema: json!({"type": "object"}),
            risk_class: AgentRiskClassV1::Medium,
            connection_bindings: Vec::new(),
            connection_authority: RuntimeConnectionAuthorityV1::Structured {
                operations: Vec::new(),
            },
        };
        let compiled = crate::CompiledWorkflowV1::compile(&definition, vec![agent_spec])
            .expect("compile workflow");
        let deployment = crate::WorkflowDeploymentV1::new(
            "Frozen workflow",
            "production",
            compiled,
            "release-manager",
        )
        .expect("deployment");
        let run = FlowRunV1::new_from_deployment(thread.id, &deployment, json!({"case": 1}))
            .expect("create run");
        let run_id = run.id;
        store.insert_flow_run(&run).expect("persist run");
        let harness = Arc::new(SpecRecordingHarness::default());
        let mut context = runtime_context(store.clone(), thread.id);
        context.flow_harness = Some(harness.clone());

        spawn_flow_run(run_id, context).expect("spawn run");
        wait_for_status(&store, run_id, FlowRunStatusV1::WaitingHuman).await;

        let specs = harness.specs.lock().expect("spec mutex");
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].template_id, "reviewer");
        assert_eq!(specs[0].template_version, 7);
        assert_eq!(specs[0].template_content_hash, "sha256:frozen-reviewer");
        assert_eq!(specs[0].instructions, "Use only the frozen reviewer policy");
    }

    #[tokio::test]
    async fn deterministic_flow_runs_to_persisted_output() {
        let store = Arc::new(SqliteSessionStore::open(":memory:").expect("open store"));
        let thread = store
            .create_thread_with_mode(
                Some("Flow runtime".to_string()),
                PathBuf::from("."),
                ExperienceMode::Flow,
            )
            .expect("create Flow thread");
        let definition = definition(
            vec![
                runtime_node("check", GraphNodeKindV1::Condition, json!({})),
                runtime_node("output", GraphNodeKindV1::Output, json!({})),
            ],
            vec![GraphEdgeV1 {
                from: "check".to_string(),
                to: "output".to_string(),
                condition: None,
                allowed_fields: BTreeSet::new(),
                data_classification: DataClassification::Internal,
                on_error: None,
                loop_policy: None,
            }],
        );
        let run = FlowRunV1::new(
            thread.id,
            &definition,
            json!({"ready": true}),
            &CapabilityProjection::unrestricted(),
        )
        .expect("create run");
        store.insert_flow_run(&run).expect("persist run");
        spawn_flow_run(run.id, runtime_context(store.clone(), thread.id)).expect("spawn run");

        let completed = wait_for_status(&store, run.id, FlowRunStatusV1::Succeeded).await;
        assert_eq!(completed.node_runs.len(), 2);
        assert_eq!(
            completed.node_runs[0]
                .transcript
                .iter()
                .map(|entry| entry.kind)
                .collect::<Vec<_>>(),
            vec![
                FlowTranscriptEntryKindV1::Input,
                FlowTranscriptEntryKindV1::Output,
            ]
        );
        assert_eq!(
            completed.output,
            Some(json!({"matched": true, "value": {"ready": true}}))
        );
        assert_eq!(completed.superstep, 2);
        assert_eq!(completed.checkpoint_history.len(), 2);
        assert!(completed.active_checkpoint.is_none());
        assert_eq!(
            store
                .list_all_flow_runs(Some(FlowRunStatusV1::Succeeded), 20)
                .expect("list global Flow runs")
                .iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            vec![run.id]
        );
    }

    #[tokio::test]
    async fn parallel_superstep_commits_pending_writes_in_stable_node_order() {
        let store = Arc::new(SqliteSessionStore::open(":memory:").expect("open store"));
        let thread = store
            .create_thread_with_mode(
                Some("Parallel Flow".to_string()),
                PathBuf::from("."),
                ExperienceMode::Flow,
            )
            .expect("create Flow thread");
        let state_write = json!([{"channel":"results","reducer":"append"}]);
        let definition = definition(
            vec![
                runtime_node("entry", GraphNodeKindV1::Condition, json!({})),
                runtime_node(
                    "left",
                    GraphNodeKindV1::Tool,
                    json!({"reference":"left_tool","parallelSafe":true,"stateWrites":state_write}),
                ),
                runtime_node(
                    "right",
                    GraphNodeKindV1::Tool,
                    json!({"reference":"right_tool","parallelSafe":true,"stateWrites":[{"channel":"results","reducer":"append"}]}),
                ),
                runtime_node("join", GraphNodeKindV1::Join, json!({})),
                runtime_node("output", GraphNodeKindV1::Output, json!({})),
            ],
            vec![
                edge("entry", "left"),
                edge("entry", "right"),
                edge("left", "join"),
                edge("right", "join"),
                edge("join", "output"),
            ],
        );
        let run = FlowRunV1::new(
            thread.id,
            &definition,
            json!({"request":"parallel"}),
            &CapabilityProjection::unrestricted(),
        )
        .expect("create run");
        store.insert_flow_run(&run).expect("persist run");
        let harness = Arc::new(ParallelHarness::new());
        let mut context = runtime_context(store.clone(), thread.id);
        context.flow_harness = Some(harness.clone());
        spawn_flow_run(run.id, context).expect("spawn run");

        let completed = wait_for_status(&store, run.id, FlowRunStatusV1::Succeeded).await;
        assert_eq!(harness.max_active(), 2);
        assert_eq!(
            completed.state["results"],
            json!([{"node":"left"},{"node":"right"}])
        );
        let parallel_checkpoint = completed
            .checkpoint_history
            .iter()
            .find(|checkpoint| checkpoint.node_ids == vec!["left", "right"])
            .expect("parallel checkpoint");
        assert_eq!(parallel_checkpoint.pending_write_count, 2);
        assert_eq!(
            parallel_checkpoint.status,
            WorkflowCheckpointStatusV1::Committed
        );
    }

    #[test]
    fn recovery_retries_only_nodes_without_a_successful_pending_write() {
        let store = SqliteSessionStore::open(":memory:").expect("open store");
        let thread = store
            .create_thread_with_mode(
                Some("Pending write recovery".to_string()),
                PathBuf::from("."),
                ExperienceMode::Flow,
            )
            .expect("create Flow thread");
        let definition = definition(
            vec![
                runtime_node("left", GraphNodeKindV1::Validator, json!({})),
                runtime_node("right", GraphNodeKindV1::Validator, json!({})),
                runtime_node("output", GraphNodeKindV1::Output, json!({})),
            ],
            vec![
                edge("left", "right"),
                edge("left", "output"),
                edge("right", "output"),
            ],
        );
        let mut run = FlowRunV1::new(
            thread.id,
            &definition,
            json!({"request":"recover"}),
            &CapabilityProjection::unrestricted(),
        )
        .expect("create run");
        run.ready_nodes = vec!["left".to_string(), "right".to_string()];
        store.insert_flow_run(&run).expect("persist run");
        begin_superstep(&store, &mut run, &["left".to_string(), "right".to_string()])
            .expect("begin superstep");
        let checkpoint = run.active_checkpoint.clone().expect("checkpoint");
        let left = checkpoint
            .nodes
            .iter()
            .find(|node| node.node_id == "left")
            .expect("left item");
        let right = checkpoint
            .nodes
            .iter()
            .find(|node| node.node_id == "right")
            .expect("right item");
        let right_original_run_id = right.node_run_id;
        persist_pending_write(
            &store,
            run.id,
            checkpoint.id,
            WorkflowPendingWriteV1 {
                node_id: "left".to_string(),
                node_run_id: left.node_run_id,
                result: Some(FlowNodeExecutionResultV1 {
                    output: json!({"completed":"left"}),
                    tool_calls: 0,
                    transcript: Vec::new(),
                }),
                error: None,
                interrupt: None,
                resume_command: None,
                completed_at: Utc::now(),
            },
        )
        .expect("persist left pending write");

        let mut recovered = store.get_flow_run(run.id).expect("read run").expect("run");
        prepare_flow_resume(&mut recovered, true).expect("prepare retry");
        let resumed = recovered.active_checkpoint.as_ref().expect("checkpoint");
        assert_eq!(resumed.pending_writes.len(), 1);
        assert_eq!(resumed.pending_writes[0].node_id, "left");
        assert_eq!(
            resumed
                .nodes
                .iter()
                .find(|node| node.node_id == "left")
                .expect("left")
                .attempt,
            1
        );
        let right_retry = resumed
            .nodes
            .iter()
            .find(|node| node.node_id == "right")
            .expect("right retry");
        assert_eq!(right_retry.attempt, 2);
        assert_ne!(right_retry.node_run_id, right_original_run_id);
        assert_eq!(
            recovered
                .node_runs
                .iter()
                .find(|node| node.id == right_original_run_id)
                .expect("old right attempt")
                .status,
            FlowNodeRunStatusV1::Cancelled
        );
    }

    #[tokio::test]
    async fn approval_node_resumes_from_the_persisted_boundary() {
        let store = Arc::new(SqliteSessionStore::open(":memory:").expect("open store"));
        let thread = store
            .create_thread_with_mode(
                Some("Flow approval".to_string()),
                PathBuf::from("."),
                ExperienceMode::Flow,
            )
            .expect("create Flow thread");
        let definition = definition(
            vec![
                runtime_node("approve", GraphNodeKindV1::Approval, json!({})),
                runtime_node("output", GraphNodeKindV1::Output, json!({})),
            ],
            vec![GraphEdgeV1 {
                from: "approve".to_string(),
                to: "output".to_string(),
                condition: "approved == true".to_string().into(),
                allowed_fields: BTreeSet::new(),
                data_classification: DataClassification::Internal,
                on_error: None,
                loop_policy: None,
            }],
        );
        let run = FlowRunV1::new(
            thread.id,
            &definition,
            json!({"request": "deploy"}),
            &CapabilityProjection::unrestricted(),
        )
        .expect("create run");
        store.insert_flow_run(&run).expect("persist run");
        let context = runtime_context(store.clone(), thread.id);
        spawn_flow_run(run.id, context.clone()).expect("spawn run");
        let mut waiting = wait_for_status(&store, run.id, FlowRunStatusV1::WaitingApproval).await;
        let mut task = store
            .get_pending_human_task_for_flow_run(run.id)
            .expect("load Human task")
            .expect("approval task");
        assert_eq!(waiting.active_human_task_id, Some(task.id));
        let expected = waiting.revision;
        let task_expected = task.revision;
        resolve_flow_approval(&mut waiting, true, Some("reviewed")).expect("approve");
        task.resolve(
            crate::human_task::HumanTaskActionV1::Approve,
            Some("reviewed"),
            "test_operator",
        )
        .expect("resolve Human task");
        store
            .update_flow_run_and_human_task(&waiting, expected, &task, Some(task_expected))
            .expect("persist approval");
        spawn_flow_run(run.id, context).expect("resume run");

        let completed = wait_for_status(&store, run.id, FlowRunStatusV1::Succeeded).await;
        assert_eq!(
            completed.output,
            Some(json!({"approved": true, "note": "reviewed"}))
        );
    }

    #[tokio::test]
    async fn dynamic_agent_interrupt_resumes_the_same_checkpoint_and_node_attempt() {
        let store = Arc::new(SqliteSessionStore::open(":memory:").expect("open store"));
        let thread = store
            .create_thread_with_mode(
                Some("Dynamic interrupt".to_string()),
                PathBuf::from("."),
                ExperienceMode::Flow,
            )
            .expect("create Flow thread");
        let definition = definition(
            vec![
                runtime_node(
                    "agent",
                    GraphNodeKindV1::Skill,
                    json!({ "reference": "test-skill" }),
                ),
                runtime_node("output", GraphNodeKindV1::Output, json!({})),
            ],
            vec![edge("agent", "output")],
        );
        let run = FlowRunV1::new(
            thread.id,
            &definition,
            json!({ "request": "perform action" }),
            &CapabilityProjection::unrestricted(),
        )
        .expect("create run");
        store.insert_flow_run(&run).expect("persist run");
        let harness = Arc::new(InterruptHarness::new());
        let mut context = runtime_context(store.clone(), thread.id);
        context.flow_harness = Some(harness.clone());
        spawn_flow_run(run.id, context.clone()).expect("spawn run");

        let mut waiting = wait_for_status(&store, run.id, FlowRunStatusV1::WaitingHuman).await;
        let checkpoint_id = waiting.active_checkpoint.as_ref().expect("checkpoint").id;
        let node_run_id = waiting.node_runs[0].id;
        let mut task = store
            .get_pending_human_task_for_flow_run(run.id)
            .expect("load task")
            .expect("interrupt task");
        assert_eq!(task.continuation_id.is_some(), true);
        let run_revision = waiting.revision;
        let task_revision = task.revision;
        let command = prepare_flow_interrupt_resume(
            &mut waiting,
            &task,
            HumanTaskActionV1::Approve,
            None,
            Some("reviewed"),
            "operator",
            "stable-command-key",
        )
        .expect("prepare resume");
        task.resolve_with_command(
            HumanTaskActionV1::Approve,
            Some("reviewed"),
            "operator",
            Some(command.id),
            Some("stable-command-key"),
            None,
        )
        .expect("resolve task");
        store
            .update_flow_run_and_human_task(&waiting, run_revision, &task, Some(task_revision))
            .expect("persist resume command");
        spawn_flow_run(run.id, context).expect("resume run");

        let completed = wait_for_status(&store, run.id, FlowRunStatusV1::Succeeded).await;
        assert_eq!(harness.starts.load(Ordering::SeqCst), 1);
        assert_eq!(harness.resumes.load(Ordering::SeqCst), 1);
        assert_eq!(completed.node_runs[0].id, node_run_id);
        assert_eq!(completed.node_runs[0].attempt, 1);
        assert_eq!(completed.checkpoint_history[0].id, checkpoint_id);
        assert_eq!(completed.node_outputs["agent"], json!({ "resumed": true }));
    }

    #[test]
    fn restart_creates_a_recovery_human_task_for_an_interrupted_node() {
        let database_path = std::env::temp_dir().join(format!(
            "opentopia-flow-recovery-{}-{}.db",
            std::process::id(),
            Uuid::new_v4()
        ));
        let run_id;
        {
            let store = SqliteSessionStore::open(&database_path).expect("open store");
            let thread = store
                .create_thread_with_mode(
                    Some("Interrupted Flow".to_string()),
                    PathBuf::from("."),
                    ExperienceMode::Flow,
                )
                .expect("create Flow thread");
            let definition = definition(
                vec![
                    runtime_node("external-write", GraphNodeKindV1::Condition, json!({})),
                    runtime_node("output", GraphNodeKindV1::Output, json!({})),
                ],
                vec![GraphEdgeV1 {
                    from: "external-write".to_string(),
                    to: "output".to_string(),
                    condition: None,
                    allowed_fields: BTreeSet::new(),
                    data_classification: DataClassification::Internal,
                    on_error: None,
                    loop_policy: None,
                }],
            );
            let mut run = FlowRunV1::new(
                thread.id,
                &definition,
                json!({"request": "sync"}),
                &CapabilityProjection::unrestricted(),
            )
            .expect("create run");
            run_id = run.id;
            run.status = FlowRunStatusV1::Running;
            run.ready_nodes.clear();
            run.started_at = Some(Utc::now());
            run.node_runs.push(FlowNodeRunV1 {
                id: Uuid::new_v4(),
                node_id: "external-write".to_string(),
                attempt: 1,
                status: FlowNodeRunStatusV1::Running,
                input: json!({"request": "sync"}),
                output: None,
                error: None,
                tool_calls: 1,
                transcript: vec![],
                started_at: Utc::now(),
                completed_at: None,
            });
            store.insert_flow_run(&run).expect("persist running Flow");
        }

        {
            let reopened = SqliteSessionStore::open(&database_path).expect("reopen store");
            let run = reopened
                .get_flow_run(run_id)
                .expect("load recovered Flow")
                .expect("recovered Flow");
            let task = reopened
                .get_pending_human_task_for_flow_run(run_id)
                .expect("load recovery task")
                .expect("recovery task");
            assert_eq!(run.status, FlowRunStatusV1::Paused);
            assert_eq!(run.active_human_task_id, Some(task.id));
            assert_eq!(task.task_type, crate::human_task::HumanTaskTypeV1::Recovery);
            assert_eq!(task.payload["sideEffectState"], "unknown");
        }

        let _ = std::fs::remove_file(&database_path);
        let _ = std::fs::remove_file(database_path.with_extension("db-wal"));
        let _ = std::fs::remove_file(database_path.with_extension("db-shm"));
    }

    #[tokio::test]
    async fn node_budget_stops_before_the_next_node() {
        let store = Arc::new(SqliteSessionStore::open(":memory:").expect("open store"));
        let thread = store
            .create_thread_with_mode(
                Some("Flow budget".to_string()),
                PathBuf::from("."),
                ExperienceMode::Flow,
            )
            .expect("create Flow thread");
        let mut definition = definition(
            vec![
                runtime_node("check", GraphNodeKindV1::Condition, json!({})),
                runtime_node("output", GraphNodeKindV1::Output, json!({})),
            ],
            vec![GraphEdgeV1 {
                from: "check".to_string(),
                to: "output".to_string(),
                condition: None,
                allowed_fields: BTreeSet::new(),
                data_classification: DataClassification::Internal,
                on_error: None,
                loop_policy: None,
            }],
        );
        definition.budget.max_node_executions = 1;
        let run = FlowRunV1::new(
            thread.id,
            &definition,
            json!({"ready": true}),
            &CapabilityProjection::unrestricted(),
        )
        .expect("create run");
        store.insert_flow_run(&run).expect("persist run");
        spawn_flow_run(run.id, runtime_context(store.clone(), thread.id)).expect("spawn run");

        let failed = wait_for_status(&store, run.id, FlowRunStatusV1::Failed).await;
        assert_eq!(failed.node_runs.len(), 1);
        assert!(failed
            .error
            .as_deref()
            .is_some_and(|error| error.contains("execution budget exhausted")));
    }
}
