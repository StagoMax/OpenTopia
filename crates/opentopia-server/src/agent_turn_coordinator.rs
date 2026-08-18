mod turn_execution;

pub(crate) use turn_execution::{drive_agent_turn, resume_agent_turn};

use crate::agent_runs::AgentRunExecutor;
use async_trait::async_trait;
use opentopia_core::collaboration::{
    AgentCollaborationRuntime, AgentMailboxNotifier, AgentRunCommand, AgentThreadId, AgentTurnId,
    AgentTurnStatus, CollaborationRegistry, RuntimeForkTurnsLabelV1, RuntimeForkTurnsV1,
    RuntimeSnapshotDeriver, RuntimeSnapshotV1, RuntimeWorkspaceAssignmentV1,
    RuntimeWorkspaceModeV1, SqliteAgentActivitySource, SqliteCollaborationRepository,
};
use opentopia_core::{
    execute_git_workflow, isolated_agent_worktree_request, AgentCore, AgentEventPayload,
    AppSettings, ExecutionContext, LocalExecutionEnvironment, McpExtensionHost,
    ModelConversationMessage, ModelConversationRole, ResourceLimit, SessionStore,
    SqliteSessionStore, TurnInbox,
};
use serde_json::{json, Value};
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct AgentTurnCoordinator {
    store: Arc<SqliteSessionStore>,
    repository: Arc<SqliteCollaborationRepository>,
    activity: Arc<SqliteAgentActivitySource>,
    collaboration: AgentCollaborationRuntime,
    snapshot_deriver: Arc<dyn RuntimeSnapshotDeriver>,
    mailbox_notifier: Arc<dyn AgentMailboxNotifier>,
    base_agent: Arc<RwLock<AgentCore>>,
    settings: Arc<RwLock<AppSettings>>,
    mcp_host: McpExtensionHost,
    turn_inbox: Arc<dyn TurnInbox>,
}

impl AgentTurnCoordinator {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store: Arc<SqliteSessionStore>,
        repository: Arc<SqliteCollaborationRepository>,
        activity: Arc<SqliteAgentActivitySource>,
        collaboration: AgentCollaborationRuntime,
        snapshot_deriver: Arc<dyn RuntimeSnapshotDeriver>,
        mailbox_notifier: Arc<dyn AgentMailboxNotifier>,
        base_agent: Arc<RwLock<AgentCore>>,
        settings: Arc<RwLock<AppSettings>>,
        mcp_host: McpExtensionHost,
        turn_inbox: Arc<dyn TurnInbox>,
    ) -> Self {
        Self {
            store,
            repository,
            activity,
            collaboration,
            snapshot_deriver,
            mailbox_notifier,
            base_agent,
            settings,
            mcp_host,
            turn_inbox,
        }
    }

    async fn load_conversation(
        &self,
        thread: &opentopia_core::collaboration::AgentThreadRecord,
        snapshot: &RuntimeSnapshotV1,
    ) -> anyhow::Result<Vec<ModelConversationMessage>> {
        let stored = self
            .repository
            .list_ledger_items(thread.id, "conversation")?;
        if !stored.is_empty() {
            return stored
                .into_iter()
                .map(|value| serde_json::from_value(value).map_err(Into::into))
                .collect();
        }
        if snapshot.fork_turns == RuntimeForkTurnsV1::Label(RuntimeForkTurnsLabelV1::None) {
            return Ok(Vec::new());
        }
        let Some(parent_id) = thread.parent_agent_thread_id else {
            return Ok(Vec::new());
        };
        let parent = self.repository.get_thread(parent_id).await?;
        if parent.path.as_str() == "/root" {
            let session = self.repository.get_session(thread.session_id).await?;
            let messages = self.store.list_messages(session.user_task_id)?;
            return Ok(apply_fork_window(
                crate::project_model_conversation(&messages, &[]),
                &snapshot.fork_turns,
            ));
        }
        let conversation = self
            .repository
            .list_ledger_items(parent.id, "conversation")?
            .into_iter()
            .map(|value| serde_json::from_value(value).map_err(Into::into))
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(apply_fork_window(conversation, &snapshot.fork_turns))
    }

    async fn prepare_workspace(
        &self,
        snapshot: &RuntimeSnapshotV1,
        settings: &AppSettings,
    ) -> anyhow::Result<PathBuf> {
        let root = snapshot.workspace_root.clone();
        if snapshot.workspace_mode != RuntimeWorkspaceModeV1::IsolatedWorktree || root.exists() {
            return Ok(root);
        }
        let RuntimeWorkspaceAssignmentV1::IsolatedWorktree {
            repository_root: repository,
            branch,
            base_commit,
            ..
        } = &snapshot.workspace_assignment
        else {
            anyhow::bail!("isolated workspace assignment is missing");
        };
        let repository = repository.clone();
        if let Some(parent) = root.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut sandbox = settings.sandbox.to_local_sandbox_config();
        sandbox.grant_write_path(repository.join(".git"));
        if let Some(parent) = root.parent() {
            sandbox.grant_write_path(parent.to_path_buf());
        }
        let environment =
            LocalExecutionEnvironment::with_sandbox_config(repository.clone(), sandbox);
        let request = isolated_agent_worktree_request(
            repository,
            root.clone(),
            branch.clone(),
            base_commit.clone(),
        )?;
        let result = execute_git_workflow(
            &environment,
            &request,
            ExecutionContext::with_timeout(std::time::Duration::from_secs(120))
                .with_resource_limits(ResourceLimit {
                    max_output_bytes: Some(256 * 1024),
                    ..ResourceLimit::default()
                }),
        )
        .await?;
        anyhow::ensure!(
            result.success,
            "isolated worktree creation failed: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        );
        Ok(root)
    }

    fn record_event(
        &self,
        user_task_id: uuid::Uuid,
        thread: &opentopia_core::collaboration::AgentThreadRecord,
        turn: &opentopia_core::collaboration::AgentTurnRecord,
        payload: AgentEventPayload,
    ) -> anyhow::Result<()> {
        if let AgentEventPayload::ApprovalRequested {
            approval_id,
            action,
            reason,
        } = &payload
        {
            self.store
                .insert_approval(opentopia_core::Approval::pending(
                    *approval_id,
                    user_task_id,
                    action.clone(),
                    reason.clone(),
                ))?;
        }
        self.repository.append_activity_event(
            thread.session_id,
            thread.id,
            turn.id,
            turn.invocation_id,
            payload,
            None,
        )?;
        self.activity.notify(thread.id);
        Ok(())
    }

    fn finish(
        &self,
        agent_thread_id: AgentThreadId,
        agent_turn_id: AgentTurnId,
        status: AgentTurnStatus,
        payload: Value,
    ) -> anyhow::Result<()> {
        if let Some(message) = self
            .repository
            .record_turn_state(agent_turn_id, status, &payload)?
        {
            self.mailbox_notifier.message_enqueued(&message);
        }
        self.activity.notify(agent_thread_id);
        Ok(())
    }
}

