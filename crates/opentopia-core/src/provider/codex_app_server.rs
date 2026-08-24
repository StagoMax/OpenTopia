use super::{
    app_server_idle_timeout, encode_base64, ensure_visual_input_supported, instruction_messages,
    model_response_observation, provider_tool_result_content, redact_transport_value,
    resource_fallback_text, ModelConversationRole, ModelFinishReason, ModelProvider, ModelRequest,
    ModelResponse, ModelStreamCallback, ModelStreamDelta, PreparedProviderRequest,
    ProviderToolCall, ProviderToolCandidate, ProviderToolResult, ProviderTransportCallback,
    ProviderTransportEvent, NATIVE_WEB_SEARCH_PRIORITY_INSTRUCTION, PROVIDER_NETWORK_RETRY_LIMIT,
};
use crate::model::{ModelContentPart, ProviderRetryKind};
use crate::model_context::ContextRole;
use crate::settings::{ProviderHealthCheck, ProviderSettings, ProviderTransportKind};
use anyhow::Context;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;
use uuid::Uuid;

/// A local adapter for the Codex App Server protocol.
///
/// Images stay generic until this boundary. They are materialized into a
/// private temporary file and referenced through the documented `localImage`
/// input, allowing Codex to use its own attachment/upload path without an
/// OpenTopia-hosted asset server.
pub struct CodexAppServerProvider {
    supports_vision: bool,
    pub(super) native_web_search: bool,
    sessions: Mutex<HashMap<String, CodexAppServerSession>>,
}

mod account;

pub use account::{CodexAccountManager, CodexAccountStatus, CodexLoginStart};

struct CodexAppServerSession {
    child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    attachment_paths: Vec<PathBuf>,
    thread_id: String,
    turn_id: String,
    assistant_text: String,
    received_agent_delta: bool,
    network_retry_count: usize,
    final_message_item_ids: HashSet<String>,
    pending_tool_call: Option<CodexDynamicToolCall>,
}

pub(super) struct CodexDynamicToolCall {
    pub(super) rpc_id: Value,
    pub(super) call_id: String,
    pub(super) name: String,
    pub(super) arguments: Value,
}

enum CodexDriveResult {
    ToolCall(CodexDynamicToolCall),
    Completed(String),
}

impl CodexAppServerProvider {
    pub fn from_settings(settings: &ProviderSettings) -> Option<Self> {
        (settings.effective_transport() == ProviderTransportKind::CodexAppServer).then(|| Self {
            supports_vision: settings.supports_vision_for_model(),
            native_web_search: false,
            sessions: Mutex::new(HashMap::new()),
        })
    }

    pub(crate) fn for_guardian(mut self) -> Self {
        self.native_web_search = false;
        self
    }

    async fn start_session(&self, request: &ModelRequest) -> anyhow::Result<CodexAppServerSession> {
        ensure_visual_input_supported(request, self.supports_vision)?;

        let mut command = codex_app_server_command();
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn().context(
            "failed to start the local Codex App Server; install Codex or add it to PATH",
        )?;
        let stdin = child
            .stdin
            .take()
            .context("Codex App Server did not expose stdin")?;
        let stdout = child
            .stdout
            .take()
            .context("Codex App Server did not expose stdout")?;
        let mut session = CodexAppServerSession {
            child,
            stdin,
            stdout: BufReader::new(stdout).lines(),
            attachment_paths: Vec::new(),
            thread_id: String::new(),
            turn_id: String::new(),
            assistant_text: String::new(),
            received_agent_delta: false,
            network_retry_count: 0,
            final_message_item_ids: HashSet::new(),
            pending_tool_call: None,
        };

        if let Err(error) = self.initialize_and_start(&mut session, request).await {
            session.cleanup().await;
            return Err(error);
        }
        Ok(session)
    }

