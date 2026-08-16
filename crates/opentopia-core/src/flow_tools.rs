use crate::flow::{
    simulate_flow, validate_flow_spec, FlowDraftStatusV1, FlowDraftV1, FlowSourceV1, FlowSpecV1,
};
use crate::flow_runtime::{
    prepare_flow_resume, resolve_flow_approval, spawn_flow_run, FlowRunStatusV1, FlowRunV1,
};
use crate::model::{ExperienceMode, ToolCall, ToolResult, TurnStatus};
use crate::tools::{Tool, ToolExecutionPolicy, ToolInvocationContext, ToolSideEffect};
use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Copy)]
enum FlowToolAction {
    Search,
    Create,
    Update,
    Inspect,
    Validate,
    Simulate,
    Publish,
    Run,
    Status,
    Pause,
    Resume,
    Cancel,
}

pub fn flow_tools() -> Vec<(String, Arc<dyn Tool>)> {
    [
        FlowToolAction::Search,
        FlowToolAction::Create,
        FlowToolAction::Update,
        FlowToolAction::Inspect,
        FlowToolAction::Validate,
        FlowToolAction::Simulate,
        FlowToolAction::Publish,
        FlowToolAction::Run,
        FlowToolAction::Status,
        FlowToolAction::Pause,
        FlowToolAction::Resume,
        FlowToolAction::Cancel,
    ]
    .into_iter()
    .map(|action| {
        let tool = FlowTool { action };
        (tool.name().to_string(), Arc::new(tool) as Arc<dyn Tool>)
    })
    .collect()
}

struct FlowTool {
    action: FlowToolAction,
}

