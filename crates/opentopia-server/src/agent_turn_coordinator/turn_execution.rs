use super::AgentTurnCoordinator;
use opentopia_core::collaboration::{
    AgentCollaborationInvocation, AgentInvocationIdentity, AgentRunResumeSignal, AgentThreadId,
    AgentTurnId, AgentTurnStatus, CollaborationRegistry, RuntimeSnapshotV1, RuntimeWorkspaceModeV1,
};
use opentopia_core::{
    AgentContinuation, AgentEventPayload, AgentEventSender, AgentProfile, AgentResumeSignal,
    AgentRunConfig, AgentRunIdentity, AgentTurnDriver, AgentTurnInput, AgentTurnOutcome,
    AgentTurnResult, CapabilityProjection, CompiledModelContext, ExecutionAuthority,
    ModelConversationMessage, ModelConversationRole, PreparedAgentRun, ProviderConversationCursor,
    ProviderSettings, SandboxMode, SessionStore, TurnInboxItem,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// The sole server-side entry point into the reusable AgentCore turn kernel.
/// Root and descendant orchestration may differ, but neither owns a second
/// model/tool control loop.
pub(crate) async fn drive_agent_turn(
    agent: &PreparedAgentRun,
    input: AgentTurnInput,
    model_context: Option<CompiledModelContext>,
    sender: Option<AgentEventSender>,
) -> anyhow::Result<AgentTurnResult> {
    let context = agent.prepare_turn(input, model_context)?;
    AgentTurnDriver::run_turn(agent, context, sender).await
}

/// Resume the exact continuation through the same reusable turn kernel.
pub(crate) async fn resume_agent_turn(
    agent: &PreparedAgentRun,
    continuation: AgentContinuation,
    signal: AgentResumeSignal,
    store: Option<Arc<dyn SessionStore>>,
    cancellation: Option<CancellationToken>,
    sender: Option<AgentEventSender>,
) -> anyhow::Result<AgentTurnResult> {
    AgentTurnDriver::resume_turn(agent, continuation, signal, store, cancellation, sender).await
}

impl AgentTurnCoordinator {
    pub(super) async fn execute_start(
        &self,
        agent_thread_id: AgentThreadId,
        agent_turn_id: AgentTurnId,
        cancellation: CancellationToken,
    ) -> anyhow::Result<()> {
        let thread = self.repository.get_thread(agent_thread_id).await?;
        let turn = self.repository.get_turn(agent_turn_id).await?;
        anyhow::ensure!(
            turn.agent_thread_id == thread.id && turn.session_id == thread.session_id,
            "Agent Run command identity is inconsistent"
        );
        if cancellation.is_cancelled() {
            self.finish(
                thread.id,
                turn.id,
                AgentTurnStatus::Cancelled,
                json!({
                    "status": "cancelled",
                    "reason": "cancelled before admission",
                    "agentPath": thread.path,
                }),
            )?;
            return Ok(());
        }
        self.repository
            .transition_turn(turn.id, AgentTurnStatus::Running)
            .await?;
        self.activity.notify(thread.id);

        let session = self.repository.get_session(thread.session_id).await?;
        let _user_thread = self
            .store
            .get_thread(session.user_task_id)?
            .ok_or_else(|| anyhow::anyhow!("user task thread no longer exists"))?;
        let snapshot = self
            .repository
            .get_runtime_snapshot(thread.runtime_snapshot_id)
            .await?
            .decode()?;
        let mut settings = self
            .settings
            .read()
            .expect("settings lock poisoned")
            .clone();
        let provider: ProviderSettings = serde_json::from_value(
            snapshot
                .provider
                .clone()
                .ok_or_else(|| anyhow::anyhow!("runtime snapshot is missing provider"))?,
        )?;
        settings.active_provider_id = provider.id.clone();
        settings.providers = vec![provider];
        settings.permission_mode = serde_json::from_value(
            snapshot
                .permission_mode
                .clone()
                .ok_or_else(|| anyhow::anyhow!("runtime snapshot is missing permissionMode"))?,
        )?;
        settings.sandbox = serde_json::from_value(
            snapshot
                .sandbox
                .clone()
                .ok_or_else(|| anyhow::anyhow!("runtime snapshot is missing sandbox"))?,
        )?;
        if let Some(agent_runtime) = snapshot.agent_runtime.clone() {
            settings.agent_runtime = serde_json::from_value(agent_runtime)?;
        }
        let projection: CapabilityProjection =
            serde_json::from_value(snapshot.capability_projection.clone().ok_or_else(|| {
                anyhow::anyhow!("runtime snapshot is missing capabilityProjection")
            })?)?;
        let workspace_root = self.prepare_workspace(&snapshot, &settings).await?;
        let sandbox = if snapshot.workspace_mode == RuntimeWorkspaceModeV1::SharedReadOnly {
            settings
                .sandbox
                .to_local_sandbox_config()
                .with_sandbox_mode(SandboxMode::ReadOnly)
        } else {
            settings.sandbox.to_local_sandbox_config()
        };
        let profile = frozen_agent_profile(&snapshot, &thread.agent_type)?;
        let authority = ExecutionAuthority::new(
            workspace_root.clone(),
            settings.permission_mode,
            sandbox,
            projection,
        )?;
        let config = AgentRunConfig::from_settings(
            &settings,
            None,
            authority,
            AgentRunIdentity::new(turn.id.as_uuid(), turn.invocation_id, thread.path.clone()),
        )
        .with_profile(profile);
        let mut agent = self
            .base_agent
            .read()
            .expect("agent lock poisoned")
            .begin_run(config)?;
        agent.set_mcp_host(self.mcp_host.clone());
        if agent.capability_projection().allow_all_plugins
            || !agent.capability_projection().plugins.is_empty()
        {
            crate::sync_thread_bundled_plugin_activations(
                &self.store,
                session.user_task_id,
                &mut agent,
            );
        } else {
            agent.disable_all_bundled_plugins();
        }
        crate::sync_thread_attachment_tool_preloads(&self.store, session.user_task_id, &mut agent);
        crate::thread_runtime::sync_runtime_connection_tools(
            &self.store,
            &self.mcp_host,
            session.user_task_id,
            &snapshot.connection_authority,
            &mut agent,
        )
        .await?;
        if !snapshot.tools.is_empty() {
            agent.restrict_to_tools(snapshot.tools.iter().map(String::as_str));
        }

        let identity = AgentInvocationIdentity {
            session_id: thread.session_id,
            agent_thread_id: thread.id,
            agent_turn_id: turn.id,
            runtime_snapshot_id: thread.runtime_snapshot_id,
        };
        let invocation = AgentCollaborationInvocation::new(
            self.collaboration.clone(),
            self.activity.clone(),
            self.snapshot_deriver.clone(),
            identity,
        );
        agent.set_agent_collaboration(invocation.clone());
        for message in invocation.pending_messages(256).await? {
            self.turn_inbox
                .push(turn.id.as_uuid(), TurnInboxItem::AgentMessage { message });
        }

        let mut conversation = self.load_conversation(&thread, &snapshot).await?;
        let user_entry = ModelConversationMessage {
            role: ModelConversationRole::User,
            content: turn.task_message.clone(),
            content_parts: Vec::new(),
            tool_calls: Vec::new(),
            tool_results: Vec::new(),
        };
        self.repository.append_ledger_item(
            turn.session_id,
            thread.id,
            turn.id,
            "conversation",
            &serde_json::to_value(&user_entry)?,
        )?;

        let provider = settings.active_provider().clone();
        let provider_cursor = self
            .repository
            .load_provider_state(thread.id, &provider.id)?
            .and_then(|(model, _, value)| {
                (model == provider.model)
                    .then(|| serde_json::from_value::<ProviderConversationCursor>(value).ok())
                    .flatten()
            });
        let input = AgentTurnInput {
            thread_id: session.user_task_id,
            user_message_id: turn.id.as_uuid(),
            workspace_root,
            content: turn.task_message.clone(),
            user_content: Vec::new(),
            context_summary: None,
            conversation: conversation.clone(),
            permission_mode: settings.permission_mode,
            context_budget: None,
            provider_cursor,
            store: Some(self.store.clone()),
            cancellation: Some(cancellation),
        };
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let sink = self.clone();
        let sink_thread = thread.clone();
        let sink_turn = turn.clone();
        let user_task_id = session.user_task_id;
        let event_task = tokio::spawn(async move {
            while let Some(payload) = receiver.recv().await {
                if let Err(error) =
                    sink.record_event(user_task_id, &sink_thread, &sink_turn, payload)
                {
                    tracing::error!(?error, "failed to persist Agent activity event");
                }
            }
        });
        let agent = agent.finalize()?;
        let result = drive_agent_turn(&agent, input, None, Some(sender)).await;
        let _ = event_task.await;

        match result {
            Ok(result) => {
                match &result.outcome {
                    AgentTurnOutcome::Suspended {
                        approval_id,
                        continuation,
                    } => {
                        let value =
                            crate::encode_turn_checkpoint(&self.store, "approval", continuation)?;
                        self.store.put_approval_continuation(
                            *approval_id,
                            session.user_task_id,
                            value,
                        )?;
                    }
                    AgentTurnOutcome::AwaitingInput {
                        request,
                        continuation,
                    } => {
                        let value =
                            crate::encode_turn_checkpoint(&self.store, "user_input", continuation)?;
                        self.store
                            .put_user_input_request(session.user_task_id, request, value)?;
                    }
                    AgentTurnOutcome::WaitingUserAction { .. }
                    | AgentTurnOutcome::Completed
                    | AgentTurnOutcome::Cancelled { .. }
                    | AgentTurnOutcome::Partial { .. }
                    | AgentTurnOutcome::Blocked { .. }
                    | AgentTurnOutcome::Stopped { .. } => {}
                }
                if let Some(cursor) = result.provider_cursor.as_ref() {
                    self.repository.save_provider_state(
                        thread.id,
                        &provider.id,
                        &provider.model,
                        &cursor.response_id,
                        &cursor.compatibility_hash,
                        &serde_json::to_value(cursor)?,
                    )?;
                }
                let result_text = crate::agent_result_text(&result.events);
                let assistant_entry = ModelConversationMessage {
                    role: ModelConversationRole::Assistant,
                    content: result_text.clone(),
                    content_parts: Vec::new(),
                    tool_calls: Vec::new(),
                    tool_results: Vec::new(),
                };
                self.repository.append_ledger_item(
                    turn.session_id,
                    thread.id,
                    turn.id,
                    "conversation",
                    &serde_json::to_value(&assistant_entry)?,
                )?;
                conversation.push(user_entry);
                conversation.push(assistant_entry);
                let (status, mut payload, checkpoint) =
                    outcome_payload(&thread.path.to_string(), result.outcome, result_text)?;
                attach_workspace_delivery(&mut payload, &snapshot);
                if let Some((kind, continuation)) = checkpoint {
                    self.repository
                        .put_turn_checkpoint(turn.id, kind, &continuation)?;
                } else {
                    self.repository.delete_turn_checkpoint(turn.id)?;
                }
                self.finish(thread.id, turn.id, status, payload)?;
            }
            Err(error) => {
                self.record_event(
                    session.user_task_id,
                    &thread,
                    &turn,
                    AgentEventPayload::Error {
                        message: error.to_string(),
                    },
                )?;
                let mut payload = json!({
                    "status": "failed",
                    "agentPath": thread.path,
                    "error": error.to_string(),
                });
                attach_workspace_delivery(&mut payload, &snapshot);
                self.finish(thread.id, turn.id, AgentTurnStatus::Failed, payload)?;
            }
        }
        Ok(())
    }

    pub(super) async fn execute_resume(
        &self,
        agent_thread_id: AgentThreadId,
        agent_turn_id: AgentTurnId,
        signal: AgentRunResumeSignal,
        cancellation: CancellationToken,
    ) -> anyhow::Result<()> {
        let thread = self.repository.get_thread(agent_thread_id).await?;
        let turn = self.repository.resume_turn(agent_turn_id)?;
        anyhow::ensure!(turn.agent_thread_id == thread.id, "resume target mismatch");
        let (_, checkpoint) = self
            .repository
            .get_turn_checkpoint(turn.id)?
            .ok_or_else(|| anyhow::anyhow!("Agent Turn checkpoint is missing"))?;
        let continuation: opentopia_core::AgentContinuation = serde_json::from_value(checkpoint)?;
        let session = self.repository.get_session(thread.session_id).await?;
        let snapshot = self
            .repository
            .get_runtime_snapshot(thread.runtime_snapshot_id)
            .await?
            .decode()?;
        let mut settings = self
            .settings
            .read()
            .expect("settings lock poisoned")
            .clone();
        let frozen_provider: ProviderSettings = serde_json::from_value(
            snapshot
                .provider
                .clone()
                .ok_or_else(|| anyhow::anyhow!("runtime snapshot is missing provider"))?,
        )?;
        settings.active_provider_id = frozen_provider.id.clone();
        settings.providers = vec![frozen_provider];
        settings.permission_mode = serde_json::from_value(
            snapshot
                .permission_mode
                .clone()
                .ok_or_else(|| anyhow::anyhow!("runtime snapshot is missing permissionMode"))?,
        )?;
        settings.sandbox = serde_json::from_value(
            snapshot
                .sandbox
                .clone()
                .ok_or_else(|| anyhow::anyhow!("runtime snapshot is missing sandbox"))?,
        )?;
        if let Some(agent_runtime) = snapshot.agent_runtime.clone() {
            settings.agent_runtime = serde_json::from_value(agent_runtime)?;
        }
        let snapshot_projection: CapabilityProjection =
            serde_json::from_value(snapshot.capability_projection.clone().ok_or_else(|| {
                anyhow::anyhow!("runtime snapshot is missing capabilityProjection")
            })?)?;
        let authority = continuation
            .execution_authority
            .clone()
            .ok_or_else(|| anyhow::anyhow!("continuation is missing its execution authority"))?;
        let profile = frozen_agent_profile(&snapshot, &thread.agent_type)?;
        let mut expected_sandbox =
            if snapshot.workspace_mode == RuntimeWorkspaceModeV1::SharedReadOnly {
                settings
                    .sandbox
                    .to_local_sandbox_config()
                    .with_sandbox_mode(SandboxMode::ReadOnly)
            } else {
                settings.sandbox.to_local_sandbox_config()
            };
        if let Some(mode) = profile.sandbox_mode {
            if mode.is_attenuation_of(expected_sandbox.sandbox_mode) {
                expected_sandbox = expected_sandbox.with_sandbox_mode(mode);
            }
        }
        anyhow::ensure!(
            authority.workspace_root() == snapshot.workspace_assignment.root(),
            "continuation workspace does not match the runtime snapshot"
        );
        anyhow::ensure!(
            authority.permission_mode() == settings.permission_mode,
            "continuation permission mode does not match the runtime snapshot"
        );
        anyhow::ensure!(
            authority.capability_projection() == &snapshot_projection,
            "continuation capability projection does not match the runtime snapshot"
        );
        anyhow::ensure!(
            authority.sandbox_config() == &expected_sandbox,
            "continuation sandbox does not match the runtime snapshot"
        );
        let provider = settings.active_provider().clone();
        let config = AgentRunConfig::from_settings(
            &settings,
            None,
            authority,
            AgentRunIdentity::new(turn.id.as_uuid(), turn.invocation_id, thread.path.clone()),
        )
        .with_profile(profile);
        let mut agent = self
            .base_agent
            .read()
            .expect("agent lock poisoned")
            .begin_run(config)?;
        agent.set_mcp_host(self.mcp_host.clone());
        if agent.capability_projection().allow_all_plugins
            || !agent.capability_projection().plugins.is_empty()
        {
            crate::sync_thread_bundled_plugin_activations(
                &self.store,
                session.user_task_id,
                &mut agent,
            );
        } else {
            agent.disable_all_bundled_plugins();
        }
        crate::sync_thread_attachment_tool_preloads(&self.store, session.user_task_id, &mut agent);
        crate::thread_runtime::sync_runtime_connection_tools(
            &self.store,
            &self.mcp_host,
            session.user_task_id,
            &snapshot.connection_authority,
            &mut agent,
        )
        .await?;
        if !snapshot.tools.is_empty() {
            agent.restrict_to_tools(snapshot.tools.iter().map(String::as_str));
        }
        let invocation = AgentCollaborationInvocation::new(
            self.collaboration.clone(),
            self.activity.clone(),
            self.snapshot_deriver.clone(),
            AgentInvocationIdentity {
                session_id: thread.session_id,
                agent_thread_id: thread.id,
                agent_turn_id: turn.id,
                runtime_snapshot_id: thread.runtime_snapshot_id,
            },
        );
        agent.set_agent_collaboration(invocation.clone());
        for message in invocation.pending_messages(256).await? {
            self.turn_inbox
                .push(turn.id.as_uuid(), TurnInboxItem::AgentMessage { message });
        }

        let (sender, mut receiver) = mpsc::unbounded_channel();
        let sink = self.clone();
        let sink_thread = thread.clone();
        let sink_turn = turn.clone();
        let user_task_id = session.user_task_id;
        let event_task = tokio::spawn(async move {
            while let Some(payload) = receiver.recv().await {
                if let Err(error) =
                    sink.record_event(user_task_id, &sink_thread, &sink_turn, payload)
                {
                    tracing::error!(?error, "failed to persist resumed Agent activity event");
                }
            }
        });
        let agent = agent.finalize()?;
        let result = resume_agent_turn(
            &agent,
            continuation,
            signal.into(),
            Some(self.store.clone()),
            Some(cancellation),
            Some(sender),
        )
        .await;
        let _ = event_task.await;
        match result {
            Ok(result) => {
                if let Some(cursor) = result.provider_cursor.as_ref() {
                    self.repository.save_provider_state(
                        thread.id,
                        &provider.id,
                        &provider.model,
                        &cursor.response_id,
                        &cursor.compatibility_hash,
                        &serde_json::to_value(cursor)?,
                    )?;
                }
                match &result.outcome {
                    AgentTurnOutcome::Suspended {
                        approval_id,
                        continuation,
                    } => {
                        let value =
                            crate::encode_turn_checkpoint(&self.store, "approval", continuation)?;
                        self.store.put_approval_continuation(
                            *approval_id,
                            session.user_task_id,
                            value,
                        )?;
                    }
                    AgentTurnOutcome::AwaitingInput {
                        request,
                        continuation,
                    } => {
                        let value =
                            crate::encode_turn_checkpoint(&self.store, "user_input", continuation)?;
                        self.store
                            .put_user_input_request(session.user_task_id, request, value)?;
                    }
                    _ => {}
                }
                let result_text = crate::agent_result_text(&result.events);
                let assistant_entry = ModelConversationMessage {
                    role: ModelConversationRole::Assistant,
                    content: result_text.clone(),
                    content_parts: Vec::new(),
                    tool_calls: Vec::new(),
                    tool_results: Vec::new(),
                };
                self.repository.append_ledger_item(
                    turn.session_id,
                    thread.id,
                    turn.id,
                    "conversation",
                    &serde_json::to_value(&assistant_entry)?,
                )?;
                let (status, mut payload, checkpoint) =
                    outcome_payload(&thread.path.to_string(), result.outcome, result_text)?;
                attach_workspace_delivery(&mut payload, &snapshot);
                if let Some((kind, continuation)) = checkpoint {
                    self.repository
                        .put_turn_checkpoint(turn.id, kind, &continuation)?;
                } else {
                    self.repository.delete_turn_checkpoint(turn.id)?;
                }
                self.finish(thread.id, turn.id, status, payload)?;
            }
            Err(error) => {
                self.record_event(
                    session.user_task_id,
                    &thread,
                    &turn,
                    AgentEventPayload::Error {
                        message: error.to_string(),
                    },
                )?;
                let mut payload = json!({
                    "status": "failed",
                    "agentPath": thread.path,
                    "error": error.to_string(),
                });
                attach_workspace_delivery(&mut payload, &snapshot);
                self.finish(thread.id, turn.id, AgentTurnStatus::Failed, payload)?;
            }
        }
        Ok(())
    }
}

fn frozen_agent_profile(
    snapshot: &RuntimeSnapshotV1,
    agent_type: &str,
) -> anyhow::Result<AgentProfile> {
    snapshot
        .agent_profiles
        .iter()
        .filter_map(|value| serde_json::from_value::<AgentProfile>(value.clone()).ok())
        .find(|profile| profile.name == agent_type)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "agent profile `{agent_type}` is absent from the frozen runtime snapshot"
            )
        })
}