    async fn initialize_and_start(
        &self,
        session: &mut CodexAppServerSession,
        request: &ModelRequest,
    ) -> anyhow::Result<()> {
        codex_write_rpc(
            &mut session.stdin,
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "clientInfo": { "name": "OpenTopia", "version": env!("CARGO_PKG_VERSION") },
                    "capabilities": { "experimentalApi": true }
                }
            }),
        )
        .await?;
        codex_wait_for_response(&mut session.stdout, 1).await?;

        let cwd = std::env::current_dir()
            .context("failed to resolve current directory for Codex App Server")?;
        let mut thread_params = json!({
            "cwd": cwd,
            "sandbox": "read-only",
            "approvalPolicy": "never",
            "ephemeral": true,
            "environments": [],
            "developerInstructions": codex_developer_instructions(
                request,
                self.native_web_search,
            ),
        });
        if !request.tool_candidates.is_empty() {
            thread_params["dynamicTools"] = json!(codex_dynamic_tools(&request.tool_candidates));
        }
        codex_write_rpc(
            &mut session.stdin,
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "thread/start",
                "params": thread_params,
            }),
        )
        .await?;
        let thread_response = codex_wait_for_response(&mut session.stdout, 2).await?;
        session.thread_id = thread_response
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .context("Codex App Server thread/start response omitted thread.id")?
            .to_string();

        let input = codex_turn_input(request, &mut session.attachment_paths)?;
        codex_write_rpc(
            &mut session.stdin,
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "turn/start",
                "params": { "threadId": session.thread_id, "input": input },
            }),
        )
        .await?;
        let turn_response = codex_wait_for_response(&mut session.stdout, 3).await?;
        session.turn_id = turn_response
            .pointer("/turn/id")
            .and_then(Value::as_str)
            .context("Codex App Server turn/start response omitted turn.id")?
            .to_string();
        Ok(())
    }

    async fn resume_session(
        &self,
        session: &mut CodexAppServerSession,
        results: &[ProviderToolResult],
    ) -> anyhow::Result<()> {
        let pending = session
            .pending_tool_call
            .take()
            .context("Codex App Server session has no pending dynamic tool call")?;
        let result = results
            .iter()
            .find(|result| result.call_id == pending.call_id)
            .context("OpenTopia did not return the pending Codex dynamic tool result")?;

        let mut content_items = vec![json!({
            "type": "inputText",
            "text": provider_tool_result_content(result),
        })];
        for part in &result.content {
            if let ModelContentPart::Image { content_type, data } = part {
                content_items.push(json!({
                    "type": "inputImage",
                    "imageUrl": format!("data:{content_type};base64,{}", encode_base64(&data)),
                }));
            }
        }
        codex_write_rpc(
            &mut session.stdin,
            json!({
                "jsonrpc": "2.0",
                "id": pending.rpc_id,
                "result": {
                    "success": !result.is_error,
                    "contentItems": content_items,
                }
            }),
        )
        .await
    }

    async fn drive_session(
        &self,
        session: &mut CodexAppServerSession,
        on_transport: &mut ProviderTransportCallback<'_>,
    ) -> anyhow::Result<CodexDriveResult> {
        // The bound is on silence, not on total turn length: a Codex turn may legitimately
        // run for a long time as long as it keeps emitting events, but a session that goes
        // quiet must not hold the caller open indefinitely.
        let idle_timeout = app_server_idle_timeout();
        loop {
            let event = tokio::time::timeout(idle_timeout, codex_next_event(&mut session.stdout))
                .await
                .map_err(|_| {
                    anyhow::anyhow!(
                        "Codex App Server stalled: no event for {} seconds",
                        idle_timeout.as_secs()
                    )
                })??;
            let Some(method) = event.get("method").and_then(Value::as_str) else {
                continue;
            };
            let params = event.get("params").cloned().unwrap_or(Value::Null);
            match method {
                "item/started" => {
                    if params.get("turnId").and_then(Value::as_str) == Some(&session.turn_id) {
                        if let Some(item_type) =
                            params.pointer("/item/type").and_then(Value::as_str)
                        {
                            if is_codex_builtin_action(item_type) {
                                anyhow::bail!(
                                    "Codex App Server attempted built-in action {item_type}; OpenTopia must execute all actions through its own tools"
                                );
                            }
                        }
                        if params.pointer("/item/type").and_then(Value::as_str)
                            == Some("agentMessage")
                            && params.pointer("/item/phase").and_then(Value::as_str)
                                == Some("final_answer")
                        {
                            if let Some(item_id) =
                                params.pointer("/item/id").and_then(Value::as_str)
                            {
                                session.final_message_item_ids.insert(item_id.to_string());
                            }
                        }
                    }
                }
                "item/agentMessage/delta" | "agentMessage/delta" => {
                    if params.get("turnId").and_then(Value::as_str) == Some(&session.turn_id)
                        && params
                            .get("itemId")
                            .and_then(Value::as_str)
                            .is_some_and(|item_id| session.final_message_item_ids.contains(item_id))
                    {
                        if let Some(delta) = params.get("delta").and_then(Value::as_str) {
                            session.assistant_text.push_str(delta);
                            session.received_agent_delta = true;
                        }
                    }
                }
                "item/completed" => {
                    if params.get("turnId").and_then(Value::as_str) == Some(&session.turn_id)
                        && !session.received_agent_delta
                        && params
                            .pointer("/item/id")
                            .and_then(Value::as_str)
                            .is_some_and(|item_id| session.final_message_item_ids.contains(item_id))
                    {
                        session
                            .assistant_text
                            .push_str(&codex_item_text(params.get("item").unwrap_or(&Value::Null)));
                    }
                }
                "item/tool/call" => {
                    let call = codex_dynamic_tool_call(&event)?;
                    if call.call_id.is_empty() || call.name.is_empty() {
                        anyhow::bail!("Codex App Server returned an incomplete dynamic tool call");
                    }
                    return Ok(CodexDriveResult::ToolCall(call));
                }
                "turn/completed" => {
                    if params.pointer("/turn/id").and_then(Value::as_str) != Some(&session.turn_id)
                    {
                        continue;
                    }
                    let status = params
                        .pointer("/turn/status")
                        .and_then(Value::as_str)
                        .unwrap_or("completed");
                    if status != "completed" {
                        let detail = params
                            .pointer("/turn/error/message")
                            .and_then(Value::as_str)
                            .unwrap_or(status);
                        anyhow::bail!("Codex App Server turn failed: {detail}");
                    }
                    return Ok(CodexDriveResult::Completed(session.assistant_text.clone()));
                }
                "error" => {
                    let error = params.get("error").unwrap_or(&Value::Null);
                    let will_retry = params
                        .get("willRetry")
                        .and_then(Value::as_bool)
                        // Older App Server builds nested this field in the error payload.
                        .or_else(|| error.get("willRetry").and_then(Value::as_bool))
                        .unwrap_or(false);
                    let detail = error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("Codex App Server reported an error");
                    if will_retry {
                        if session.network_retry_count >= PROVIDER_NETWORK_RETRY_LIMIT {
                            anyhow::bail!(
                                "Codex App Server exceeded {PROVIDER_NETWORK_RETRY_LIMIT} reconnect attempts: {detail}"
                            );
                        }
                        session.network_retry_count += 1;
                        on_transport(ProviderTransportEvent::Retry {
                            attempt: session.network_retry_count + 1,
                            retry_kind: ProviderRetryKind::Network,
                            retry_index: Some(session.network_retry_count),
                            retry_limit: Some(PROVIDER_NETWORK_RETRY_LIMIT),
                            reason: detail.to_string(),
                            cache_trace: None,
                            body: redact_transport_value(error),
                        })?;
                    } else {
                        anyhow::bail!("Codex App Server error: {detail}");
                    }
                }
                _ => {}
            }
        }
    }

    async fn complete_request(
        &self,
        request: ModelRequest,
        on_transport: &mut ProviderTransportCallback<'_>,
    ) -> anyhow::Result<(ModelResponse, usize)> {
        let previous_response_id = request.previous_response_id.clone();
        let mut session = match previous_response_id.as_deref() {
            Some(response_id) => self.sessions.lock().await.remove(response_id),
            None => None,
        }
        .unwrap_or(self.start_session(&request).await?);
        session.network_retry_count = 0;

        let outcome = async {
            if previous_response_id.is_some() && session.pending_tool_call.is_some() {
                self.resume_session(&mut session, &request.input.tool_results)
                    .await?;
            }
            self.drive_session(&mut session, on_transport).await
        }
        .await;

        match outcome {
            Ok(CodexDriveResult::ToolCall(call)) => {
                let session_id = Uuid::new_v4().to_string();
                let response = ModelResponse {
                    text: String::new(),
                    tool_calls: vec![ProviderToolCall {
                        id: call.call_id.clone(),
                        name: call.name.clone(),
                        arguments: call.arguments.clone(),
                    }],
                    usage: None,
                    response_id: Some(session_id.clone()),
                    provider_items: Vec::new(),
                    finish_reason: ModelFinishReason::ToolCalls,
                };
                let attempt = session.network_retry_count + 1;
                session.pending_tool_call = Some(call);
                self.sessions.lock().await.insert(session_id, session);
                Ok((response, attempt))
            }
            Ok(CodexDriveResult::Completed(text)) => {
                session.cleanup().await;
                if text.trim().is_empty() {
                    anyhow::bail!("Codex App Server completed without an assistant message");
                }
                let response = ModelResponse::text(text);
                let attempt = session.network_retry_count + 1;
                Ok((response, attempt))
            }
            Err(error) => {
                session.cleanup().await;
                Err(error)
            }
        }
    }
}