fn apply_fork_window(
    conversation: Vec<ModelConversationMessage>,
    fork: &RuntimeForkTurnsV1,
) -> Vec<ModelConversationMessage> {
    let RuntimeForkTurnsV1::Count { count } = fork else {
        return conversation;
    };
    if *count == 0 {
        return Vec::new();
    }
    let user_starts = conversation
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            (message.role == ModelConversationRole::User).then_some(index)
        })
        .collect::<Vec<_>>();
    let start = user_starts
        .len()
        .checked_sub(*count)
        .and_then(|index| user_starts.get(index).copied())
        .unwrap_or(0);
    conversation.into_iter().skip(start).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_runs::ServerAgentRunScheduler;
    use opentopia_core::collaboration::{
        AgentCollaborationInvocation, AgentInvocationIdentity, AgentMailbox, AgentMailboxMessage,
        AgentMailboxMessageKind, AgentRunResumeSignal, AgentRunScheduler, AgentSpawnPolicy,
        AgentThreadRecord, AgentTurnRecord, CollaborationSessionId, CollaborationSessionPolicy,
        CreateCollaborationSession, RuntimeSnapshotSeed,
    };
    use opentopia_core::{
        AgentProfileRegistry, ApprovalStatus, BufferedTurnInbox, CapabilityProjection,
        ExecutionAuthority, LocalSandboxConfig, ModelConversationMessage, ModelConversationRole,
        OpenAiCompatibilityReport, PermissionMode, ProviderAdapterKind, ProviderAuthKind,
        ProviderKind, ProviderSettings, ProviderTransportKind, ToolCall, ToolInvocationContext,
        ToolRegistry,
    };
    use serde_json::json;
    use std::process::Command;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::Semaphore;
    use tokio::task::JoinHandle;
    use tokio::time::{sleep, timeout};
    use uuid::Uuid;

    struct ScriptedChatServer {
        base_url: String,
        task: JoinHandle<()>,
    }

    impl ScriptedChatServer {
        async fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind scripted provider");
            let address = listener.local_addr().expect("scripted provider address");
            let task = tokio::spawn(async move {
                while let Ok((mut socket, _)) = listener.accept().await {
                    tokio::spawn(async move {
                        let request = read_http_request(&mut socket).await;
                        let body = if request.contains("coordinate nested work") {
                            if request.contains("wait_leaf_call") {
                                chat_text_stream("lead integrated the completed leaf")
                            } else {
                                chat_tool_stream(
                                    "wait_leaf_call",
                                    "wait_agent",
                                    r#"{"target":"leaf","timeout_ms":5000}"#,
                                )
                            }
                        } else if request.contains("perform nested leaf work") {
                            chat_text_stream("leaf completed its nested work")
                        } else {
                            chat_text_stream("scripted task completed")
                        };
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        socket
                            .write_all(response.as_bytes())
                            .await
                            .expect("write scripted provider response");
                        socket.shutdown().await.expect("close scripted provider");
                    });
                }
            });
            Self {
                base_url: format!("http://{address}/v1"),
                task,
            }
        }

        fn provider_settings(&self) -> ProviderSettings {
            openai_test_provider(&self.base_url, "multi-agent-e2e-scripted")
        }
    }

    impl Drop for ScriptedChatServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    struct BlockingChatServer {
        base_url: String,
        started: Arc<Semaphore>,
        task: JoinHandle<()>,
    }

    impl BlockingChatServer {
        async fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind blocking provider");
            let address = listener.local_addr().expect("blocking provider address");
            let started = Arc::new(Semaphore::new(0));
            let server_started = started.clone();
            let task = tokio::spawn(async move {
                while let Ok((mut socket, _)) = listener.accept().await {
                    let _ = read_http_request(&mut socket).await;
                    server_started.add_permits(1);
                    sleep(Duration::from_secs(30)).await;
                    let body = chat_text_stream("response should have been cancelled");
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.shutdown().await;
                }
            });
            Self {
                base_url: format!("http://{address}/v1"),
                started,
                task,
            }
        }

        fn provider_settings(&self) -> ProviderSettings {
            openai_test_provider(&self.base_url, "multi-agent-e2e-blocking")
        }

        async fn wait_until_requested(&self) {
            timeout(Duration::from_secs(5), self.started.acquire())
                .await
                .expect("blocking provider received a request")
                .expect("blocking provider semaphore open")
                .forget();
        }
    }

    impl Drop for BlockingChatServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    struct RejectingChatServer {
        base_url: String,
        task: JoinHandle<()>,
    }

    impl RejectingChatServer {
        async fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind rejecting provider");
            let address = listener.local_addr().expect("rejecting provider address");
            let task = tokio::spawn(async move {
                while let Ok((mut socket, _)) = listener.accept().await {
                    let _ = read_http_request(&mut socket).await;
                    let body = r#"{"error":{"message":"scripted provider rejection"}}"#;
                    let response = format!(
                        "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.shutdown().await;
                }
            });
            Self {
                base_url: format!("http://{address}/v1"),
                task,
            }
        }

        fn provider_settings(&self) -> ProviderSettings {
            openai_test_provider(&self.base_url, "multi-agent-e2e-rejecting")
        }
    }

    impl Drop for RejectingChatServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    struct ApprovalChatServer {
        base_url: String,
        task: JoinHandle<()>,
    }

    impl ApprovalChatServer {
        async fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind approval provider");
            let address = listener.local_addr().expect("approval provider address");
            let task = tokio::spawn(async move {
                while let Ok((mut socket, _)) = listener.accept().await {
                    let request = read_http_request(&mut socket).await;
                    let body = if request.contains("call_approval_shell") {
                        chat_text_stream("approval decision was applied and the turn resumed")
                    } else {
                        chat_tool_stream(
                            "call_approval_shell",
                            "shell",
                            r#"{"command":"git reset --hard HEAD~1"}"#,
                        )
                    };
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.shutdown().await;
                }
            });
            Self {
                base_url: format!("http://{address}/v1"),
                task,
            }
        }

        fn provider_settings(&self) -> ProviderSettings {
            openai_test_provider(&self.base_url, "multi-agent-e2e-approval")
        }
    }

    impl Drop for ApprovalChatServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    fn openai_test_provider(base_url: &str, id: &str) -> ProviderSettings {
        let model = id;
        let mut provider = ProviderSettings::default();
        provider.id = id.to_string();
        provider.name = "Multi-agent E2E local provider".to_string();
        provider.apply_legacy_kind_preset(ProviderKind::OpenAiCompatible);
        provider.transport = Some(ProviderTransportKind::Http);
        provider.auth = Some(ProviderAuthKind::None);
        provider.allowed_adapters = vec![ProviderAdapterKind::OpenAiChat];
        provider.preferred_adapter = Some(ProviderAdapterKind::OpenAiChat);
        provider.base_url = base_url.to_string();
        provider.model = model.to_string();
        provider.api_key_configured = true;
        let report: OpenAiCompatibilityReport = serde_json::from_value(json!({
            "baseUrl": base_url,
            "model": model,
            "selectedProtocol": "chat_completions",
            "chatCompletions": "supported",
            "chatFunctionTools": "supported",
            "chatStreamingTools": "supported",
            "chatParallelToolCalls": "supported",
            "responses": "unsupported",
            "developerMessages": "supported",
            "messageCompatibility": false,
            "checkedAt": "2026-08-17T00:00:00Z"
        }))
        .expect("local provider compatibility report");
        provider.apply_openai_compatibility_report(report);
        provider
    }

    fn chat_text_stream(text: &str) -> String {
        format!(
            "data: {}\n\ndata: [DONE]\n\n",
            json!({
                "choices": [{
                    "delta": { "content": text },
                    "finish_reason": "stop"
                }]
            })
        )
    }

    fn chat_tool_stream(call_id: &str, name: &str, arguments: &str) -> String {
        format!(
            "data: {}\n\ndata: [DONE]\n\n",
            json!({
                "choices": [{
                    "delta": {
                        "tool_calls": [{
                            "index": 0,
                            "id": call_id,
                            "type": "function",
                            "function": {
                                "name": name,
                                "arguments": arguments
                            }
                        }]
                    },
                    "finish_reason": "tool_calls"
                }]
            })
        )
    }

    async fn read_http_request(socket: &mut TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let read = socket
                .read(&mut buffer)
                .await
                .expect("read scripted provider request");
            assert!(read > 0, "provider client closed before request completed");
            bytes.extend_from_slice(&buffer[..read]);
            let Some(headers_end) = find_bytes(&bytes, b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&bytes[..headers_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().expect("content length"))
                })
                .unwrap_or(0);
            if bytes.len() >= headers_end + 4 + content_length {
                return String::from_utf8(bytes).expect("UTF-8 provider request");
            }
        }
    }

    fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }

    fn run_git(repository: &Path, arguments: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(repository)
            .args(arguments)
            .output()
            .expect("execute git command");
        assert!(
            output.status.success(),
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("UTF-8 git output")
            .trim()
            .to_string()
    }

    struct MultiAgentE2eFixture {
        store: Arc<SqliteSessionStore>,
        repository: Arc<SqliteCollaborationRepository>,
        activity: Arc<SqliteAgentActivitySource>,
        runtime: AgentCollaborationRuntime,
        snapshot_deriver: Arc<dyn RuntimeSnapshotDeriver>,
        scheduler: Arc<ServerAgentRunScheduler>,
        coordinator: Arc<AgentTurnCoordinator>,
        user_task_id: Uuid,
        session_id: CollaborationSessionId,
        root: AgentThreadRecord,
        root_turn: AgentTurnRecord,
        workspace: PathBuf,
    }

    impl MultiAgentE2eFixture {
        async fn new() -> Self {
            Self::new_with_provider(None).await
        }

        async fn new_with_provider(provider: Option<ProviderSettings>) -> Self {
            Self::new_with_policy(
                provider,
                CollaborationSessionPolicy {
                    max_agents: 16,
                    max_active_runs: 8,
                    max_depth: 4,
                },
                AgentSpawnPolicy::allows_children(4, 6),
            )
            .await
        }

        async fn new_with_policy(
            provider: Option<ProviderSettings>,
            session_policy: CollaborationSessionPolicy,
            spawn_policy: AgentSpawnPolicy,
        ) -> Self {
            let workspace = std::env::current_dir().expect("current test workspace");
            let store = Arc::new(SqliteSessionStore::open(":memory:").expect("in-memory store"));
            let user_thread = store
                .create_thread(None, workspace.clone())
                .expect("user-visible thread");
            let mut settings = AppSettings::from_env(PermissionMode::FullAccess);
            if let Some(provider) = provider {
                settings.active_provider_id = provider.id.clone();
                settings.providers = vec![provider];
            } else {
                let provider = settings.active_provider_mut();
                provider.apply_legacy_kind_preset(ProviderKind::Mock);
                provider.model = "multi-agent-e2e-mock".to_string();
            }

            let turn_inbox: Arc<dyn TurnInbox> = Arc::new(BufferedTurnInbox::default());
            let base_agent = Arc::new(RwLock::new(
                AgentCore::from_settings(&settings).with_turn_inbox(turn_inbox.clone()),
            ));
            let repository = Arc::new(
                SqliteCollaborationRepository::new(store.clone())
                    .expect("collaboration repository"),
            );
            let activity = Arc::new(SqliteAgentActivitySource::new(repository.clone()));
            let snapshot_deriver: Arc<dyn RuntimeSnapshotDeriver> =
                Arc::new(opentopia_core::collaboration::AttenuatingRuntimeSnapshotDeriver);
            let scheduler = ServerAgentRunScheduler::new(repository.clone(), turn_inbox.clone(), 8);
            let runtime = AgentCollaborationRuntime::new(
                repository.clone(),
                scheduler.clone(),
                repository.clone(),
            )
            .with_mailbox_notifier(scheduler.clone());
            let coordinator = Arc::new(AgentTurnCoordinator::new(
                store.clone(),
                repository.clone(),
                activity.clone(),
                runtime.clone(),
                snapshot_deriver.clone(),
                scheduler.clone(),
                base_agent.clone(),
                Arc::new(RwLock::new(settings.clone())),
                McpExtensionHost::new(),
                turn_inbox,
            ));

            let profiles = AgentProfileRegistry::load(&workspace).list();
            let allowed_agent_types = profiles
                .iter()
                .map(|profile| profile.name.clone())
                .collect::<Vec<_>>();
            let tool_catalog = base_agent
                .read()
                .expect("agent lock")
                .provider_tool_catalog();
            let tool_names = tool_catalog
                .iter()
                .map(|tool| tool.name.clone())
                .collect::<Vec<_>>();
            let (_, root, root_turn) = repository
                .create_session(CreateCollaborationSession {
                    user_task_id: user_thread.id,
                    root_turn_id: AgentTurnId::new(),
                    root_task_message: "multi-agent e2e root".to_string(),
                    root_agent_type: "default".to_string(),
                    root_runtime_snapshot: RuntimeSnapshotSeed::new(
                        None,
                        json!({
                            "schemaVersion": 1,
                            "agentType": "default",
                            "allowedAgentTypes": allowed_agent_types,
                            "agentProfiles": profiles,
                            "workspaceRoot": workspace,
                            "workspaceMode": "shared_read_only",
                            "workspaceAssignment": {
                                "mode": "shared_read_only",
                                "root": workspace,
                            },
                            "forkTurns": "all",
                            "provider": settings.active_provider(),
                            "permissionMode": settings.permission_mode,
                            "sandbox": settings.sandbox,
                            "agentRuntime": settings.agent_runtime,
                            "capabilityProjection": base_agent
                                .read()
                                .expect("agent lock")
                                .capability_projection(),
                            "tools": tool_names,
                            "toolCatalog": tool_catalog,
                            "spawnPolicy": {
                                "allowChildSpawns": spawn_policy.allow_child_spawns,
                                "maxDepth": spawn_policy.max_depth,
                                "maxDirectChildren": spawn_policy.max_direct_children,
                            }
                        }),
                    ),
                    session_policy,
                    root_spawn_policy: spawn_policy,
                })
                .await
                .expect("root collaboration session");
            let root_turn = repository
                .transition_turn(root_turn.id, AgentTurnStatus::Running)
                .await
                .expect("running root turn");

            Self {
                store,
                repository,
                activity,
                runtime,
                snapshot_deriver,
                scheduler,
                coordinator,
                user_task_id: user_thread.id,
                session_id: root.session_id,
                root,
                root_turn,
                workspace,
            }
        }

        fn start(&self) {
            self.scheduler.start(self.coordinator.clone());
        }

        fn tool_context(
            &self,
            agent: &AgentThreadRecord,
            turn: &AgentTurnRecord,
        ) -> ToolInvocationContext {
            let invocation = AgentCollaborationInvocation::new(
                self.runtime.clone(),
                self.activity.clone(),
                self.snapshot_deriver.clone(),
                AgentInvocationIdentity {
                    session_id: agent.session_id,
                    agent_thread_id: agent.id,
                    agent_turn_id: turn.id,
                    runtime_snapshot_id: agent.runtime_snapshot_id,
                },
            );
            let authority = ExecutionAuthority::new(
                self.workspace.clone(),
                PermissionMode::FullAccess,
                LocalSandboxConfig::from_env(),
                CapabilityProjection::unrestricted(),
            )
            .unwrap();
            let mut context = authority.local_tool_context();
            context.thread_id = Some(self.user_task_id);
            context.collaboration = Some(invocation);
            context.agent_turn_id = Some(turn.id.as_uuid());
            context.agent_depth = agent.path.depth().min(u8::MAX as u16) as u8;
            context.agent_path = agent.path.to_string();
            context
        }

        async fn invoke_tool(
            &self,
            context: ToolInvocationContext,
            name: &str,
            input: Value,
        ) -> opentopia_core::ToolResult {
            self.try_invoke_tool(context, name, input)
                .await
                .unwrap_or_else(|error| panic!("{name} failed: {error}"))
        }

        async fn try_invoke_tool(
            &self,
            context: ToolInvocationContext,
            name: &str,
            input: Value,
        ) -> anyhow::Result<opentopia_core::ToolResult> {
            ToolRegistry::with_builtins()
                .get(name)
                .unwrap_or_else(|| panic!("missing tool {name}"))
                .execute(ToolCall::new(name, input), context)
                .await
        }

        async fn await_terminal(&self, turn_id: AgentTurnId) -> AgentTurnRecord {
            timeout(Duration::from_secs(10), async {
                loop {
                    let turn = self
                        .repository
                        .get_turn(turn_id)
                        .await
                        .expect("persisted Agent Turn");
                    if turn.status.is_terminal() {
                        break turn;
                    }
                    sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("Agent Turn reached a terminal state")
        }

        async fn await_status(
            &self,
            turn_id: AgentTurnId,
            expected: AgentTurnStatus,
        ) -> AgentTurnRecord {
            timeout(Duration::from_secs(10), async {
                loop {
                    let turn = self
                        .repository
                        .get_turn(turn_id)
                        .await
                        .expect("persisted Agent Turn");
                    if turn.status == expected {
                        break turn;
                    }
                    assert!(
                        !turn.status.is_terminal(),
                        "Agent Turn reached {:?} before {:?}",
                        turn.status,
                        expected
                    );
                    sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("Agent Turn reached the expected state")
        }

        async fn pending_mailbox(&self, target: AgentThreadId) -> Vec<AgentMailboxMessage> {
            AgentMailbox::snapshot(&*self.repository, self.session_id, target, None, 64)
                .await
                .expect("mailbox snapshot")
        }
    }

    fn agent_thread_id(result: &opentopia_core::ToolResult) -> AgentThreadId {
        AgentThreadId::from_uuid(
            Uuid::parse_str(
                result.metadata["agentThreadId"]
                    .as_str()
                    .expect("agentThreadId metadata"),
            )
            .expect("agent thread UUID"),
        )
    }

    fn agent_turn_id(result: &opentopia_core::ToolResult) -> AgentTurnId {
        AgentTurnId::from_uuid(
            Uuid::parse_str(
                result.metadata["agentTurnId"]
                    .as_str()
                    .expect("agentTurnId metadata"),
            )
            .expect("agent turn UUID"),
        )
    }

    fn message(role: ModelConversationRole, content: &str) -> ModelConversationMessage {
        ModelConversationMessage {
            role,
            content: content.to_string(),
            content_parts: Vec::new(),
            tool_calls: Vec::new(),
            tool_results: Vec::new(),
        }
    }

    #[test]
    fn fork_count_keeps_complete_trailing_user_turns() {
        let conversation = vec![
            message(ModelConversationRole::User, "one"),
            message(ModelConversationRole::Assistant, "answer one"),
            message(ModelConversationRole::User, "two"),
            message(ModelConversationRole::Tool, "tool two"),
            message(ModelConversationRole::Assistant, "answer two"),
        ];
        let forked = apply_fork_window(conversation, &RuntimeForkTurnsV1::Count { count: 1 });
        assert_eq!(forked.len(), 3);
        assert_eq!(forked[0].content, "two");
        assert_eq!(forked[2].content, "answer two");
    }

    #[tokio::test]
    async fn multi_agent_e2e_spawn_runs_agent_core_and_returns_completion_to_parent() {
        let fixture = MultiAgentE2eFixture::new().await;
        fixture.start();
        let spawned = fixture
            .invoke_tool(
                fixture.tool_context(&fixture.root, &fixture.root_turn),
                "spawn_agent",
                json!({
                    "task_name": "worker",
                    "message": "inspect the production multi-agent path",
                    "agent_type": "default",
                    "workspace_mode": "shared_read_only"
                }),
            )
            .await;
        let child_id = agent_thread_id(&spawned);
        let child_turn_id = agent_turn_id(&spawned);
        let child_turn = fixture.await_terminal(child_turn_id).await;
        assert_eq!(child_turn.status, AgentTurnStatus::Completed);

        let waited = fixture
            .invoke_tool(
                fixture.tool_context(&fixture.root, &fixture.root_turn),
                "wait_agent",
                json!({ "target": child_id.to_string(), "timeout_ms": 0 }),
            )
            .await;
        let waited: Value = serde_json::from_str(&waited.output).expect("wait_agent JSON");
        assert_eq!(waited["turn"]["status"], "completed");
        assert_eq!(waited["agent"]["path"], "/root/worker");
        assert!(waited["activity"]["recentEvents"]
            .as_array()
            .is_some_and(|events| !events.is_empty()));
        assert!(waited["messages"].as_array().is_some_and(|messages| {
            messages.iter().any(|message| {
                message["kind"] == "completion"
                    && message["payload"]["result"].as_str().is_some_and(|result| {
                        result.contains("inspect the production multi-agent path")
                    })
            })
        }));
        assert_eq!(
            fixture
                .repository
                .list_ledger_items(child_id, "conversation")
                .expect("child conversation")
                .len(),
            2,
            "the real AgentCore run must persist its user and assistant messages"
        );
    }

    #[tokio::test]
    async fn multi_agent_e2e_followup_creates_a_second_turn_without_overwriting_the_first() {
        let fixture = MultiAgentE2eFixture::new().await;
        fixture.start();
        let first = fixture
            .invoke_tool(
                fixture.tool_context(&fixture.root, &fixture.root_turn),
                "spawn_agent",
                json!({
                    "task_name": "analyst",
                    "message": "first analysis",
                    "agent_type": "default"
                }),
            )
            .await;
        let child_id = agent_thread_id(&first);
        let first_turn_id = agent_turn_id(&first);
        assert_eq!(
            fixture.await_terminal(first_turn_id).await.status,
            AgentTurnStatus::Completed
        );

        let followup = fixture
            .invoke_tool(
                fixture.tool_context(&fixture.root, &fixture.root_turn),
                "followup_task",
                json!({ "target": child_id.to_string(), "message": "second analysis" }),
            )
            .await;
        let second_turn_id = agent_turn_id(&followup);
        assert_ne!(first_turn_id, second_turn_id);
        assert_eq!(
            fixture.await_terminal(second_turn_id).await.status,
            AgentTurnStatus::Completed
        );
        assert_eq!(
            fixture
                .repository
                .get_turn(first_turn_id)
                .await
                .expect("first turn preserved")
                .status,
            AgentTurnStatus::Completed
        );
        assert_eq!(
            fixture
                .repository
                .list_ledger_items(child_id, "conversation")
                .expect("continued conversation")
                .len(),
            4,
            "both independent turns must remain in the AgentThread ledger"
        );
        let completions = fixture.pending_mailbox(fixture.root.id).await;
        assert_eq!(
            completions
                .iter()
                .filter(|message| message.kind == AgentMailboxMessageKind::Completion)
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn multi_agent_e2e_authorized_child_can_spawn_a_grandchild_and_receive_its_outcome() {
        let scripted_provider = ScriptedChatServer::start().await;
        let fixture =
            MultiAgentE2eFixture::new_with_provider(Some(scripted_provider.provider_settings()))
                .await;
        let parent_spawn = fixture
            .invoke_tool(
                fixture.tool_context(&fixture.root, &fixture.root_turn),
                "spawn_agent",
                json!({
                    "task_name": "lead",
                    "message": "coordinate nested work",
                    "agent_type": "default",
                    "allow_child_spawns": true
                }),
            )
            .await;
        let parent_id = agent_thread_id(&parent_spawn);
        let parent_turn_id = agent_turn_id(&parent_spawn);
        let parent = fixture
            .repository
            .get_thread(parent_id)
            .await
            .expect("parent AgentThread");
        let parent_turn = fixture
            .repository
            .get_turn(parent_turn_id)
            .await
            .expect("parent AgentTurn");
        let leaf_spawn = fixture
            .invoke_tool(
                fixture.tool_context(&parent, &parent_turn),
                "spawn_agent",
                json!({
                    "task_name": "leaf",
                    "message": "perform nested leaf work",
                    "agent_type": "default"
                }),
            )
            .await;
        let leaf_id = agent_thread_id(&leaf_spawn);
        let leaf_turn_id = agent_turn_id(&leaf_spawn);
        fixture.start();

        let leaf_turn = fixture.await_terminal(leaf_turn_id).await;
        let parent_turn = fixture.await_terminal(parent_turn_id).await;
        assert_eq!(leaf_turn.status, AgentTurnStatus::Completed);
        assert_eq!(
            parent_turn.status,
            AgentTurnStatus::Completed,
            "parent outcome: {:?}",
            fixture
                .repository
                .list_ledger_items(parent_id, "turn_outcome")
                .expect("parent outcome ledger")
        );
        assert_eq!(
            fixture
                .repository
                .get_thread(leaf_id)
                .await
                .expect("leaf AgentThread")
                .path
                .as_str(),
            "/root/lead/leaf"
        );
        assert!(
            fixture.pending_mailbox(parent_id).await.is_empty(),
            "the parent AgentCore must consume and acknowledge the leaf completion"
        );
        let root_messages = fixture.pending_mailbox(fixture.root.id).await;
        assert!(root_messages.iter().any(|message| {
            message.kind == AgentMailboxMessageKind::Completion
                && message.from_agent_thread_id == parent_id
        }));
    }

    #[tokio::test]
    async fn multi_agent_e2e_list_agents_filters_tree_and_wait_timeout_is_non_mutating() {
        let fixture = MultiAgentE2eFixture::new().await;
        let lead_spawn = fixture
            .invoke_tool(
                fixture.tool_context(&fixture.root, &fixture.root_turn),
                "spawn_agent",
                json!({
                    "task_name": "lead",
                    "message": "own a queued subtree",
                    "agent_type": "default",
                    "allow_child_spawns": true
                }),
            )
            .await;
        let lead_id = agent_thread_id(&lead_spawn);
        let lead_turn_id = agent_turn_id(&lead_spawn);
        let lead = fixture
            .repository
            .get_thread(lead_id)
            .await
            .expect("lead AgentThread");
        let lead_turn = fixture
            .repository
            .get_turn(lead_turn_id)
            .await
            .expect("lead AgentTurn");
        let leaf_spawn = fixture
            .invoke_tool(
                fixture.tool_context(&lead, &lead_turn),
                "spawn_agent",
                json!({
                    "task_name": "leaf",
                    "message": "remain queued for wait timeout coverage",
                    "agent_type": "default"
                }),
            )
            .await;
        let leaf_id = agent_thread_id(&leaf_spawn);
        let leaf_turn_id = agent_turn_id(&leaf_spawn);

        let listed = fixture
            .invoke_tool(
                fixture.tool_context(&fixture.root, &fixture.root_turn),
                "list_agents",
                json!({}),
            )
            .await;
        let listed: Value = serde_json::from_str(&listed.output).expect("list_agents JSON");
        let paths = listed["agents"]
            .as_array()
            .expect("agents array")
            .iter()
            .map(|item| item["agent"]["path"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(paths, ["/root", "/root/lead", "/root/lead/leaf"]);
        assert_eq!(listed["agents"][1]["availability"], "queued");
        assert_eq!(listed["agents"][2]["availability"], "queued");

        let subtree = fixture
            .invoke_tool(
                fixture.tool_context(&fixture.root, &fixture.root_turn),
                "list_agents",
                json!({ "path_prefix": "/root/lead" }),
            )
            .await;
        let subtree: Value = serde_json::from_str(&subtree.output).expect("subtree JSON");
        assert_eq!(subtree["agents"].as_array().unwrap().len(), 2);

        let waited = fixture
            .invoke_tool(
                fixture.tool_context(&fixture.root, &fixture.root_turn),
                "wait_agent",
                json!({
                    "target": leaf_id.to_string(),
                    "after_cursor": 0,
                    "timeout_ms": 25
                }),
            )
            .await;
        let waited: Value = serde_json::from_str(&waited.output).expect("wait timeout JSON");
        assert_eq!(waited["timedOut"], true);
        assert_eq!(waited["agent"]["id"], leaf_id.to_string());
        assert_eq!(waited["turn"]["status"], "queued");
        assert_eq!(
            fixture
                .repository
                .get_turn(leaf_turn_id)
                .await
                .expect("leaf remains persisted")
                .status,
            AgentTurnStatus::Queued,
            "wait_agent must never mutate the target"
        );

        let immediate = fixture
            .invoke_tool(
                fixture.tool_context(&fixture.root, &fixture.root_turn),
                "wait_agent",
                json!({ "target": "/root/lead/leaf", "timeout_ms": 0 }),
            )
            .await;
        let immediate: Value =
            serde_json::from_str(&immediate.output).expect("immediate wait JSON");
        assert_eq!(immediate["timedOut"], false);
        assert_eq!(immediate["turn"]["status"], "queued");
    }

    #[tokio::test]
    async fn multi_agent_e2e_cancelling_wait_agent_does_not_cancel_its_target() {
        let fixture = MultiAgentE2eFixture::new().await;
        let spawned = fixture
            .invoke_tool(
                fixture.tool_context(&fixture.root, &fixture.root_turn),
                "spawn_agent",
                json!({
                    "task_name": "wait_target",
                    "message": "remain queued while the waiter is cancelled",
                    "agent_type": "default"
                }),
            )
            .await;
        let child_turn_id = agent_turn_id(&spawned);
        let cancellation = CancellationToken::new();
        let mut context = fixture.tool_context(&fixture.root, &fixture.root_turn);
        context.cancel = Some(cancellation.clone());
        tokio::spawn(async move {
            sleep(Duration::from_millis(25)).await;
            cancellation.cancel();
        });
        let error = fixture
            .try_invoke_tool(
                context,
                "wait_agent",
                json!({
                    "target": agent_thread_id(&spawned).to_string(),
                    "after_cursor": 0,
                    "timeout_ms": 5000
                }),
            )
            .await
            .expect_err("cancelled waiter must stop blocking");
        assert!(error.to_string().contains("cancelled"));
        assert_eq!(
            fixture
                .repository
                .get_turn(child_turn_id)
                .await
                .expect("target turn")
                .status,
            AgentTurnStatus::Queued
        );
    }

    #[tokio::test]
    async fn multi_agent_e2e_wait_agent_wakes_from_durable_activity_notification() {
        let fixture = MultiAgentE2eFixture::new().await;
        let spawned = fixture
            .invoke_tool(
                fixture.tool_context(&fixture.root, &fixture.root_turn),
                "spawn_agent",
                json!({
                    "task_name": "wait_wakeup",
                    "message": "produce activity after the waiter subscribes",
                    "agent_type": "default"
                }),
            )
            .await;
        let child_id = agent_thread_id(&spawned);
        let child_turn_id = agent_turn_id(&spawned);
        let wait_tool = ToolRegistry::with_builtins()
            .get("wait_agent")
            .expect("wait_agent tool");
        let wait_context = fixture.tool_context(&fixture.root, &fixture.root_turn);
        let wait_task = tokio::spawn(async move {
            wait_tool
                .execute(
                    ToolCall::new(
                        "wait_agent",
                        json!({
                            "target": child_id.to_string(),
                            "after_cursor": 0,
                            "timeout_ms": 5000
                        }),
                    ),
                    wait_context,
                )
                .await
                .expect("event-driven wait")
        });
        sleep(Duration::from_millis(25)).await;
        fixture.start();
        let waited = timeout(Duration::from_secs(2), wait_task)
            .await
            .expect("wait_agent woke without polling")
            .expect("wait task joined");
        let waited: Value = serde_json::from_str(&waited.output).expect("wait wakeup JSON");
        assert_eq!(waited["timedOut"], false);
        assert_eq!(waited["agent"]["id"], child_id.to_string());
        assert!(matches!(
            waited["turn"]["status"].as_str(),
            Some("running" | "completed")
        ));
        assert_eq!(
            fixture.await_terminal(child_turn_id).await.status,
            AgentTurnStatus::Completed
        );
    }

    #[tokio::test]
    async fn multi_agent_e2e_tool_boundary_rejects_capability_escalation_and_invalid_targets() {
        let fixture = MultiAgentE2eFixture::new().await;
        let child_spawn = fixture
            .invoke_tool(
                fixture.tool_context(&fixture.root, &fixture.root_turn),
                "spawn_agent",
                json!({
                    "task_name": "bounded",
                    "message": "must remain a leaf",
                    "agent_type": "default"
                }),
            )
            .await;
        let child = fixture
            .repository
            .get_thread(agent_thread_id(&child_spawn))
            .await
            .expect("bounded child");
        let child_turn = fixture
            .repository
            .get_turn(agent_turn_id(&child_spawn))
            .await
            .expect("bounded child turn");

        let recursive_error = fixture
            .try_invoke_tool(
                fixture.tool_context(&child, &child_turn),
                "spawn_agent",
                json!({
                    "task_name": "forbidden",
                    "message": "attempt capability escalation",
                    "agent_type": "default",
                    "allow_child_spawns": true
                }),
            )
            .await
            .expect_err("leaf child must not create a grandchild");
        assert!(recursive_error
            .to_string()
            .contains("does not allow recursive spawn"));

        let workspace_error = fixture
            .try_invoke_tool(
                fixture.tool_context(&fixture.root, &fixture.root_turn),
                "spawn_agent",
                json!({
                    "task_name": "writer",
                    "message": "attempt write escalation",
                    "agent_type": "default",
                    "workspace_mode": "shared_coordinated"
                }),
            )
            .await
            .expect_err("read-only parent must not grant coordinated writes");
        assert!(workspace_error
            .to_string()
            .contains("expand a read-only parent snapshot"));

        let profile_error = fixture
            .try_invoke_tool(
                fixture.tool_context(&fixture.root, &fixture.root_turn),
                "spawn_agent",
                json!({
                    "task_name": "unknown_profile",
                    "message": "use an unavailable profile",
                    "agent_type": "definitely_missing"
                }),
            )
            .await
            .expect_err("unknown agent profile must fail");
        assert!(profile_error.to_string().contains("unknown agent_type"));

        let target_error = fixture
            .try_invoke_tool(
                fixture.tool_context(&fixture.root, &fixture.root_turn),
                "send_message",
                json!({ "target": "missing", "message": "must not escape the tree" }),
            )
            .await
            .expect_err("missing target must fail");
        assert!(target_error.to_string().contains("was not found"));

        let duplicate_error = fixture
            .try_invoke_tool(
                fixture.tool_context(&fixture.root, &fixture.root_turn),
                "spawn_agent",
                json!({
                    "task_name": "bounded",
                    "message": "duplicate canonical path",
                    "agent_type": "default"
                }),
            )
            .await
            .expect_err("duplicate task name must fail atomically");
        assert!(duplicate_error.to_string().contains("already exists"));
        assert_eq!(
            fixture
                .repository
                .list_threads(fixture.session_id)
                .await
                .expect("tree remains readable")
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn multi_agent_e2e_sqlite_spawn_limits_fail_atomically() {
        let direct_fixture = MultiAgentE2eFixture::new_with_policy(
            None,
            CollaborationSessionPolicy {
                max_agents: 8,
                max_active_runs: 2,
                max_depth: 2,
            },
            AgentSpawnPolicy::allows_children(2, 1),
        )
        .await;
        direct_fixture
            .invoke_tool(
                direct_fixture.tool_context(&direct_fixture.root, &direct_fixture.root_turn),
                "spawn_agent",
                json!({
                    "task_name": "only_child",
                    "message": "consume the direct child allowance",
                    "agent_type": "default"
                }),
            )
            .await;
        let direct_error = direct_fixture
            .try_invoke_tool(
                direct_fixture.tool_context(&direct_fixture.root, &direct_fixture.root_turn),
                "spawn_agent",
                json!({
                    "task_name": "excess_child",
                    "message": "must exceed direct child allowance",
                    "agent_type": "default"
                }),
            )
            .await
            .expect_err("maximum direct children must be enforced");
        assert!(direct_error
            .to_string()
            .contains("direct child limit reached"));
        assert_eq!(
            direct_fixture
                .repository
                .list_threads(direct_fixture.session_id)
                .await
                .unwrap()
                .len(),
            2
        );

        let capacity_fixture = MultiAgentE2eFixture::new_with_policy(
            None,
            CollaborationSessionPolicy {
                max_agents: 2,
                max_active_runs: 2,
                max_depth: 2,
            },
            AgentSpawnPolicy::allows_children(2, 4),
        )
        .await;
        capacity_fixture
            .invoke_tool(
                capacity_fixture.tool_context(&capacity_fixture.root, &capacity_fixture.root_turn),
                "spawn_agent",
                json!({
                    "task_name": "capacity_one",
                    "message": "consume the session capacity",
                    "agent_type": "default"
                }),
            )
            .await;
        let capacity_error = capacity_fixture
            .try_invoke_tool(
                capacity_fixture.tool_context(&capacity_fixture.root, &capacity_fixture.root_turn),
                "spawn_agent",
                json!({
                    "task_name": "capacity_two",
                    "message": "must exceed session capacity",
                    "agent_type": "default"
                }),
            )
            .await
            .expect_err("maximum session agents must be enforced");
        assert!(capacity_error
            .to_string()
            .contains("session agent limit reached"));
        assert_eq!(
            capacity_fixture
                .repository
                .list_threads(capacity_fixture.session_id)
                .await
                .unwrap()
                .len(),
            2
        );

        let depth_fixture = MultiAgentE2eFixture::new_with_policy(
            None,
            CollaborationSessionPolicy {
                max_agents: 8,
                max_active_runs: 2,
                max_depth: 1,
            },
            AgentSpawnPolicy::allows_children(1, 4),
        )
        .await;
        let child_spawn = depth_fixture
            .invoke_tool(
                depth_fixture.tool_context(&depth_fixture.root, &depth_fixture.root_turn),
                "spawn_agent",
                json!({
                    "task_name": "depth_one",
                    "message": "create the deepest permitted child",
                    "agent_type": "default",
                    "allow_child_spawns": true
                }),
            )
            .await;
        let child = depth_fixture
            .repository
            .get_thread(agent_thread_id(&child_spawn))
            .await
            .unwrap();
        let child_turn = depth_fixture
            .repository
            .get_turn(agent_turn_id(&child_spawn))
            .await
            .unwrap();
        let depth_error = depth_fixture
            .try_invoke_tool(
                depth_fixture.tool_context(&child, &child_turn),
                "spawn_agent",
                json!({
                    "task_name": "depth_two",
                    "message": "must exceed maximum depth",
                    "agent_type": "default"
                }),
            )
            .await
            .expect_err("maximum depth must be enforced");
        assert!(depth_error.to_string().contains("exceeds maximum"));
        assert_eq!(
            depth_fixture
                .repository
                .list_threads(depth_fixture.session_id)
                .await
                .unwrap()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn multi_agent_e2e_send_message_is_delivered_at_the_child_model_safe_point() {
        let fixture = MultiAgentE2eFixture::new().await;
        let spawned = fixture
            .invoke_tool(
                fixture.tool_context(&fixture.root, &fixture.root_turn),
                "spawn_agent",
                json!({
                    "task_name": "messaged",
                    "message": "process a task with parent context",
                    "agent_type": "default"
                }),
            )
            .await;
        let child_id = agent_thread_id(&spawned);
        let child_turn_id = agent_turn_id(&spawned);
        for text in [
            "first durable context",
            "second durable context",
            "third durable context",
        ] {
            fixture
                .invoke_tool(
                    fixture.tool_context(&fixture.root, &fixture.root_turn),
                    "send_message",
                    json!({ "target": child_id.to_string(), "message": text }),
                )
                .await;
        }
        let pending = fixture.pending_mailbox(child_id).await;
        assert_eq!(pending.len(), 3);
        assert!(pending
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence));
        assert_eq!(
            pending
                .iter()
                .map(|message| message.payload["text"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "first durable context",
                "second durable context",
                "third durable context"
            ]
        );

        fixture.start();
        let child_turn = fixture.await_terminal(child_turn_id).await;
        assert_eq!(
            child_turn.status,
            AgentTurnStatus::Completed,
            "child outcome: {:?}",
            fixture
                .repository
                .list_ledger_items(child_id, "turn_outcome")
                .expect("child outcome ledger")
        );
        assert!(
            fixture.pending_mailbox(child_id).await.is_empty(),
            "the child must acknowledge the message only after a model round receives it"
        );
        assert!(fixture
            .pending_mailbox(fixture.root.id)
            .await
            .iter()
            .any(|message| {
                message.kind == AgentMailboxMessageKind::Completion
                    && message.from_agent_thread_id == child_id
            }));
    }

    #[tokio::test]
    async fn multi_agent_e2e_interrupt_cancels_an_in_flight_provider_request() {
        let blocking_provider = BlockingChatServer::start().await;
        let fixture =
            MultiAgentE2eFixture::new_with_provider(Some(blocking_provider.provider_settings()))
                .await;
        fixture.start();
        let spawned = fixture
            .invoke_tool(
                fixture.tool_context(&fixture.root, &fixture.root_turn),
                "spawn_agent",
                json!({
                    "task_name": "running_cancel",
                    "message": "block in the provider until interrupted",
                    "agent_type": "default"
                }),
            )
            .await;
        let child_id = agent_thread_id(&spawned);
        let child_turn_id = agent_turn_id(&spawned);
        blocking_provider.wait_until_requested().await;
        fixture
            .await_status(child_turn_id, AgentTurnStatus::Running)
            .await;

        fixture
            .invoke_tool(
                fixture.tool_context(&fixture.root, &fixture.root_turn),
                "interrupt_agent",
                json!({ "target": child_id.to_string() }),
            )
            .await;
        assert_eq!(
            fixture.await_terminal(child_turn_id).await.status,
            AgentTurnStatus::Cancelled
        );
        assert!(fixture
            .pending_mailbox(fixture.root.id)
            .await
            .iter()
            .any(|message| {
                message.kind == AgentMailboxMessageKind::Completion
                    && message.from_agent_thread_id == child_id
                    && message.payload["status"] == "cancelled"
            }));
    }

    #[tokio::test]
    async fn multi_agent_e2e_provider_failure_is_persisted_and_reported_to_the_parent() {
        let rejecting_provider = RejectingChatServer::start().await;
        let fixture =
            MultiAgentE2eFixture::new_with_provider(Some(rejecting_provider.provider_settings()))
                .await;
        fixture.start();
        let spawned = fixture
            .invoke_tool(
                fixture.tool_context(&fixture.root, &fixture.root_turn),
                "spawn_agent",
                json!({
                    "task_name": "provider_failure",
                    "message": "exercise the provider error path",
                    "agent_type": "default"
                }),
            )
            .await;
        let child_id = agent_thread_id(&spawned);
        let child_turn_id = agent_turn_id(&spawned);
        assert_eq!(
            fixture.await_terminal(child_turn_id).await.status,
            AgentTurnStatus::Failed
        );
        let outcome = fixture
            .repository
            .list_ledger_items(child_id, "turn_outcome")
            .expect("failed outcome ledger");
        assert_eq!(outcome.len(), 1);
        assert!(outcome[0]["error"]
            .as_str()
            .is_some_and(|error| error.contains("scripted provider rejection")));

        let waited = fixture
            .invoke_tool(
                fixture.tool_context(&fixture.root, &fixture.root_turn),
                "wait_agent",
                json!({ "target": child_id.to_string(), "timeout_ms": 0 }),
            )
            .await;
        let waited: Value = serde_json::from_str(&waited.output).expect("failed wait JSON");
        assert_eq!(waited["turn"]["status"], "failed");
        assert!(waited["activity"]["recentEvents"]
            .as_array()
            .is_some_and(|events| events.iter().any(|event| event["kind"] == "error")));
        assert!(waited["messages"].as_array().is_some_and(|messages| {
            messages.iter().any(|message| {
                message["kind"] == "completion" && message["payload"]["status"] == "failed"
            })
        }));
    }

    #[tokio::test]
    async fn multi_agent_e2e_child_approval_suspends_and_resumes_the_same_turn() {
        let approval_provider = ApprovalChatServer::start().await;
        let fixture =
            MultiAgentE2eFixture::new_with_provider(Some(approval_provider.provider_settings()))
                .await;
        fixture.start();
        let spawned = fixture
            .invoke_tool(
                fixture.tool_context(&fixture.root, &fixture.root_turn),
                "spawn_agent",
                json!({
                    "task_name": "approval_child",
                    "message": "request approval and resume this exact turn",
                    "agent_type": "default"
                }),
            )
            .await;
        let child_id = agent_thread_id(&spawned);
        let child_turn_id = agent_turn_id(&spawned);
        let waiting = fixture
            .await_status(child_turn_id, AgentTurnStatus::WaitingApproval)
            .await;
        assert_eq!(waiting.invocation_id, 1);
        let approvals = fixture
            .store
            .list_approvals(fixture.user_task_id, Some(ApprovalStatus::Pending))
            .expect("pending child approval");
        assert_eq!(approvals.len(), 1);
        let approval_id = approvals[0].approval_id;
        assert!(fixture
            .repository
            .get_turn_checkpoint(child_turn_id)
            .expect("approval checkpoint")
            .is_some());
        assert!(fixture
            .pending_mailbox(fixture.root.id)
            .await
            .iter()
            .any(|message| {
                message.kind == AgentMailboxMessageKind::NeedsAttention
                    && message.from_agent_thread_id == child_id
                    && message.payload["status"] == "waiting_approval"
            }));

        fixture
            .store
            .update_approval_status(approval_id, ApprovalStatus::Denied)
            .expect("deny approval");
        fixture
            .scheduler
            .submit(AgentRunCommand::Resume {
                session_id: fixture.session_id,
                agent_thread_id: child_id,
                agent_turn_id: child_turn_id,
                invocation_id: 2,
                signal: AgentRunResumeSignal::Approval {
                    approval_id: Some(approval_id),
                    approved: false,
                },
            })
            .await
            .expect("submit child approval resume");
        let completed = fixture.await_terminal(child_turn_id).await;
        assert_eq!(completed.status, AgentTurnStatus::Completed);
        assert_eq!(completed.invocation_id, 2);
        assert!(fixture
            .repository
            .get_turn_checkpoint(child_turn_id)
            .expect("checkpoint removed")
            .is_none());
        assert!(fixture
            .pending_mailbox(fixture.root.id)
            .await
            .iter()
            .any(|message| {
                message.kind == AgentMailboxMessageKind::Completion
                    && message.from_agent_thread_id == child_id
                    && message.payload["status"] == "completed"
            }));
    }

    #[tokio::test]
    async fn multi_agent_e2e_interrupt_cancels_a_queued_child_and_reports_the_outcome() {
        let fixture = MultiAgentE2eFixture::new().await;
        let spawned = fixture
            .invoke_tool(
                fixture.tool_context(&fixture.root, &fixture.root_turn),
                "spawn_agent",
                json!({
                    "task_name": "cancelled",
                    "message": "this queued task should never start",
                    "agent_type": "default"
                }),
            )
            .await;
        let child_id = agent_thread_id(&spawned);
        let child_turn_id = agent_turn_id(&spawned);
        fixture
            .invoke_tool(
                fixture.tool_context(&fixture.root, &fixture.root_turn),
                "interrupt_agent",
                json!({ "target": child_id.to_string() }),
            )
            .await;
        fixture.start();

        assert_eq!(
            fixture.await_terminal(child_turn_id).await.status,
            AgentTurnStatus::Cancelled
        );
        let root_messages = fixture.pending_mailbox(fixture.root.id).await;
        assert!(root_messages.iter().any(|message| {
            message.kind == AgentMailboxMessageKind::Completion
                && message.payload["status"] == "cancelled"
        }));
        assert!(fixture
            .repository
            .list_ledger_items(child_id, "conversation")
            .expect("cancelled child ledger")
            .is_empty());
    }

    #[tokio::test]
    async fn multi_agent_e2e_interrupt_is_idempotent_and_cascades_through_the_subtree() {
        let fixture = MultiAgentE2eFixture::new().await;
        let parent_spawn = fixture
            .invoke_tool(
                fixture.tool_context(&fixture.root, &fixture.root_turn),
                "spawn_agent",
                json!({
                    "task_name": "cancel_tree",
                    "message": "own a subtree that will be cancelled",
                    "agent_type": "default",
                    "allow_child_spawns": true
                }),
            )
            .await;
        let parent_id = agent_thread_id(&parent_spawn);
        let parent_turn_id = agent_turn_id(&parent_spawn);
        let parent = fixture.repository.get_thread(parent_id).await.unwrap();
        let parent_turn = fixture.repository.get_turn(parent_turn_id).await.unwrap();
        let leaf_spawn = fixture
            .invoke_tool(
                fixture.tool_context(&parent, &parent_turn),
                "spawn_agent",
                json!({
                    "task_name": "leaf",
                    "message": "be cancelled with the parent",
                    "agent_type": "default"
                }),
            )
            .await;
        let leaf_turn_id = agent_turn_id(&leaf_spawn);

        let first_interrupt = fixture
            .invoke_tool(
                fixture.tool_context(&fixture.root, &fixture.root_turn),
                "interrupt_agent",
                json!({ "target": "/root/cancel_tree" }),
            )
            .await;
        let first_interrupt: Value =
            serde_json::from_str(&first_interrupt.output).expect("first interrupt JSON");
        assert_eq!(first_interrupt["interruptRequested"], true);
        fixture.start();
        assert_eq!(
            fixture.await_terminal(leaf_turn_id).await.status,
            AgentTurnStatus::Cancelled
        );
        assert_eq!(
            fixture.await_terminal(parent_turn_id).await.status,
            AgentTurnStatus::Cancelled
        );

        let second_interrupt = fixture
            .invoke_tool(
                fixture.tool_context(&fixture.root, &fixture.root_turn),
                "interrupt_agent",
                json!({ "target": parent_id.to_string() }),
            )
            .await;
        let second_interrupt: Value =
            serde_json::from_str(&second_interrupt.output).expect("second interrupt JSON");
        assert_eq!(second_interrupt["turn"]["id"], parent_turn_id.to_string());
        assert_eq!(second_interrupt["turn"]["status"], "cancelled");
        assert_eq!(second_interrupt["interruptRequested"], false);
    }

    #[tokio::test]
    async fn multi_agent_e2e_prepares_a_real_isolated_git_worktree() {
        let fixture = MultiAgentE2eFixture::new().await;
        let test_root = std::env::temp_dir().join(format!(
            "opentopia-isolated-worktree-test-{}",
            Uuid::new_v4()
        ));
        assert!(test_root.starts_with(std::env::temp_dir()));
        let repository = test_root.join("repository");
        std::fs::create_dir_all(&repository).expect("create temporary repository");
        run_git(&repository, &["init"]);
        run_git(
            &repository,
            &["config", "user.email", "test@opentopia.local"],
        );
        run_git(&repository, &["config", "user.name", "OpenTopia Test"]);
        std::fs::write(repository.join("seed.txt"), "frozen base\n").expect("write seed file");
        run_git(&repository, &["add", "seed.txt"]);
        run_git(&repository, &["commit", "-m", "test base"]);
        let base_commit = run_git(&repository, &["rev-parse", "HEAD"]);

        let worktree = repository
            .join(".opentopia")
            .join("agents")
            .join("isolated-child");
        let branch = format!("codex/test-worktree-{}", Uuid::new_v4());
        let settings = AppSettings::from_env(PermissionMode::FullAccess);
        let snapshot = RuntimeSnapshotV1::decode(&json!({
            "schemaVersion": 1,
            "agentType": "default",
            "allowedAgentTypes": ["default"],
            "agentProfiles": [],
            "workspaceMode": "isolated_worktree",
            "workspaceRoot": worktree,
            "workspaceAssignment": {
                "mode": "isolated_worktree",
                "repositoryRoot": repository,
                "root": worktree,
                "branch": branch,
                "baseCommit": base_commit,
                "deliveryState": "pending"
            },
            "gitBaseCommit": base_commit,
            "forkTurns": "all",
            "provider": settings.active_provider(),
            "permissionMode": settings.permission_mode,
            "sandbox": settings.sandbox,
            "agentRuntime": settings.agent_runtime,
            "capabilityProjection": CapabilityProjection::unrestricted(),
            "tools": [],
            "toolCatalog": [],
            "pluginContributions": [],
            "attachmentReferences": [],
            "spawnPolicy": {
                "allowChildSpawns": false,
                "maxDepth": 1,
                "maxDirectChildren": 0
            }
        }))
        .expect("valid isolated worktree snapshot");
        let prepared = fixture
            .coordinator
            .prepare_workspace(&snapshot, &settings)
            .await
            .expect("prepare isolated worktree");

        assert_eq!(prepared, worktree);
        assert_eq!(
            std::fs::read_to_string(prepared.join("seed.txt"))
                .expect("read worktree seed")
                .trim_end(),
            "frozen base"
        );
        assert_eq!(
            run_git(&prepared, &["rev-parse", "--abbrev-ref", "HEAD"]),
            branch
        );
        assert_eq!(run_git(&prepared, &["rev-parse", "HEAD"]), base_commit);

        let worktree_string = prepared.to_string_lossy().into_owned();
        run_git(
            &repository,
            &["worktree", "remove", "--force", &worktree_string],
        );
        run_git(&repository, &["branch", "-D", &branch]);
        std::fs::remove_dir_all(&test_root).expect("remove temporary repository");
    }
}

#[async_trait]
impl AgentRunExecutor for AgentTurnCoordinator {
    async fn execute(&self, command: AgentRunCommand, cancellation: CancellationToken) {
        let (agent_thread_id, agent_turn_id) = match &command {
            AgentRunCommand::Start {
                agent_thread_id,
                agent_turn_id,
                ..
            }
            | AgentRunCommand::Resume {
                agent_thread_id,
                agent_turn_id,
                ..
            }
            | AgentRunCommand::Cancel {
                agent_thread_id,
                agent_turn_id,
                ..
            } => (*agent_thread_id, *agent_turn_id),
        };
        let result = match command {
            AgentRunCommand::Start {
                agent_thread_id,
                agent_turn_id,
                ..
            } => {
                self.execute_start(agent_thread_id, agent_turn_id, cancellation)
                    .await
            }
            AgentRunCommand::Resume {
                agent_thread_id,
                agent_turn_id,
                signal,
                ..
            } => {
                self.execute_resume(agent_thread_id, agent_turn_id, signal, cancellation)
                    .await
            }
            AgentRunCommand::Cancel { .. } => Ok(()),
        };
        if let Err(error) = result {
            tracing::error!(?error, "Agent Turn coordinator failed");
            if let Ok(turn) = self.repository.get_turn(agent_turn_id).await {
                if !turn.status.is_terminal() {
                    if turn.status == AgentTurnStatus::Queued {
                        let _ = self
                            .repository
                            .transition_turn(agent_turn_id, AgentTurnStatus::Running)
                            .await;
                    }
                    let _ = self.finish(
                        agent_thread_id,
                        agent_turn_id,
                        AgentTurnStatus::Failed,
                        json!({
                            "status": "failed",
                            "error": error.to_string(),
                        }),
                    );
                }
            }
        }
    }
}
