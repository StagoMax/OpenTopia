use super::{
    decode_typed_tool_input, derived_tool_schema, enforce_policy_decision, tool_resource_key, Tool,
    ToolExecutionPolicy, ToolInvocationContext, ToolSideEffect, TypedTool,
};
use crate::agent_profiles::AgentProfileRegistry;
use crate::collaboration::{
    AgentCollaborationInvocation, AgentWorkspaceMode, ForkTurns, SpawnChildAgentRequest,
    WaitAgentRequest as CollaborationWaitAgentRequest,
};
use crate::execution_authorization::ToolExecutionIntent;
use crate::model::{ModelContentPart, ToolCall, ToolResult};
use crate::policy::PolicyDecision;
use anyhow::Context;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use std::num::NonZeroU64;
use std::path::Path;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum ForkTurnsLabel {
    None,
    All,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub(super) enum ForkTurnsInput {
    Label(ForkTurnsLabel),
    Count(NonZeroU64),
}

impl ForkTurnsInput {
    fn into_collaboration(self) -> ForkTurns {
        match self {
            Self::Label(ForkTurnsLabel::None) => ForkTurns::None,
            Self::Label(ForkTurnsLabel::All) => ForkTurns::All,
            Self::Count(value) => ForkTurns::Count(value.get() as usize),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum AgentWorkspaceModeInput {
    #[default]
    Auto,
    SharedReadOnly,
    SharedCoordinated,
    IsolatedWorktree,
}

impl AgentWorkspaceModeInput {
    fn into_collaboration(self) -> AgentWorkspaceMode {
        match self {
            Self::Auto => AgentWorkspaceMode::Auto,
            Self::SharedReadOnly => AgentWorkspaceMode::SharedReadOnly,
            Self::SharedCoordinated => AgentWorkspaceMode::SharedCoordinated,
            Self::IsolatedWorktree => AgentWorkspaceMode::IsolatedWorktree,
        }
    }
}

fn default_agent_type() -> String {
    "default".to_string()
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpawnAgentInput {
    /// Stable lowercase task name used in the canonical agent path.
    pub(super) task_name: String,
    /// Concrete initial task for the child agent.
    pub(super) message: String,
    /// Parent history to copy: none, all, or a positive number of turns.
    #[serde(default)]
    pub(super) fork_turns: Option<ForkTurnsInput>,
    /// Built-in or project agent profile name. Defaults to default.
    #[serde(default = "default_agent_type")]
    pub(super) agent_type: String,
    /// Harness workspace contract.
    #[serde(default)]
    pub(super) workspace_mode: AgentWorkspaceModeInput,
    /// Whether the child may recursively create children. Session and parent
    /// policy can still reject or further narrow this request.
    #[serde(default)]
    pub(super) allow_child_spawns: bool,
}

pub struct SpawnAgentTool;

#[async_trait]
impl TypedTool for SpawnAgentTool {
    type Input = SpawnAgentInput;

    fn name(&self) -> &str {
        "spawn_agent"
    }

    fn description(&self) -> &str {
        "Create an independently running child agent. The harness can keep read-only work shared or prepare an isolated Git worktree for an independent writer; the parent remains responsible for selecting and integrating results."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        let resource_keys = if matches!(
            input.workspace_mode,
            AgentWorkspaceModeInput::IsolatedWorktree
        ) {
            vec!["git:index-and-worktree".to_string()]
        } else {
            vec![tool_resource_key("agent-name", &input.task_name)]
        };
        ToolExecutionPolicy {
            read_only: false,
            idempotent: false,
            parallel_safe: true,
            side_effect: ToolSideEffect::ControlPlane,
            resource_keys,
        }
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let collaboration = ctx
            .collaboration
            .as_ref()
            .context("agent collaboration runtime is unavailable")?;
        let name = input.task_name.trim().to_string();
        anyhow::ensure!(!name.is_empty(), "task_name must be a non-empty string");
        let message = input.message.trim().to_string();
        anyhow::ensure!(!message.is_empty(), "message must be a non-empty string");
        let fork_turns = input
            .fork_turns
            .map(ForkTurnsInput::into_collaboration)
            .unwrap_or(ForkTurns::None);
        let agent_type = input.agent_type;
        let profiles = AgentProfileRegistry::load(&ctx.workspace_root);
        if profiles.get(&agent_type).is_none() {
            anyhow::bail!(
                "unknown agent_type `{agent_type}`; call list_agents to inspect available profiles"
            );
        }
        let outcome = collaboration
            .spawn_agent(SpawnChildAgentRequest {
                task_name: name,
                message,
                agent_type,
                fork_turns,
                workspace_mode: input.workspace_mode.into_collaboration(),
                allow_child_spawns: input.allow_child_spawns,
            })
            .await?;
        Ok(ToolResult {
            call_id,
            output: serde_json::to_string(&json!({
                "agent": outcome.agent,
                "turn": outcome.turn,
            }))?,
            content: vec![ModelContentPart::json(json!({
                "agent": outcome.agent,
                "turn": outcome.turn,
            }))],
            metadata: json!({
                "toolName": "spawn_agent",
                "agentThreadId": outcome.agent.id,
                "agentTurnId": outcome.turn.id,
                "agentPath": outcome.agent.path,
                "status": outcome.turn.status,
                "success": true
            }),
        })
    }
}

impl_typed_tool!(SpawnAgentTool);

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentTargetMessageInput {
    /// Agent UUID, canonical path, or direct child task name.
    pub(super) target: String,
    /// Message or follow-up task to deliver.
    pub(super) message: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentTargetInput {
    /// Agent UUID, canonical path, or direct child task name.
    target: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListAgentsInput {
    /// Optional canonical path prefix.
    #[serde(default)]
    pub(super) path_prefix: Option<String>,
}

fn agent_control_policy(target: &str) -> ToolExecutionPolicy {
    let resource_keys = if target.trim().is_empty() {
        vec!["*".to_string()]
    } else {
        vec![tool_resource_key("agent", target)]
    };
    ToolExecutionPolicy {
        read_only: false,
        idempotent: false,
        parallel_safe: true,
        side_effect: ToolSideEffect::ControlPlane,
        resource_keys,
    }
}

pub struct SendAgentMessageTool;

#[async_trait]
impl TypedTool for SendAgentMessageTool {
    type Input = AgentTargetMessageInput;

    fn name(&self) -> &str {
        "send_message"
    }

    fn description(&self) -> &str {
        "Queue a message for any visible agent in the current task tree. This does not start a new turn when the target is idle."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        agent_control_policy(&input.target)
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let collaboration = collaboration_runtime(&ctx)?;
        let target = input.target.trim();
        anyhow::ensure!(!target.is_empty(), "target must be a non-empty string");
        let message = input.message.trim().to_string();
        anyhow::ensure!(!message.is_empty(), "message must be a non-empty string");
        let delivery = collaboration
            .send_message(target, message, Some(call_id))
            .await?;
        Ok(ToolResult {
            call_id,
            output: serde_json::to_string(&delivery)?,
            content: vec![ModelContentPart::json(serde_json::to_value(&delivery)?)],
            metadata: json!({
                "toolName": "send_message",
                "messageId": delivery.id,
                "targetAgentThreadId": delivery.to_agent_thread_id,
                "sequence": delivery.sequence,
                "queued": true,
                "success": true
            }),
        })
    }
}

impl_typed_tool!(SendAgentMessageTool);

pub struct FollowupAgentTaskTool;

#[async_trait]
impl TypedTool for FollowupAgentTaskTool {
    type Input = AgentTargetMessageInput;

    fn name(&self) -> &str {
        "followup_task"
    }

    fn description(&self) -> &str {
        "Give an existing agent a follow-up task, starting a new turn when it is idle or delivering at the next boundary when it is active."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        agent_control_policy(&input.target)
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let collaboration = collaboration_runtime(&ctx)?;
        let target = input.target.trim();
        anyhow::ensure!(!target.is_empty(), "target must be a non-empty string");
        let message = input.message.trim().to_string();
        anyhow::ensure!(!message.is_empty(), "message must be a non-empty string");
        let turn = collaboration.followup_task(target, message).await?;
        Ok(ToolResult {
            call_id,
            output: serde_json::to_string(&turn)?,
            content: vec![ModelContentPart::json(serde_json::to_value(&turn)?)],
            metadata: json!({
                "toolName": "followup_task",
                "agentThreadId": turn.agent_thread_id,
                "agentTurnId": turn.id,
                "status": turn.status,
                "success": true
            }),
        })
    }
}

impl_typed_tool!(FollowupAgentTaskTool);

pub struct InterruptAgentTool;

#[async_trait]
impl TypedTool for InterruptAgentTool {
    type Input = AgentTargetInput;

    fn name(&self) -> &str {
        "interrupt_agent"
    }

    fn description(&self) -> &str {
        "Interrupt an agent's current turn. The agent identity remains available for a later followup_task."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        agent_control_policy(&input.target)
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let collaboration = collaboration_runtime(&ctx)?;
        let target = input.target.trim();
        anyhow::ensure!(!target.is_empty(), "target must be a non-empty string");
        let turn = collaboration.interrupt_agent(target).await?;
        let value = json!({
            "target": target,
            "turn": turn,
            "interruptRequested": turn.as_ref().is_some_and(|turn| !turn.status.is_terminal())
        });
        Ok(ToolResult {
            call_id,
            output: serde_json::to_string(&value)?,
            content: vec![ModelContentPart::json(value)],
            metadata: json!({
                "toolName": "interrupt_agent",
                "agentTurnId": turn.as_ref().map(|turn| turn.id),
                "success": true
            }),
        })
    }
}

impl_typed_tool!(InterruptAgentTool);

pub struct ListAgentsTool;

#[async_trait]
impl TypedTool for ListAgentsTool {
    type Input = ListAgentsInput;

    fn name(&self) -> &str {
        "list_agents"
    }

    fn description(&self) -> &str {
        "List visible agents in the current root task tree with their canonical paths, profiles, status, and latest task."
    }

    fn execution_policy(&self, _input: &Self::Input) -> ToolExecutionPolicy {
        ToolExecutionPolicy::read_only(vec!["agents:tree".to_string()])
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let collaboration = collaboration_runtime(&ctx)?;
        let agents = collaboration
            .list_agents(input.path_prefix.as_deref())
            .await?;
        let agent_count = agents.len();
        let profiles = AgentProfileRegistry::load(&ctx.workspace_root);
        let value = json!({
            "agents": agents,
            "availableAgentTypes": profiles.list(),
            "profileWarnings": profiles.warnings()
        });
        let output = serde_json::to_string_pretty(&value)?;
        Ok(ToolResult {
            call_id,
            output,
            content: vec![ModelContentPart::json(value)],
            metadata: json!({ "toolName": "list_agents", "count": agent_count, "success": true }),
        })
    }
}

impl_typed_tool!(ListAgentsTool);

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WaitAgentInput {
    /// Optional agent UUID or canonical path.
    #[serde(default)]
    target: Option<String>,
    /// Durable event cursor returned by the previous call.
    #[serde(default)]
    after_cursor: Option<i64>,
    /// How long to block, up to one hour. Zero reads immediately.
    #[serde(default)]
    #[schemars(range(min = 0, max = 3600000))]
    timeout_ms: Option<u64>,
    /// Maximum reasoning tail characters returned.
    #[serde(default)]
    reasoning_tail_chars: Option<usize>,
    /// Maximum characters in each Tool Result projection.
    #[serde(default)]
    tool_result_chars: Option<usize>,
    /// Maximum recent lifecycle events and Tool Results returned.
    #[serde(default)]
    event_limit: Option<usize>,
}

pub struct WaitAgentTool;

#[async_trait]
impl TypedTool for WaitAgentTool {
    type Input = WaitAgentInput;

    fn name(&self) -> &str {
        "wait_agent"
    }

    fn description(&self) -> &str {
        "Read or wait for agent activity derived from reasoning deltas, model/tool lifecycle events, actual tool results, durable turn status, and mailbox messages. A zero timeout reads immediately and never changes the target agent."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        match input.target.as_deref() {
            Some(target) => agent_control_policy(target),
            None => agent_control_policy("mailbox"),
        }
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let collaboration = collaboration_runtime(&ctx)?;
        let timeout_ms = input
            .timeout_ms
            .unwrap_or_default()
            .min(MAX_WAIT_TIMEOUT_MS);
        let outcome = await_cancellable(
            ctx.cancel.as_ref(),
            collaboration.wait_agent(CollaborationWaitAgentRequest {
                target: input.target,
                after_cursor: input.after_cursor,
                timeout: Duration::from_millis(timeout_ms),
                reasoning_tail_chars: input.reasoning_tail_chars.unwrap_or(2_000),
                tool_result_chars: input.tool_result_chars.unwrap_or(4_000),
                event_limit: input.event_limit.unwrap_or(12),
            }),
        )
        .await??;
        let cursor = outcome
            .activity
            .as_ref()
            .map(|activity| activity.cursor)
            .unwrap_or_default();
        let message_count = outcome.messages.len();
        let value = serde_json::to_value(&outcome)?;
        Ok(ToolResult {
            call_id,
            output: serde_json::to_string_pretty(&value)?,
            content: vec![ModelContentPart::json(value.clone())],
            metadata: json!({
                "toolName": "wait_agent",
                "agentThreadId": outcome.agent.id,
                "agentTurnId": outcome.turn.as_ref().map(|turn| turn.id),
                "cursor": cursor,
                "timedOut": outcome.timed_out,
                "messageCount": message_count,
                "success": true
            }),
        })
    }
}

impl_typed_tool!(WaitAgentTool);

/// Longest a wait tool may block.
///
/// Waiting is the cheap way to wait: a blocked tool call burns no tokens, while a
/// short cap forces the model to spend a whole round every time it polls. The cap
/// exists only so a wait cannot outlive any plausible turn, and it matches the
/// ceiling the interactive terminal already allows.
pub(super) const MAX_WAIT_TIMEOUT_MS: u64 = 3_600_000;

/// Runs a future while staying responsive to turn cancellation.
///
/// A long wait is only acceptable if the user can still stop it.
pub(super) async fn await_cancellable<F>(
    cancel: Option<&CancellationToken>,
    future: F,
) -> anyhow::Result<F::Output>
where
    F: std::future::Future,
{
    match cancel {
        Some(token) => {
            tokio::select! {
                value = future => Ok(value),
                _ = token.cancelled() => anyhow::bail!("cancelled"),
            }
        }
        None => Ok(future.await),
    }
}

fn collaboration_runtime(
    ctx: &ToolInvocationContext,
) -> anyhow::Result<&AgentCollaborationInvocation> {
    ctx.collaboration
        .as_ref()
        .context("agent collaboration runtime is unavailable")
}