impl CodexAppServerSession {
    async fn cleanup(&mut self) {
        let _ = self.stdin.shutdown().await;
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
        for path in self.attachment_paths.drain(..) {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[async_trait]
impl ModelProvider for CodexAppServerProvider {
    async fn complete(&self, request: ModelRequest) -> anyhow::Result<ModelResponse> {
        self.complete_request(request, &mut |_| Ok(()))
            .await
            .map(|(response, _)| response)
    }

    async fn stream_prepared(
        &self,
        prepared: PreparedProviderRequest,
        on_delta: &mut ModelStreamCallback<'_>,
        on_transport: &mut ProviderTransportCallback<'_>,
    ) -> anyhow::Result<ModelResponse> {
        let (response, attempt) = self
            .complete_request(prepared.logical_request, on_transport)
            .await?;
        if !response.text.is_empty() {
            on_delta(ModelStreamDelta::Text {
                text: response.text.clone(),
            })?;
        }
        for (index, call) in response.tool_calls.iter().enumerate() {
            on_delta(ModelStreamDelta::ToolCall {
                index,
                id: Some(call.id.clone()),
                name: Some(call.name.clone()),
                arguments_delta: call.arguments.to_string(),
            })?;
        }
        if let Some(usage) = &response.usage {
            on_delta(ModelStreamDelta::Usage {
                usage: usage.clone(),
            })?;
        }
        on_transport(ProviderTransportEvent::Response {
            attempt,
            status: None,
            response_id: response.response_id.clone(),
            body: model_response_observation(&response),
        })?;
        Ok(response)
    }

    async fn check_health(&self) -> anyhow::Result<ProviderHealthCheck> {
        let start = std::time::Instant::now();
        let mut command = codex_app_server_command();
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command
            .spawn()
            .context("failed to start the local Codex App Server")?;
        let mut stdin = child
            .stdin
            .take()
            .context("Codex App Server did not expose stdin")?;
        let stdout = child
            .stdout
            .take()
            .context("Codex App Server did not expose stdout")?;
        let mut stdout = BufReader::new(stdout).lines();
        let health = async {
            codex_write_rpc(
                &mut stdin,
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "initialize",
                    "params": {
                        "clientInfo": { "name": "OpenTopia", "version": env!("CARGO_PKG_VERSION") },
                        "capabilities": { "experimentalApi": true }
                    }
                }),
            )
            .await?;
            codex_wait_for_response(&mut stdout, 1).await
        }
        .await;
        let _ = stdin.shutdown().await;
        let _ = child.start_kill();
        let _ = child.wait().await;
        health?;
        Ok(ProviderHealthCheck {
            reachable: true,
            latency_ms: Some(start.elapsed().as_millis() as u64),
            model_available: true,
            error: None,
            openai_compatibility: None,
        })
    }
}

fn codex_app_server_command() -> Command {
    #[cfg(windows)]
    {
        if let Some(candidate) = latest_codex_desktop_binary() {
            let mut command = Command::new(candidate);
            command.arg("app-server");
            configure_codex_app_server_environment(&mut command);
            return command;
        }
        if let Some(app_data) = std::env::var_os("APPDATA") {
            let candidate = PathBuf::from(app_data).join("npm").join("codex.cmd");
            if candidate.is_file() {
                let mut command = Command::new("cmd.exe");
                command.args(["/d", "/s", "/c"]);
                command.arg(format!("\"\"{}\" app-server\"", candidate.display()));
                configure_codex_app_server_environment(&mut command);
                return command;
            }
        }
    }
    let mut command = Command::new(OsString::from("codex"));
    command.arg("app-server");
    configure_codex_app_server_environment(&mut command);
    command
}

/// A development host such as the Codex IDE can inject an isolated `CODEX_HOME`.
/// Do not let that host-only profile hide the user's normal Codex credentials
/// from the OpenTopia child process.
fn configure_codex_app_server_environment(command: &mut Command) {
    let codex_home = std::env::var_os("CODEX_HOME");
    let originator = std::env::var("CODEX_INTERNAL_ORIGINATOR_OVERRIDE").ok();
    let uses_host_profile = codex_home
        .as_deref()
        .is_some_and(|home| is_isolated_codex_host_profile(Path::new(home), originator.as_deref()));
    if !uses_host_profile {
        return;
    }

    for variable in [
        "CODEX_HOME",
        "CODEX_INTERNAL_ORIGINATOR_OVERRIDE",
        "CODEX_THREAD_ID",
        "CODEX_PERMISSION_PROFILE",
        "CODEX_SANDBOX_NETWORK_DISABLED",
    ] {
        command.env_remove(variable);
    }
}

pub(super) fn is_isolated_codex_host_profile(codex_home: &Path, originator: Option<&str>) -> bool {
    originator.is_some()
        && codex_home
            .file_name()
            .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("codex-api"))
}