impl FlowTool {
    fn store<'a>(
        &self,
        ctx: &'a ToolInvocationContext,
    ) -> anyhow::Result<&'a Arc<dyn crate::store::SessionStore>> {
        ctx.state
            .as_ref()
            .map(crate::tool_state::ToolStateStore::flow_session_store)
            .ok_or_else(|| anyhow::anyhow!("{} requires a persistent SessionStore", self.name()))
    }

    fn thread_id(&self, ctx: &ToolInvocationContext) -> anyhow::Result<Uuid> {
        ctx.thread_id
            .ok_or_else(|| anyhow::anyhow!("{} requires an active thread", self.name()))
    }

    fn require_flow_thread(&self, ctx: &ToolInvocationContext) -> anyhow::Result<Uuid> {
        let thread_id = self.thread_id(ctx)?;
        let thread = self
            .store(ctx)?
            .get_thread(thread_id)?
            .ok_or_else(|| anyhow::anyhow!("active thread not found"))?;
        anyhow::ensure!(
            thread.experience_mode == ExperienceMode::Flow,
            "{} is only available in Flow mode",
            self.name()
        );
        Ok(thread_id)
    }

    fn result(call_id: Uuid, value: Value) -> anyhow::Result<ToolResult> {
        let output = serde_json::to_string_pretty(&value)?;
        Ok(ToolResult::text(call_id, output, value))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SearchInput {
    #[serde(default)]
    query: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateInput {
    spec: FlowSpecV1,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateInput {
    draft_id: Uuid,
    expected_revision: u32,
    spec: FlowSpecV1,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InspectInput {
    #[serde(default)]
    draft_id: Option<Uuid>,
    #[serde(default)]
    flow_id: Option<String>,
    #[serde(default)]
    version: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DraftInput {
    draft_id: Uuid,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SimulateInput {
    draft_id: Uuid,
    #[serde(default)]
    input: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PublishInput {
    draft_id: Uuid,
    published_by: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RunInput {
    flow_id: String,
    #[serde(default)]
    version: Option<u32>,
    #[serde(default)]
    input: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RunIdInput {
    run_id: Uuid,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StatusInput {
    #[serde(default)]
    run_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResumeInput {
    run_id: Uuid,
    #[serde(default)]
    approved: Option<bool>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    retry_interrupted_node: bool,
}

fn flow_spec_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["flowId", "name", "description", "owner", "source", "graph", "requestedCapabilities", "riskClass"],
        "properties": {
            "flowId": {"type": "string", "description": "Stable kebab-case identifier."},
            "name": {"type": "string"},
            "description": {"type": "string"},
            "owner": {"type": "string"},
            "categories": {"type": "array", "items": {"type": "string"}},
            "source": {
                "type": "object",
                "description": "Use kind=natural_language with description, or kind=run_trace with a successful runId and traceHash."
            },
            "inputSchema": {"type": "object"},
            "outputSchema": {"type": "object"},
            "graph": {
                "type": "object",
                "description": "Complete reviewable graph. Node kinds: agent, skill, tool, condition, validator, approval, join, loop, output. Cycles require a bounded loopPolicy on the feedback edge."
            },
            "requestedCapabilities": {
                "type": "object",
                "description": "Capabilities requested from the current ExecutionContext. This can only narrow, never grant access."
            },
            "budget": {"type": "object"},
            "riskClass": {"type": "string", "enum": ["low", "medium", "high", "critical"]},
            "pendingDecisions": {"type": "array", "items": {"type": "string"}}
        }
    })
}

#[async_trait]
impl Tool for FlowTool {
    fn name(&self) -> &str {
        match self.action {
            FlowToolAction::Search => "flow_search",
            FlowToolAction::Create => "flow_create",
            FlowToolAction::Update => "flow_update",
            FlowToolAction::Inspect => "flow_inspect",
            FlowToolAction::Validate => "flow_validate",
            FlowToolAction::Simulate => "flow_simulate",
            FlowToolAction::Publish => "flow_publish",
            FlowToolAction::Run => "flow_run",
            FlowToolAction::Status => "flow_status",
            FlowToolAction::Pause => "flow_pause",
            FlowToolAction::Resume => "flow_resume",
            FlowToolAction::Cancel => "flow_cancel",
        }
    }

    fn description(&self) -> &str {
        match self.action {
            FlowToolAction::Search => "Search reusable published Flows and current Flow drafts before designing a duplicate.",
            FlowToolAction::Create => "Create and bind a complete FlowDraft from a natural-language workflow or a successful existing Run/Trace. Use this for reusable cross-role or long-running dependency graphs, not a short vertical task that belongs in a Skill. The Flow only requests capabilities already visible in the current ExecutionContext.",
            FlowToolAction::Update => "Replace a FlowDraft specification using optimistic revision control after reviewing validation issues or user feedback.",
            FlowToolAction::Inspect => "Inspect a FlowDraft with its validation and simulation history, or inspect an immutable published Flow version.",
            FlowToolAction::Validate => "Statically validate graph topology, schemas, references, capability boundaries, risk gates, budgets, and bounded termination without calling another model.",
            FlowToolAction::Simulate => "Compile and dry-run a valid FlowDraft against the existing Agent Harness, showing which AgentCore, SubagentScheduler, SkillRuntime, ToolRegistry, and runtime-control primitives each node will use. No business side effects are executed.",
            FlowToolAction::Publish => "Publish an immutable Flow version after current-revision validation and simulation pass. High-risk Flows require an independent approver.",
            FlowToolAction::Run => "Start an immutable published Flow in the durable Flow Runtime. The runtime schedules graph dependencies and control nodes, while Agent, Skill, and Tool nodes execute through the currently restricted Agent Harness.",
            FlowToolAction::Status => "Inspect one durable Flow run or list recent runs for the current Flow session, including node attempts, outputs, budgets, and pending control state.",
            FlowToolAction::Pause => "Request a Flow run pause. The request takes effect at the next node boundary so an in-flight side effect is not interrupted into an unknown state.",
            FlowToolAction::Resume => "Resume a paused Flow run, or resolve its explicit approval node with approved=true/false. Execution continues from the persisted node boundary.",
            FlowToolAction::Cancel => "Request cancellation of a Flow run at the next node boundary.",
        }
    }

    fn schema(&self) -> Value {
        match self.action {
            FlowToolAction::Search => json!({
                "type": "object", "additionalProperties": false,
                "properties": {"query": {"type": "string"}}
            }),
            FlowToolAction::Create => json!({
                "type": "object", "additionalProperties": false,
                "required": ["spec"], "properties": {"spec": flow_spec_schema()}
            }),
            FlowToolAction::Update => json!({
                "type": "object", "additionalProperties": false,
                "required": ["draftId", "expectedRevision", "spec"],
                "properties": {
                    "draftId": {"type": "string", "format": "uuid"},
                    "expectedRevision": {"type": "integer", "minimum": 1},
                    "spec": flow_spec_schema()
                }
            }),
            FlowToolAction::Inspect => json!({
                "type": "object", "additionalProperties": false,
                "properties": {
                    "draftId": {"type": "string", "format": "uuid"},
                    "flowId": {"type": "string"},
                    "version": {"type": "integer", "minimum": 1}
                }
            }),
            FlowToolAction::Validate => draft_schema(),
            FlowToolAction::Simulate => json!({
                "type": "object", "additionalProperties": false,
                "required": ["draftId"],
                "properties": {
                    "draftId": {"type": "string", "format": "uuid"},
                    "input": {}
                }
            }),
            FlowToolAction::Publish => json!({
                "type": "object", "additionalProperties": false,
                "required": ["draftId", "publishedBy"],
                "properties": {
                    "draftId": {"type": "string", "format": "uuid"},
                    "publishedBy": {"type": "string", "minLength": 1}
                }
            }),
            FlowToolAction::Run => json!({
                "type": "object", "additionalProperties": false,
                "required": ["flowId"],
                "properties": {
                    "flowId": {"type": "string"},
                    "version": {"type": "integer", "minimum": 1},
                    "input": {}
                }
            }),
            FlowToolAction::Status => json!({
                "type": "object", "additionalProperties": false,
                "properties": {"runId": {"type": "string", "format": "uuid"}}
            }),
            FlowToolAction::Pause | FlowToolAction::Cancel => run_id_schema(),
            FlowToolAction::Resume => json!({
                "type": "object", "additionalProperties": false,
                "required": ["runId"],
                "properties": {
                    "runId": {"type": "string", "format": "uuid"},
                    "approved": {"type": "boolean"},
                    "note": {"type": "string"},
                    "retryInterruptedNode": {
                        "type": "boolean",
                        "description": "After a process restart, explicitly retry a node that may have stopped mid-side-effect. Inspect external state first."
                    }
                }
            }),
        }
    }

    fn has_derived_input_schema(&self) -> bool {
        // Flow tools are one host-owned, action-generated static family. Their
        // action match owns both the inline schema above and the matching typed
        // serde input used by execute, so they satisfy the same static-tool
        // contract without twelve otherwise identical wrapper structs.
        true
    }

    fn execution_policy(&self, call: &ToolCall) -> ToolExecutionPolicy {
        let resource = |kind: &str, field: &str, fallback: &str| {
            let value = call
                .input
                .get(field)
                .and_then(Value::as_str)
                .unwrap_or(fallback)
                .trim()
                .replace('\\', "/");
            format!("{kind}:{value}")
        };
        match self.action {
            FlowToolAction::Search => {
                ToolExecutionPolicy::read_only(vec!["flows:catalog".to_string()])
            }
            FlowToolAction::Inspect => ToolExecutionPolicy::read_only(vec![if call
                .input
                .get("flowId")
                .and_then(Value::as_str)
                .is_some()
            {
                resource("flow-definition", "flowId", "current")
            } else {
                resource("flow-draft", "draftId", "current")
            }]),
            FlowToolAction::Status => {
                ToolExecutionPolicy::read_only(vec![resource("flow-run", "runId", "current")])
            }
            FlowToolAction::Validate | FlowToolAction::Simulate => ToolExecutionPolicy {
                read_only: false,
                idempotent: false,
                parallel_safe: true,
                side_effect: ToolSideEffect::SessionMutation,
                resource_keys: vec![resource("flow-draft", "draftId", "current")],
            },
            FlowToolAction::Create | FlowToolAction::Update | FlowToolAction::Publish => {
                ToolExecutionPolicy {
                    read_only: false,
                    idempotent: false,
                    parallel_safe: true,
                    side_effect: ToolSideEffect::ControlPlane,
                    resource_keys: vec![resource("flow-draft", "draftId", "current")],
                }
            }
            FlowToolAction::Run => ToolExecutionPolicy {
                read_only: false,
                idempotent: false,
                parallel_safe: true,
                side_effect: ToolSideEffect::ControlPlane,
                resource_keys: vec![resource("flow-definition", "flowId", "current")],
            },
            FlowToolAction::Pause | FlowToolAction::Resume | FlowToolAction::Cancel => {
                ToolExecutionPolicy {
                    read_only: false,
                    idempotent: false,
                    parallel_safe: true,
                    side_effect: ToolSideEffect::ControlPlane,
                    resource_keys: vec![resource("flow-run", "runId", "current")],
                }
            }
        }
    }

    async fn execute(
        &self,
        call: ToolCall,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let store = self.store(&ctx)?;
        match self.action {
            FlowToolAction::Search => {
                self.require_flow_thread(&ctx)?;
                let input: SearchInput = serde_json::from_value(call.input)?;
                let definitions = store.search_flow_definitions(&input.query)?;
                let drafts = store
                    .list_flow_drafts(Some(self.thread_id(&ctx)?))?
                    .into_iter()
                    .filter(|draft| {
                        input.query.trim().is_empty()
                            || draft.spec.flow_id.contains(input.query.trim())
                            || draft.spec.name.contains(input.query.trim())
                    })
                    .collect::<Vec<_>>();
                Self::result(
                    call.id,
                    json!({"definitions": definitions, "drafts": drafts}),
                )
            }
            FlowToolAction::Create => {
                let thread_id = self.require_flow_thread(&ctx)?;
                let input: CreateInput = serde_json::from_value(call.input)?;
                if let FlowSourceV1::RunTrace { run_id, .. } = &input.spec.source {
                    let run = store
                        .get_turn(*run_id)?
                        .ok_or_else(|| anyhow::anyhow!("source Run/Trace not found: {run_id}"))?;
                    anyhow::ensure!(
                        run.status == TurnStatus::Succeeded,
                        "only a successful Run/Trace can be converted into a FlowDraft"
                    );
                }
                let draft = FlowDraftV1::new(thread_id, input.spec, &ctx.capability_projection);
                let report = validate_flow_spec(&draft.spec, &ctx.capability_projection);
                let draft = store.create_flow_draft(&draft)?;
                Self::result(call.id, json!({"draft": draft, "validation": report}))
            }
            FlowToolAction::Update => {
                self.require_flow_thread(&ctx)?;
                let input: UpdateInput = serde_json::from_value(call.input)?;
                let mut draft = store
                    .get_flow_draft(input.draft_id)?
                    .ok_or_else(|| anyhow::anyhow!("Flow draft not found"))?;
                anyhow::ensure!(
                    draft.revision == input.expected_revision,
                    "Flow draft revision conflict: current revision is {}",
                    draft.revision
                );
                draft.replace_spec(input.spec, &ctx.capability_projection);
                let report = validate_flow_spec(&draft.spec, &ctx.capability_projection);
                let draft = store.update_flow_draft(&draft, input.expected_revision)?;
                Self::result(call.id, json!({"draft": draft, "validation": report}))
            }
            FlowToolAction::Inspect => {
                self.require_flow_thread(&ctx)?;
                let input: InspectInput = serde_json::from_value(call.input)?;
                if let Some(flow_id) = input.flow_id {
                    let definition = store.get_flow_definition(&flow_id, input.version)?;
                    return Self::result(call.id, json!({"definition": definition}));
                }
                let draft = match input.draft_id {
                    Some(id) => store.get_flow_draft(id)?,
                    None => store.get_thread_flow_draft(self.thread_id(&ctx)?)?,
                };
                let trials = match &draft {
                    Some(draft) => store.list_flow_trials(draft.id)?,
                    None => Vec::new(),
                };
                Self::result(call.id, json!({"draft": draft, "trials": trials}))
            }
            FlowToolAction::Validate => {
                self.require_flow_thread(&ctx)?;
                let input: DraftInput = serde_json::from_value(call.input)?;
                let mut draft = store
                    .get_flow_draft(input.draft_id)?
                    .ok_or_else(|| anyhow::anyhow!("Flow draft not found"))?;
                let expected_revision = draft.revision;
                let report = validate_flow_spec(&draft.spec, &ctx.capability_projection);
                draft.status = if report.valid {
                    FlowDraftStatusV1::ReadyToPublish
                } else {
                    FlowDraftStatusV1::Reviewing
                };
                draft.last_validation = Some(report.clone());
                draft.updated_at = Utc::now();
                let draft = store.update_flow_draft(&draft, expected_revision)?;
                Self::result(call.id, json!({"draft": draft, "validation": report}))
            }
            FlowToolAction::Simulate => {
                self.require_flow_thread(&ctx)?;
                let input: SimulateInput = serde_json::from_value(call.input)?;
                let mut draft = store
                    .get_flow_draft(input.draft_id)?
                    .ok_or_else(|| anyhow::anyhow!("Flow draft not found"))?;
                let expected_revision = draft.revision;
                let trial = simulate_flow(&draft, input.input, &ctx.capability_projection);
                draft.last_validation = Some(trial.report.clone());
                draft.status = if trial.report.valid {
                    FlowDraftStatusV1::ReadyToPublish
                } else {
                    FlowDraftStatusV1::Reviewing
                };
                draft.updated_at = Utc::now();
                store.update_flow_draft(&draft, expected_revision)?;
                let trial = store.insert_flow_trial(&trial)?;
                Self::result(call.id, json!({"trial": trial}))
            }
            FlowToolAction::Publish => {
                self.require_flow_thread(&ctx)?;
                let input: PublishInput = serde_json::from_value(call.input)?;
                anyhow::ensure!(
                    !input.published_by.trim().is_empty(),
                    "publishedBy is required"
                );
                let definition =
                    store.publish_flow_draft(input.draft_id, input.published_by.trim())?;
                Self::result(call.id, json!({"definition": definition}))
            }
            FlowToolAction::Run => {
                let thread_id = self.require_flow_thread(&ctx)?;
                let input: RunInput = serde_json::from_value(call.input)?;
                let definition = store
                    .get_flow_definition(input.flow_id.trim(), input.version)?
                    .ok_or_else(|| anyhow::anyhow!("published Flow not found"))?;
                let run = FlowRunV1::new(
                    thread_id,
                    &definition,
                    input.input,
                    &ctx.capability_projection,
                )?;
                let run = store.insert_flow_run(&run)?;
                spawn_flow_run(run.id, ctx.clone())?;
                Self::result(call.id, json!({"run": run}))
            }
            FlowToolAction::Status => {
                let thread_id = self.require_flow_thread(&ctx)?;
                let input: StatusInput = serde_json::from_value(call.input)?;
                if let Some(run_id) = input.run_id {
                    let run = store
                        .get_flow_run(run_id)?
                        .ok_or_else(|| anyhow::anyhow!("Flow run not found"))?;
                    anyhow::ensure!(
                        run.thread_id == thread_id,
                        "Flow run belongs to another session"
                    );
                    return Self::result(call.id, json!({"run": run}));
                }
                Self::result(call.id, json!({"runs": store.list_flow_runs(thread_id)?}))
            }
            FlowToolAction::Pause => {
                let thread_id = self.require_flow_thread(&ctx)?;
                let input: RunIdInput = serde_json::from_value(call.input)?;
                let mut run = store
                    .get_flow_run(input.run_id)?
                    .ok_or_else(|| anyhow::anyhow!("Flow run not found"))?;
                anyhow::ensure!(
                    run.thread_id == thread_id,
                    "Flow run belongs to another session"
                );
                anyhow::ensure!(
                    matches!(
                        run.status,
                        FlowRunStatusV1::Queued | FlowRunStatusV1::Running
                    ),
                    "only a queued or running Flow can be paused"
                );
                let expected = run.revision;
                run.status = FlowRunStatusV1::PauseRequested;
                run.touch();
                let run = store.update_flow_run(&run, expected)?;
                Self::result(call.id, json!({"run": run}))
            }
            FlowToolAction::Resume => {
                let thread_id = self.require_flow_thread(&ctx)?;
                let input: ResumeInput = serde_json::from_value(call.input)?;
                let mut run = store
                    .get_flow_run(input.run_id)?
                    .ok_or_else(|| anyhow::anyhow!("Flow run not found"))?;
                anyhow::ensure!(
                    run.thread_id == thread_id,
                    "Flow run belongs to another session"
                );
                let expected = run.revision;
                match run.status {
                    FlowRunStatusV1::Paused => {
                        anyhow::ensure!(
                            input.approved.is_none(),
                            "approved is only valid while a Flow is waiting for approval"
                        );
                        prepare_flow_resume(&mut run, input.retry_interrupted_node)?;
                        run.status = FlowRunStatusV1::Running;
                        run.error = None;
                        run.touch();
                    }
                    FlowRunStatusV1::WaitingApproval => {
                        resolve_flow_approval(
                            &mut run,
                            input.approved.ok_or_else(|| {
                                anyhow::anyhow!("approved is required for an approval node")
                            })?,
                            input.note.as_deref(),
                        )?;
                    }
                    _ => anyhow::bail!("Flow run is not paused or waiting for approval"),
                }
                let run = store.update_flow_run(&run, expected)?;
                if !run.status.is_terminal() {
                    spawn_flow_run(run.id, ctx.clone())?;
                }
                Self::result(call.id, json!({"run": run}))
            }
            FlowToolAction::Cancel => {
                let thread_id = self.require_flow_thread(&ctx)?;
                let input: RunIdInput = serde_json::from_value(call.input)?;
                let mut run = store
                    .get_flow_run(input.run_id)?
                    .ok_or_else(|| anyhow::anyhow!("Flow run not found"))?;
                anyhow::ensure!(
                    run.thread_id == thread_id,
                    "Flow run belongs to another session"
                );
                anyhow::ensure!(!run.status.is_terminal(), "Flow run is already terminal");
                let expected = run.revision;
                if matches!(
                    run.status,
                    FlowRunStatusV1::Paused | FlowRunStatusV1::WaitingApproval
                ) {
                    run.status = FlowRunStatusV1::Cancelled;
                    run.completed_at = Some(Utc::now());
                } else {
                    run.status = FlowRunStatusV1::CancelRequested;
                }
                run.touch();
                let run = store.update_flow_run(&run, expected)?;
                Self::result(call.id, json!({"run": run}))
            }
        }
    }
}

fn draft_schema() -> Value {
    json!({
        "type": "object", "additionalProperties": false,
        "required": ["draftId"],
        "properties": {"draftId": {"type": "string", "format": "uuid"}}
    })
}

fn run_id_schema() -> Value {
    json!({
        "type": "object", "additionalProperties": false,
        "required": ["runId"],
        "properties": {"runId": {"type": "string", "format": "uuid"}}
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_policies_scope_mutations_to_their_draft_or_run() {
        let first_draft = Uuid::new_v4();
        let second_draft = Uuid::new_v4();
        let validate = FlowTool {
            action: FlowToolAction::Validate,
        }
        .execution_policy(&ToolCall::new(
            "flow_validate",
            json!({ "draftId": first_draft }),
        ));
        let simulate = FlowTool {
            action: FlowToolAction::Simulate,
        }
        .execution_policy(&ToolCall::new(
            "flow_simulate",
            json!({ "draftId": second_draft }),
        ));
        assert!(validate.parallel_safe);
        assert!(simulate.parallel_safe);
        assert!(!validate.read_only);
        assert_ne!(validate.resource_keys, simulate.resource_keys);
        assert_eq!(
            validate.resource_keys,
            vec![format!("flow-draft:{first_draft}")]
        );

        let run_id = Uuid::new_v4();
        let pause = FlowTool {
            action: FlowToolAction::Pause,
        }
        .execution_policy(&ToolCall::new("flow_pause", json!({ "runId": run_id })));
        let cancel = FlowTool {
            action: FlowToolAction::Cancel,
        }
        .execution_policy(&ToolCall::new("flow_cancel", json!({ "runId": run_id })));
        assert!(pause.parallel_safe);
        assert_eq!(pause.resource_keys, cancel.resource_keys);
        assert_eq!(pause.resource_keys, vec![format!("flow-run:{run_id}")]);
    }
}
