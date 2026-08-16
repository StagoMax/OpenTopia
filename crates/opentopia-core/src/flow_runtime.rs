use crate::enterprise::{CapabilityProjection, ENTERPRISE_SCHEMA_VERSION_V1};
use crate::flow::{
    compile_flow, FlowBudgetV1, FlowDefinitionV1, GraphDefinitionV1, GraphEdgeV1,
    GraphLoopPolicyV1, GraphNodeKindV1, GraphNodeV1, LoopExhaustionActionV1,
};
use crate::tools::ToolInvocationContext;
use anyhow::Context;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::time::Duration;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlowRunStatusV1 {
    Queued,
    Running,
    PauseRequested,
    Paused,
    WaitingApproval,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlowNodeRunStatusV1 {
    Running,
    WaitingApproval,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlowTranscriptEntryKindV1 {
    Input,
    ToolCall,
    ToolResult,
    Output,
    Approval,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowRunV1 {
    pub schema_version: u16,
    pub id: Uuid,
    pub thread_id: Uuid,
    pub flow_id: String,
    pub flow_version: u32,
    pub definition_id: Uuid,
    pub definition_content_hash: String,
    pub revision: u32,
    pub status: FlowRunStatusV1,
    pub input: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    pub graph: GraphDefinitionV1,
    pub effective_capabilities: CapabilityProjection,
    pub budget: FlowBudgetV1,
    #[serde(default)]
    pub ready_nodes: Vec<String>,
    #[serde(default)]
    pub node_runs: Vec<FlowNodeRunV1>,
    #[serde(default)]
    pub node_outputs: BTreeMap<String, Value>,
    #[serde(default)]
    pub loop_counts: BTreeMap<String, u32>,
    pub node_executions: u32,
    pub tool_calls: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_node_id: Option<String>,
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
        let effective_capabilities = definition.capabilities.intersect(available_capabilities);
        anyhow::ensure!(
            effective_capabilities == definition.capabilities,
            "the current ExecutionContext is narrower than the published Flow capability snapshot"
        );
        let spec = definition.as_spec();
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
            revision: 1,
            status: FlowRunStatusV1::Queued,
            input,
            output: None,
            graph: definition.graph.clone(),
            effective_capabilities,
            budget: definition.budget.clone(),
            ready_nodes: vec![definition.graph.entry_node_id.clone()],
            node_runs: Vec::new(),
            node_outputs: BTreeMap::new(),
            loop_counts: BTreeMap::new(),
            node_executions: 0,
            tool_calls: 0,
            waiting_node_id: None,
            error: None,
            created_at: now,
            started_at: None,
            completed_at: None,
            updated_at: now,
        })
    }

    pub fn active_node_run_mut(&mut self, node_id: &str) -> Option<&mut FlowNodeRunV1> {
        self.node_runs
            .iter_mut()
            .rev()
            .find(|node| node.node_id == node_id && node.completed_at.is_none())
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

trait FlowDefinitionSpec {
    fn as_spec(&self) -> crate::flow::FlowSpecV1;
}

impl FlowDefinitionSpec for FlowDefinitionV1 {
    fn as_spec(&self) -> crate::flow::FlowSpecV1 {
        crate::flow::FlowSpecV1 {
            flow_id: self.flow_id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            owner: self.owner.clone(),
            categories: self.categories.clone(),
            source: self.source.clone(),
            input_schema: self.input_schema.clone(),
            output_schema: self.output_schema.clone(),
            graph: self.graph.clone(),
            requested_capabilities: self.capabilities.clone(),
            budget: self.budget.clone(),
            risk_class: self.risk_class,
            pending_decisions: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub struct FlowNodeExecutionRequestV1 {
    pub flow_run_id: Uuid,
    pub node_run_id: Uuid,
    pub node: GraphNodeV1,
    pub input: Value,
    pub effective_capabilities: CapabilityProjection,
    pub remaining_tool_calls: u32,
    pub context: ToolInvocationContext,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowNodeExecutionResultV1 {
    pub output: Value,
    pub tool_calls: u32,
    #[serde(default)]
    pub transcript: Vec<FlowTranscriptEntryV1>,
}

#[async_trait]
pub trait FlowNodeHarness: Send + Sync {
    async fn execute_flow_node(
        &self,
        request: FlowNodeExecutionRequestV1,
    ) -> anyhow::Result<FlowNodeExecutionResultV1>;
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

pub fn prepare_flow_resume(
    run: &mut FlowRunV1,
    retry_interrupted_node: bool,
) -> anyhow::Result<()> {
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
        .ok_or_else(|| anyhow::anyhow!("Flow Runtime requires the Agent Harness"))?;

    loop {
        let mut run = store
            .get_flow_run(run_id)?
            .ok_or_else(|| anyhow::anyhow!("Flow run not found: {run_id}"))?;
        if run.status.is_terminal() || run.status == FlowRunStatusV1::WaitingApproval {
            return Ok(());
        }
        if run.status == FlowRunStatusV1::CancelRequested {
            let expected = run.revision;
            run.status = FlowRunStatusV1::Cancelled;
            run.completed_at = Some(Utc::now());
            run.touch();
            store.update_flow_run(&run, expected)?;
            return Ok(());
        }
        if run.status == FlowRunStatusV1::PauseRequested {
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
        let Some(node_id) = next_ready_node(&run) else {
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
        };
        let node = run
            .graph
            .nodes
            .iter()
            .find(|node| node.id == node_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Flow node disappeared from run snapshot: {node_id}"))?;
        let input = node_input(&run, &node_id);
        let expected = run.revision;
        remove_first(&mut run.ready_nodes, &node_id);
        if node.kind == GraphNodeKindV1::Approval {
            let transcript_input = input.clone();
            let node_run = FlowNodeRunV1 {
                id: Uuid::new_v4(),
                node_id: node_id.clone(),
                attempt: run.next_attempt(&node_id),
                status: FlowNodeRunStatusV1::WaitingApproval,
                input,
                output: None,
                error: None,
                tool_calls: 0,
                transcript: vec![
                    FlowTranscriptEntryV1::new(
                        FlowTranscriptEntryKindV1::Input,
                        "Node input",
                        transcript_input,
                    ),
                    FlowTranscriptEntryV1::new(
                        FlowTranscriptEntryKindV1::Approval,
                        "Waiting for human approval",
                        json!({"status": "pending"}),
                    ),
                ],
                started_at: Utc::now(),
                completed_at: None,
            };
            run.node_runs.push(node_run);
            run.node_executions = run.node_executions.saturating_add(1);
            run.started_at.get_or_insert_with(Utc::now);
            run.waiting_node_id = Some(node_id);
            run.status = FlowRunStatusV1::WaitingApproval;
            run.touch();
            store.update_flow_run(&run, expected)?;
            return Ok(());
        }

        let node_run_id = Uuid::new_v4();
        run.node_runs.push(FlowNodeRunV1 {
            id: node_run_id,
            node_id: node_id.clone(),
            attempt: run.next_attempt(&node_id),
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
        run.node_executions = run.node_executions.saturating_add(1);
        run.status = FlowRunStatusV1::Running;
        run.started_at.get_or_insert_with(Utc::now);
        run.touch();
        store.update_flow_run(&run, expected)?;

        let result = match node.kind {
            GraphNodeKindV1::Condition
            | GraphNodeKindV1::Validator
            | GraphNodeKindV1::Join
            | GraphNodeKindV1::Loop
            | GraphNodeKindV1::Output => execute_runtime_node(&node, input),
            GraphNodeKindV1::Agent | GraphNodeKindV1::Skill | GraphNodeKindV1::Tool => {
                harness
                    .execute_flow_node(FlowNodeExecutionRequestV1 {
                        flow_run_id: run_id,
                        node_run_id,
                        node: node.clone(),
                        input,
                        effective_capabilities: run.effective_capabilities.clone(),
                        remaining_tool_calls: run
                            .budget
                            .max_tool_calls
                            .saturating_sub(run.tool_calls),
                        context: context.clone(),
                    })
                    .await
            }
            GraphNodeKindV1::Approval => unreachable!("approval nodes pause before execution"),
        };

        let mut run = store
            .get_flow_run(run_id)?
            .ok_or_else(|| anyhow::anyhow!("Flow run not found after node execution"))?;
        let expected = run.revision;
        match result {
            Ok(result) => {
                let now = Utc::now();
                let node_run = run
                    .node_runs
                    .iter_mut()
                    .find(|candidate| candidate.id == node_run_id)
                    .ok_or_else(|| anyhow::anyhow!("active Flow node run disappeared"))?;
                node_run.status = FlowNodeRunStatusV1::Succeeded;
                node_run.output = Some(result.output.clone());
                node_run.tool_calls = result.tool_calls;
                node_run.transcript.extend(result.transcript);
                node_run.transcript.push(FlowTranscriptEntryV1::new(
                    FlowTranscriptEntryKindV1::Output,
                    "Node output",
                    result.output.clone(),
                ));
                node_run.completed_at = Some(now);
                run.tool_calls = run.tool_calls.saturating_add(result.tool_calls);
                run.node_outputs
                    .insert(node_id.clone(), result.output.clone());
                if node.kind == GraphNodeKindV1::Output {
                    run.output = Some(result.output);
                    run.status = FlowRunStatusV1::Succeeded;
                    run.completed_at = Some(now);
                } else {
                    route_after_node(&mut run, &node_id, &result.output)?;
                }
            }
            Err(error) => {
                if let Some(node_run) = run
                    .node_runs
                    .iter_mut()
                    .find(|candidate| candidate.id == node_run_id)
                {
                    node_run.status = FlowNodeRunStatusV1::Failed;
                    node_run.error = Some(error.to_string());
                    node_run.transcript.push(FlowTranscriptEntryV1::new(
                        FlowTranscriptEntryKindV1::Error,
                        "Node failed",
                        json!({"message": error.to_string()}),
                    ));
                    node_run.completed_at = Some(Utc::now());
                }
                if let Some(target) = error_route(&run.graph, &node_id) {
                    enqueue_unique(&mut run.ready_nodes, target);
                } else {
                    run.status = FlowRunStatusV1::Failed;
                    run.error = Some(format!("node {node_id} failed: {error}"));
                    run.completed_at = Some(Utc::now());
                }
            }
        }
        run.touch();
        store.update_flow_run(&run, expected)?;
    }
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

fn next_ready_node(run: &FlowRunV1) -> Option<String> {
    run.ready_nodes.iter().find_map(|node_id| {
        let node = run.graph.nodes.iter().find(|node| node.id == *node_id)?;
        if node.kind != GraphNodeKindV1::Join || join_ready(run, node_id) {
            Some(node_id.clone())
        } else {
            None
        }
    })
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
    use std::sync::Arc;

    struct NeverCalledHarness;

    #[async_trait]
    impl FlowNodeHarness for NeverCalledHarness {
        async fn execute_flow_node(
            &self,
            _request: FlowNodeExecutionRequestV1,
        ) -> anyhow::Result<FlowNodeExecutionResultV1> {
            anyhow::bail!("deterministic runtime test unexpectedly called the Agent Harness")
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
        let expected = waiting.revision;
        resolve_flow_approval(&mut waiting, true, Some("reviewed")).expect("approve");
        store
            .update_flow_run(&waiting, expected)
            .expect("persist approval");
        spawn_flow_run(run.id, context).expect("resume run");

        let completed = wait_for_status(&store, run.id, FlowRunStatusV1::Succeeded).await;
        assert_eq!(
            completed.output,
            Some(json!({"approved": true, "note": "reviewed"}))
        );
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