#[cfg(windows)]
fn latest_codex_desktop_binary() -> Option<PathBuf> {
    let bin_root = PathBuf::from(std::env::var_os("LOCALAPPDATA")?)
        .join("OpenAI")
        .join("Codex")
        .join("bin");
    let mut candidates = vec![bin_root.join("codex.exe")];
    if let Ok(entries) = std::fs::read_dir(&bin_root) {
        candidates.extend(
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path().join("codex.exe")),
        );
    }
    candidates
        .into_iter()
        .filter(|path| path.is_file())
        .max_by_key(|path| {
            std::fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        })
}

async fn codex_write_rpc(stdin: &mut ChildStdin, value: Value) -> anyhow::Result<()> {
    let mut line = serde_json::to_vec(&value)?;
    line.push(b'\n');
    stdin.write_all(&line).await?;
    stdin.flush().await?;
    Ok(())
}

async fn codex_next_event(stdout: &mut Lines<BufReader<ChildStdout>>) -> anyhow::Result<Value> {
    let line = stdout
        .next_line()
        .await?
        .context("Codex App Server closed its output stream")?;
    serde_json::from_str(&line).context("Codex App Server emitted malformed JSON-RPC output")
}

async fn codex_wait_for_response(
    stdout: &mut Lines<BufReader<ChildStdout>>,
    response_id: u64,
) -> anyhow::Result<Value> {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let event = codex_next_event(stdout).await?;
            if event.get("id").and_then(Value::as_u64) != Some(response_id) {
                continue;
            }
            if let Some(error) = event.get("error") {
                anyhow::bail!("Codex App Server request failed: {error}");
            }
            return event
                .get("result")
                .cloned()
                .context("Codex App Server response omitted result");
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("Codex App Server did not respond within 30 seconds"))?
}