fn attach_workspace_delivery(payload: &mut Value, snapshot: &RuntimeSnapshotV1) {
    let Ok(mut assignment) = serde_json::to_value(&snapshot.workspace_assignment) else {
        return;
    };
    if let Some(object) = assignment.as_object_mut() {
        object.insert(
            "deliveryState".to_string(),
            Value::String("ready".to_string()),
        );
    }
    if let Some(object) = payload.as_object_mut() {
        object.insert("workspaceAssignment".to_string(), assignment);
    }
}

fn outcome_payload(
    path: &str,
    outcome: AgentTurnOutcome,
    result: String,
) -> anyhow::Result<(AgentTurnStatus, Value, Option<(&'static str, Value)>)> {
    let (status, kind, reason, checkpoint) = match outcome {
        AgentTurnOutcome::Completed => (AgentTurnStatus::Completed, "completed", None, None),
        AgentTurnOutcome::Cancelled { reason } => {
            (AgentTurnStatus::Cancelled, "cancelled", Some(reason), None)
        }
        AgentTurnOutcome::Partial { reason } => {
            (AgentTurnStatus::Completed, "partial", Some(reason), None)
        }
        AgentTurnOutcome::Blocked { reason } => {
            (AgentTurnStatus::Completed, "blocked", Some(reason), None)
        }
        AgentTurnOutcome::Stopped { reason } => {
            (AgentTurnStatus::Failed, "stopped", Some(reason), None)
        }
        AgentTurnOutcome::Suspended { continuation, .. } => (
            AgentTurnStatus::WaitingApproval,
            "waiting_approval",
            None,
            Some(("approval", serde_json::to_value(continuation)?)),
        ),
        AgentTurnOutcome::AwaitingInput { continuation, .. } => (
            AgentTurnStatus::WaitingInput,
            "waiting_input",
            None,
            Some(("user_input", serde_json::to_value(continuation)?)),
        ),
        AgentTurnOutcome::WaitingUserAction {
            reason,
            continuation,
            ..
        } => (
            AgentTurnStatus::WaitingAction,
            "waiting_action",
            Some(reason),
            Some(("external_action", serde_json::to_value(continuation)?)),
        ),
    };
    Ok((
        status,
        json!({
            "status": kind,
            "agentPath": path,
            "result": result,
            "reason": reason,
            "checkpoint": checkpoint,
        }),
        checkpoint,
    ))
}

#[cfg(test)]
mod tests {
    use super::outcome_payload;
    use opentopia_core::collaboration::AgentTurnStatus;
    use opentopia_core::AgentTurnOutcome;

    #[test]
    fn completed_outcome_projects_terminal_payload_without_checkpoint() {
        let (status, payload, checkpoint) = outcome_payload(
            "/root/reviewer",
            AgentTurnOutcome::Completed,
            "finished".to_string(),
        )
        .expect("project completed outcome");

        assert_eq!(status, AgentTurnStatus::Completed);
        assert_eq!(payload["status"], "completed");
        assert_eq!(payload["agentPath"], "/root/reviewer");
        assert_eq!(payload["result"], "finished");
        assert!(payload["reason"].is_null());
        assert!(payload["checkpoint"].is_null());
        assert!(checkpoint.is_none());
    }

    #[test]
    fn cancelled_outcome_preserves_reason_and_terminal_status() {
        let (status, payload, checkpoint) = outcome_payload(
            "/root/reviewer",
            AgentTurnOutcome::Cancelled {
                reason: "interrupted".to_string(),
            },
            String::new(),
        )
        .expect("project cancelled outcome");

        assert_eq!(status, AgentTurnStatus::Cancelled);
        assert_eq!(payload["status"], "cancelled");
        assert_eq!(payload["reason"], "interrupted");
        assert!(checkpoint.is_none());
    }
}