pub(super) fn codex_dynamic_tools(candidates: &[ProviderToolCandidate]) -> Vec<Value> {
    candidates
        .iter()
        .map(|candidate| {
            json!({
                "type": "function",
                "name": candidate.name,
                "description": candidate.description,
                "inputSchema": candidate.input_schema,
            })
        })
        .collect()
}

pub(super) fn is_codex_builtin_action(item_type: &str) -> bool {
    matches!(
        item_type,
        "commandExecution"
            | "fileChange"
            | "mcpToolCall"
            | "webSearch"
            | "imageView"
            | "collabToolCall"
    )
}

pub(super) fn codex_developer_instructions(
    request: &ModelRequest,
    native_web_search: bool,
) -> String {
    let mut sections = instruction_messages(request)
        .into_iter()
        .filter_map(|(role, content)| (role != ContextRole::User).then_some(content))
        .filter(|content| !content.trim().is_empty())
        .collect::<Vec<_>>();
    sections.push(if native_web_search && request.tool_candidates.is_empty() {
        "You are executing inside OpenTopia. Respond directly. You may use the built-in web search tool when current external information is needed; do not invoke other built-in tools."
            .to_string()
    } else if native_web_search {
        "You are executing inside OpenTopia. Use only the supplied OpenTopia dynamic tools and the built-in web search tool; do not invoke other built-in tools."
            .to_string()
    } else if request.tool_candidates.is_empty() {
        "You are executing inside OpenTopia. Respond directly and do not invoke built-in tools."
            .to_string()
    } else {
        "You are executing inside OpenTopia. Use only the supplied OpenTopia dynamic tools; do not invoke built-in tools."
            .to_string()
    });
    if native_web_search {
        sections.push(NATIVE_WEB_SEARCH_PRIORITY_INSTRUCTION.to_string());
    }
    sections.join("\n\n")
}

pub(super) fn codex_turn_input(
    request: &ModelRequest,
    attachment_paths: &mut Vec<PathBuf>,
) -> anyhow::Result<Vec<Value>> {
    let mut input = Vec::new();
    for message in &request.input.conversation {
        let role = match message.role {
            ModelConversationRole::System => "System",
            ModelConversationRole::User => "User",
            ModelConversationRole::Assistant => "Assistant",
            ModelConversationRole::Tool => "Tool",
        };
        let separator = if input.is_empty() { "" } else { "\n\n" };
        push_codex_input_text(
            &mut input,
            format!("{separator}{role}:\n{}", message.content),
        );
        if !message.content.is_empty() && !message.content_parts.is_empty() {
            push_codex_input_text(&mut input, "\n");
        }
        append_codex_input_parts(&mut input, &message.content_parts, attachment_paths)?;
    }

    let separator = if input.is_empty() { "" } else { "\n\n" };
    push_codex_input_text(
        &mut input,
        format!(
            "{separator}Current user request:\n{}",
            request.input.current_user.message
        ),
    );
    if !request.input.current_user.message.is_empty()
        && !request.input.current_user.content.is_empty()
    {
        push_codex_input_text(&mut input, "\n");
    }
    append_codex_input_parts(
        &mut input,
        &request.input.current_user.content,
        attachment_paths,
    )?;

    let user_context = instruction_messages(request)
        .into_iter()
        .filter_map(|(role, content)| (role == ContextRole::User).then_some(content))
        .filter(|content| !content.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    if !user_context.is_empty() {
        push_codex_input_text(
            &mut input,
            format!("\n\nAdditional user context:\n{user_context}"),
        );
    }

    if !request.input.tool_results.is_empty() {
        push_codex_input_text(
            &mut input,
            format!(
                "\n\nCompleted OpenTopia tool results:\n{}",
                request
                    .input
                    .tool_results
                    .iter()
                    .map(provider_tool_result_content)
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
        );
        for result in &request.input.tool_results {
            for part in &result.content {
                if matches!(part, ModelContentPart::Image { .. }) {
                    append_codex_input_parts(
                        &mut input,
                        std::slice::from_ref(part),
                        attachment_paths,
                    )?;
                }
            }
        }
    }
    Ok(input)
}

fn push_codex_input_text(input: &mut Vec<Value>, text: impl Into<String>) {
    let text = text.into();
    if text.is_empty() {
        return;
    }
    if let Some(existing) = input
        .last_mut()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("text"))
    {
        if let Some(current) = existing.get("text").and_then(Value::as_str) {
            let combined = format!("{current}{text}");
            existing["text"] = Value::String(combined);
            return;
        }
    }
    input.push(json!({ "type": "text", "text": text }));
}

fn append_codex_input_parts(
    input: &mut Vec<Value>,
    parts: &[ModelContentPart],
    attachment_paths: &mut Vec<PathBuf>,
) -> anyhow::Result<()> {
    for part in parts {
        match part {
            ModelContentPart::Text { text } => push_codex_input_text(input, text),
            ModelContentPart::Json { value } => {
                push_codex_input_text(input, value.to_string());
            }
            ModelContentPart::Image { content_type, data } => {
                let path = materialize_codex_image(content_type, data)?;
                input.push(json!({
                    "type": "localImage",
                    "path": path,
                    "detail": "original",
                }));
                attachment_paths.push(path);
            }
            ModelContentPart::Resource {
                uri,
                content_type,
                name,
            } => push_codex_input_text(
                input,
                resource_fallback_text(uri, content_type.as_deref(), name.as_deref()),
            ),
        }
    }
    Ok(())
}

fn materialize_codex_image(content_type: &str, data: &[u8]) -> anyhow::Result<PathBuf> {
    let extension = match content_type.to_ascii_lowercase().as_str() {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/avif" => "avif",
        _ => "img",
    };
    let directory = std::env::temp_dir().join("opentopia-codex-attachments");
    std::fs::create_dir_all(&directory)?;
    let path = directory.join(format!("{}.{}", Uuid::new_v4(), extension));
    std::fs::write(&path, data)?;
    path.canonicalize()
        .with_context(|| format!("failed to resolve Codex attachment {}", path.display()))
}

pub(super) fn codex_dynamic_tool_call(event: &Value) -> anyhow::Result<CodexDynamicToolCall> {
    let params = event
        .get("params")
        .context("Codex dynamic tool call omitted params")?;
    let name = params
        .get("tool")
        .and_then(Value::as_str)
        .or_else(|| params.pointer("/tool/name").and_then(Value::as_str))
        .context("Codex dynamic tool call omitted tool name")?
        .to_string();
    let arguments = match params.get("arguments") {
        Some(Value::String(value)) => {
            serde_json::from_str(&value).unwrap_or_else(|_| Value::String(value.clone()))
        }
        Some(value) => value.clone(),
        None => json!({}),
    };
    Ok(CodexDynamicToolCall {
        rpc_id: event
            .get("id")
            .cloned()
            .context("Codex dynamic tool call omitted request id")?,
        call_id: params
            .get("callId")
            .and_then(Value::as_str)
            .context("Codex dynamic tool call omitted callId")?
            .to_string(),
        name,
        arguments,
    })
}

pub(super) fn codex_item_text(item: &Value) -> String {
    if item.get("type").and_then(Value::as_str) != Some("agentMessage") {
        return String::new();
    }
    if let Some(text) = item.get("text").and_then(Value::as_str) {
        return text.to_string();
    }
    item.get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<String>()
}
