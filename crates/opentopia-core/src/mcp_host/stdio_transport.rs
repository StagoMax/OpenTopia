use super::McpHostError;
use crate::execution::{ExecRequest, ExecutionContext, ExecutionEnvironment, StdioSession};
use crate::execution_authorization::ProcessLifetime;
use crate::mcp::{mcp_default_input_schema, McpServerConfig};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{
    duplex, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
};
use tokio::process::{Child, Command};
use tokio::sync::{oneshot, Mutex};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tracing::{debug, trace, warn};
use uuid::Uuid;
pub(super) const MCP_PROTOCOL_VERSION: &str = "2024-11-05";
const MCP_CLIENT_NAME: &str = "opentopia";
const MCP_MAX_TOOL_LIST_PAGES: usize = 128;

type PendingMap = Arc<Mutex<HashMap<u64, PendingRequest>>>;
type StdinWriter = Arc<Mutex<Box<dyn AsyncWrite + Unpin + Send>>>;
type BoxedReader = Box<dyn AsyncRead + Unpin + Send>;
type BoxedWriter = Box<dyn AsyncWrite + Unpin + Send>;
type ExecutionEnvironmentFactory =
    dyn Fn(&McpServerConfig) -> Arc<dyn ExecutionEnvironment> + Send + Sync;

pub(super) const JSON_RPC_METHOD_NOT_FOUND: i64 = -32601;

pub struct McpSpawnedProcess {
    stdin: BoxedWriter,
    stdout: BoxedReader,
    stderr: Option<BoxedReader>,
    process: Box<dyn McpChildProcess>,
}

impl McpSpawnedProcess {
    pub fn new<I, O, E, P>(stdin: I, stdout: O, stderr: Option<E>, process: P) -> Self
    where
        I: AsyncWrite + Unpin + Send + 'static,
        O: AsyncRead + Unpin + Send + 'static,
        E: AsyncRead + Unpin + Send + 'static,
        P: McpChildProcess + 'static,
    {
        Self {
            stdin: Box::new(stdin),
            stdout: Box::new(stdout),
            stderr: stderr.map(|stream| Box::new(stream) as BoxedReader),
            process: Box::new(process),
        }
    }
}

#[async_trait]
pub trait McpChildProcess: Send {
    async fn kill(&mut self) -> Result<(), McpHostError>;
    async fn wait(&mut self) -> Result<(), McpHostError>;
    fn start_kill(&mut self);
}

#[async_trait]
pub trait McpProcessSpawner: Send + Sync {
    async fn spawn(&self, config: &McpServerConfig) -> Result<McpSpawnedProcess, McpHostError>;
}

#[derive(Debug, Default)]
pub struct SecureLocalMcpProcessSpawner;

#[async_trait]
impl McpProcessSpawner for SecureLocalMcpProcessSpawner {
    async fn spawn(&self, config: &McpServerConfig) -> Result<McpSpawnedProcess, McpHostError> {
        let mut command = Command::new(&config.command);
        command
            .args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(cwd) = &config.cwd {
            command.current_dir(cwd);
        }
        configure_child_environment(&mut command, config);

        let mut child = command.spawn().map_err(|source| McpHostError::Spawn {
            server_name: config.name.clone(),
            source,
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpHostError::MissingPipe {
                server_name: config.name.clone(),
                stream: "stdin",
            })?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpHostError::MissingPipe {
                server_name: config.name.clone(),
                stream: "stdout",
            })?;
        let stderr = child
            .stderr
            .take()
            .map(|pipe| Box::new(pipe) as BoxedReader);

        Ok(McpSpawnedProcess {
            stdin: Box::new(stdin),
            stdout: Box::new(stdout),
            stderr,
            process: Box::new(TokioMcpChildProcess(child)),
        })
    }
}

struct TokioMcpChildProcess(Child);

#[async_trait]
impl McpChildProcess for TokioMcpChildProcess {
    async fn kill(&mut self) -> Result<(), McpHostError> {
        self.0.kill().await.map_err(McpHostError::Io)
    }

    async fn wait(&mut self) -> Result<(), McpHostError> {
        self.0.wait().await.map(|_| ()).map_err(McpHostError::Io)
    }

    fn start_kill(&mut self) {
        let _ = self.0.start_kill();
    }
}

pub struct ExecutionEnvironmentMcpProcessSpawner {
    environment_factory: Arc<ExecutionEnvironmentFactory>,
}

impl ExecutionEnvironmentMcpProcessSpawner {
    pub fn new(environment: Arc<dyn ExecutionEnvironment>) -> Self {
        Self::with_factory(move |_| environment.clone())
    }

    pub fn with_factory<F>(factory: F) -> Self
    where
        F: Fn(&McpServerConfig) -> Arc<dyn ExecutionEnvironment> + Send + Sync + 'static,
    {
        Self {
            environment_factory: Arc::new(factory),
        }
    }
}

#[async_trait]
impl McpProcessSpawner for ExecutionEnvironmentMcpProcessSpawner {
    async fn spawn(&self, config: &McpServerConfig) -> Result<McpSpawnedProcess, McpHostError> {
        let mut request = ExecRequest::new(config.command.clone())
            .args(config.args.clone())
            .env_clear()
            .envs(child_environment(config, |key| std::env::var_os(key)));
        if let Some(cwd) = &config.cwd {
            request = request.cwd(cwd.clone());
        }

        let environment = (self.environment_factory)(config);
        let session = environment
            .spawn_stdio(
                request,
                ExecutionContext::with_timeout(Duration::from_millis(config.timeout_ms.max(1)))
                    .with_process_lifetime(ProcessLifetime::PersistentService),
            )
            .await
            .map_err(|error| McpHostError::SpawnRejected {
                server_name: config.name.clone(),
                message: error.to_string(),
            })?;
        Ok(bridge_stdio_session(session))
    }
}

struct BridgedMcpChildProcess {
    session: Arc<dyn StdioSession>,
    pump_tasks: Vec<JoinHandle<()>>,
}

#[async_trait]
impl McpChildProcess for BridgedMcpChildProcess {
    async fn kill(&mut self) -> Result<(), McpHostError> {
        let result = self.session.kill().await;
        for task in &self.pump_tasks {
            task.abort();
        }
        result.map_err(|error| McpHostError::TransportClosed(error.to_string()))
    }

    async fn wait(&mut self) -> Result<(), McpHostError> {
        let result = self.session.close().await;
        for task in &self.pump_tasks {
            task.abort();
        }
        result
            .map(|_| ())
            .map_err(|error| McpHostError::TransportClosed(error.to_string()))
    }

    fn start_kill(&mut self) {
        let session = self.session.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = session.kill().await;
            });
        }
        for task in &self.pump_tasks {
            task.abort();
        }
    }
}

fn bridge_stdio_session(session: Box<dyn StdioSession>) -> McpSpawnedProcess {
    const PIPE_CAPACITY: usize = 64 * 1024;

    let session: Arc<dyn StdioSession> = Arc::from(session);
    let (stdin, mut stdin_reader) = duplex(PIPE_CAPACITY);
    let (mut stdout_writer, stdout) = duplex(PIPE_CAPACITY);
    let (mut stderr_writer, stderr) = duplex(PIPE_CAPACITY);

    let stdin_session = session.clone();
    let stdin_task = tokio::spawn(async move {
        let mut buffer = [0_u8; 8192];
        loop {
            match stdin_reader.read(&mut buffer).await {
                Ok(0) => break,
                Ok(read) => {
                    if stdin_session.write_stdin(&buffer[..read]).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let stdout_session = session.clone();
    let stdout_task = tokio::spawn(async move {
        loop {
            match stdout_session.read_stdout().await {
                Ok(bytes) if bytes.is_empty() => break,
                Ok(bytes) => {
                    if stdout_writer.write_all(&bytes).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let stderr_session = session.clone();
    let stderr_task = tokio::spawn(async move {
        loop {
            match stderr_session.read_stderr().await {
                Ok(bytes) if bytes.is_empty() => break,
                Ok(bytes) => {
                    if stderr_writer.write_all(&bytes).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    McpSpawnedProcess {
        stdin: Box::new(stdin),
        stdout: Box::new(stdout),
        stderr: Some(Box::new(stderr)),
        process: Box::new(BridgedMcpChildProcess {
            session,
            pump_tasks: vec![stdin_task, stdout_task, stderr_task],
        }),
    }
}

pub struct McpStdioClient {
    server_id: Uuid,
    server_name: String,
    stdin: StdinWriter,
    child: Mutex<Option<Box<dyn McpChildProcess>>>,
    pending: PendingMap,
    next_id: AtomicU64,
    timeout: Duration,
    reader_task: JoinHandle<()>,
    stderr_task: JoinHandle<()>,
    tools_generation: Arc<AtomicU64>,
}

impl McpStdioClient {
    pub async fn spawn(config: McpServerConfig) -> Result<Self, McpHostError> {
        Self::spawn_with(config, Arc::new(SecureLocalMcpProcessSpawner)).await
    }

    pub async fn spawn_with(
        config: McpServerConfig,
        spawner: Arc<dyn McpProcessSpawner>,
    ) -> Result<Self, McpHostError> {
        if !config.enabled {
            return Err(McpHostError::Disabled {
                server_id: config.server_id,
            });
        }
        if config.command.trim().is_empty() {
            return Err(McpHostError::EmptyCommand {
                server_id: config.server_id,
            });
        }

        let spawned = spawner.spawn(&config).await?;

        Ok(Self::from_parts(
            config.server_id,
            config.name,
            config.timeout_ms,
            Some(spawned.process),
            spawned.stdout,
            spawned.stdin,
            spawned.stderr,
        ))
    }

    pub fn server_id(&self) -> Uuid {
        self.server_id
    }

    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    pub fn tools_generation(&self) -> u64 {
        self.tools_generation.load(Ordering::Acquire)
    }

    pub async fn initialize(&self) -> Result<Value, McpHostError> {
        let result = self
            .request(
                "initialize",
                Some(json!({
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": {
                        "name": MCP_CLIENT_NAME,
                        "version": env!("CARGO_PKG_VERSION")
                    }
                })),
            )
            .await?;
        self.notify("notifications/initialized", None).await?;
        Ok(result)
    }

    pub async fn list_tools(&self) -> Result<Vec<McpRawTool>, McpHostError> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;

        for _ in 0..MCP_MAX_TOOL_LIST_PAGES {
            let params = cursor.as_ref().map(|cursor| json!({ "cursor": cursor }));
            let value = self.request("tools/list", params).await?;
            let response: McpToolsListResponse = serde_json::from_value(value)?;
            tools.extend(response.tools);

            match response
                .next_cursor
                .filter(|value| !value.trim().is_empty())
            {
                Some(next_cursor) => cursor = Some(next_cursor),
                None => return Ok(tools),
            }
        }

        Err(McpHostError::Protocol(format!(
            "tools/list exceeded {MCP_MAX_TOOL_LIST_PAGES} pages"
        )))
    }

    pub async fn call_tool(
        &self,
        tool_name: &str,
        arguments: Value,
    ) -> Result<McpToolCallResponse, McpHostError> {
        let arguments = if arguments.is_null() {
            json!({})
        } else {
            arguments
        };
        let raw = self
            .request(
                "tools/call",
                Some(json!({
                    "name": tool_name,
                    "arguments": arguments
                })),
            )
            .await?;
        let response: McpToolCallWireResponse = serde_json::from_value(raw.clone())?;
        Ok(McpToolCallResponse {
            content: response.content,
            structured_content: response.structured_content,
            is_error: response.is_error,
            raw,
        })
    }

    pub async fn shutdown(&self) -> Result<(), McpHostError> {
        self.reader_task.abort();
        self.stderr_task.abort();

        {
            let mut stdin = self.stdin.lock().await;
            let _ = stdin.shutdown().await;
        }

        if let Some(mut child) = self.child.lock().await.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }

        fail_all_pending(&self.pending, || {
            McpHostError::TransportClosed("client shut down".to_string())
        })
        .await;

        Ok(())
    }

    async fn request(&self, method: &str, params: Option<Value>) -> Result<Value, McpHostError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let operation = method.to_string();
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(
            id,
            PendingRequest {
                operation: operation.clone(),
                sender,
            },
        );

        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method,
            params,
        };
        let mut bytes = serde_json::to_vec(&request)?;
        bytes.push(b'\n');

        match timeout(self.timeout, self.write_message(bytes)).await {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                self.pending.lock().await.remove(&id);
                return Err(err);
            }
            Err(_) => {
                self.pending.lock().await.remove(&id);
                return Err(self.timeout_error(operation));
            }
        }

        match timeout(self.timeout, receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(McpHostError::TransportClosed(format!(
                "request {method} was dropped before a response"
            ))),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(self.timeout_error(method.to_string()))
            }
        }
    }

    async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), McpHostError> {
        let notification = JsonRpcNotification {
            jsonrpc: "2.0",
            method,
            params,
        };
        let mut bytes = serde_json::to_vec(&notification)?;
        bytes.push(b'\n');

        match timeout(self.timeout, self.write_message(bytes)).await {
            Ok(result) => result,
            Err(_) => Err(self.timeout_error(method.to_string())),
        }
    }

    async fn write_message(&self, bytes: Vec<u8>) -> Result<(), McpHostError> {
        write_message(&self.stdin, &bytes).await
    }

    fn timeout_error(&self, operation: String) -> McpHostError {
        McpHostError::Timeout {
            operation,
            timeout_ms: duration_millis(self.timeout),
        }
    }

    fn from_parts<R, W, E>(
        server_id: Uuid,
        server_name: String,
        timeout_ms: u64,
        child: Option<Box<dyn McpChildProcess>>,
        stdout: R,
        stdin: W,
        stderr: Option<E>,
    ) -> Self
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
        E: AsyncRead + Unpin + Send + 'static,
    {
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let stdin: StdinWriter = Arc::new(Mutex::new(Box::new(stdin)));
        let timeout = Duration::from_millis(timeout_ms.max(1));
        let tools_generation = Arc::new(AtomicU64::new(0));
        let reader_task = spawn_stdout_reader(
            stdout,
            stdin.clone(),
            pending.clone(),
            server_id,
            server_name.clone(),
            timeout,
            tools_generation.clone(),
        );
        let stderr_task = match stderr {
            Some(stderr) => spawn_stderr_reader(stderr, server_id, server_name.clone()),
            None => tokio::spawn(async {}),
        };

        Self {
            server_id,
            server_name,
            stdin,
            child: Mutex::new(child),
            pending,
            next_id: AtomicU64::new(1),
            timeout,
            reader_task,
            stderr_task,
            tools_generation,
        }
    }

    #[cfg(test)]
    pub(super) fn from_io_for_test<R, W>(config: McpServerConfig, stdout: R, stdin: W) -> Self
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        Self::from_parts(
            config.server_id,
            config.name,
            config.timeout_ms,
            None,
            stdout,
            stdin,
            Option::<tokio::io::Empty>::None,
        )
    }
}

impl Drop for McpStdioClient {
    fn drop(&mut self) {
        self.reader_task.abort();
        self.stderr_task.abort();
        if let Ok(mut child) = self.child.try_lock() {
            if let Some(child) = child.as_mut() {
                child.start_kill();
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcWireError {
    pub code: i64,
    pub message: String,
    #[serde(default)]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum McpIncomingMessage {
    Response {
        id: u64,
        result: Result<Value, JsonRpcWireError>,
    },
    Notification {
        method: String,
        params: Option<Value>,
    },
    Request {
        id: Value,
        method: String,
        params: Option<Value>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpRawTool {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "mcp_default_input_schema")]
    pub input_schema: Value,
    #[serde(default = "empty_object")]
    pub annotations: Value,
    #[serde(default = "empty_object", rename = "_meta")]
    pub meta: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolCallResponse {
    pub content: Vec<Value>,
    pub structured_content: Option<Value>,
    pub is_error: bool,
    pub raw: Value,
}

#[derive(Debug, Serialize)]
struct JsonRpcRequest<'a> {
    jsonrpc: &'static str,
    id: u64,
    method: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcNotification<'a> {
    jsonrpc: &'static str,
    method: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum JsonRpcResponse {
    Success {
        jsonrpc: &'static str,
        id: Value,
        result: Value,
    },
    Error {
        jsonrpc: &'static str,
        id: Value,
        error: JsonRpcWireError,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct McpToolsListResponse {
    #[serde(default)]
    tools: Vec<McpRawTool>,
    #[serde(default)]
    next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct McpToolCallWireResponse {
    #[serde(default)]
    content: Vec<Value>,
    #[serde(default)]
    structured_content: Option<Value>,
    #[serde(default)]
    is_error: bool,
}

struct PendingRequest {
    #[allow(dead_code)]
    operation: String,
    sender: oneshot::Sender<Result<Value, McpHostError>>,
}

fn spawn_stdout_reader<R>(
    stdout: R,
    stdin: StdinWriter,
    pending: PendingMap,
    server_id: Uuid,
    server_name: String,
    request_timeout: Duration,
    tools_generation: Arc<AtomicU64>,
) -> JoinHandle<()>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    if line.trim().is_empty() {
                        continue;
                    }
                    match parse_json_rpc_line(&line) {
                        Ok(McpIncomingMessage::Response { id, result }) => {
                            let pending_request = pending.lock().await.remove(&id);
                            if let Some(pending_request) = pending_request {
                                let result = result.map_err(|error| McpHostError::JsonRpc {
                                    code: error.code,
                                    message: error.message,
                                    data: error.data,
                                });
                                let _ = pending_request.sender.send(result);
                            } else {
                                trace!(%server_id, %server_name, id, "received response for unknown MCP request id");
                            }
                        }
                        Ok(McpIncomingMessage::Notification { method, .. }) => {
                            if method == "notifications/tools/list_changed" {
                                tools_generation.fetch_add(1, Ordering::AcqRel);
                                debug!(%server_id, %server_name, "invalidated MCP tool schema cache");
                            }
                            trace!(%server_id, %server_name, %method, "received MCP notification");
                        }
                        Ok(McpIncomingMessage::Request { id, method, .. }) => {
                            let response = response_for_server_request(id, &method);
                            match serialize_json_line(&response) {
                                Ok(bytes) => {
                                    match timeout(request_timeout, write_message(&stdin, &bytes))
                                        .await
                                    {
                                        Ok(Ok(())) => {
                                            trace!(%server_id, %server_name, %method, "answered MCP server request");
                                        }
                                        Ok(Err(err)) => {
                                            warn!(%server_id, %server_name, %method, error = %err, "failed to answer MCP server request");
                                        }
                                        Err(_) => {
                                            warn!(%server_id, %server_name, %method, "timed out answering MCP server request");
                                        }
                                    }
                                }
                                Err(err) => {
                                    warn!(%server_id, %server_name, %method, error = %err, "failed to serialize MCP server request response");
                                }
                            }
                        }
                        Err(err) => {
                            let message = err.to_string();
                            warn!(%server_id, %server_name, %message, "failed to parse MCP stdout message");
                            fail_all_pending(&pending, || McpHostError::Protocol(message.clone()))
                                .await;
                        }
                    }
                }
                Ok(None) => {
                    debug!(%server_id, %server_name, "MCP stdout closed");
                    fail_all_pending(&pending, || {
                        McpHostError::TransportClosed("stdio stdout closed".to_string())
                    })
                    .await;
                    break;
                }
                Err(err) => {
                    let message = err.to_string();
                    warn!(%server_id, %server_name, %message, "failed to read MCP stdout");
                    fail_all_pending(&pending, || McpHostError::TransportClosed(message.clone()))
                        .await;
                    break;
                }
            }
        }
    })
}

async fn write_message(stdin: &StdinWriter, bytes: &[u8]) -> Result<(), McpHostError> {
    let mut stdin = stdin.lock().await;
    stdin.write_all(bytes).await?;
    stdin.flush().await?;
    Ok(())
}

fn serialize_json_line<T: Serialize>(message: &T) -> Result<Vec<u8>, serde_json::Error> {
    let mut bytes = serde_json::to_vec(message)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn response_for_server_request(id: Value, method: &str) -> JsonRpcResponse {
    if method == "ping" {
        return JsonRpcResponse::Success {
            jsonrpc: "2.0",
            id,
            result: empty_object(),
        };
    }

    let capability = match method {
        "sampling/createMessage" => Some("sampling"),
        "roots/list" => Some("roots"),
        method if method.starts_with("elicitation/") => Some("elicitation"),
        _ => None,
    };
    let data = match capability {
        Some(capability) => json!({
            "method": method,
            "capability": capability,
            "reason": "client capability is not supported or advertised"
        }),
        None => json!({ "method": method }),
    };

    JsonRpcResponse::Error {
        jsonrpc: "2.0",
        id,
        error: JsonRpcWireError {
            code: JSON_RPC_METHOD_NOT_FOUND,
            message: "Method not found".to_string(),
            data: Some(data),
        },
    }
}

fn spawn_stderr_reader<R>(stderr: R, server_id: Uuid, server_name: String) -> JoinHandle<()>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            debug!(%server_id, %server_name, message = %line, "MCP stderr");
        }
    })
}

async fn fail_all_pending<F>(pending: &PendingMap, make_error: F)
where
    F: Fn() -> McpHostError,
{
    let pending_requests = pending
        .lock()
        .await
        .drain()
        .map(|(_, request)| request)
        .collect::<Vec<_>>();
    for request in pending_requests {
        let _ = request.sender.send(Err(make_error()));
    }
}

pub fn parse_json_rpc_line(line: &str) -> Result<McpIncomingMessage, McpHostError> {
    let value: Value = serde_json::from_str(line.trim())?;
    let object = value
        .as_object()
        .ok_or_else(|| McpHostError::Protocol("JSON-RPC message must be an object".to_string()))?;

    let id = object.get("id").cloned();
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .map(str::to_string);

    if let Some(id) = id {
        if object.contains_key("result") || object.contains_key("error") {
            let id = id.as_u64().ok_or_else(|| {
                McpHostError::Protocol(
                    "JSON-RPC response id must be an unsigned integer".to_string(),
                )
            })?;
            if let Some(error) = object.get("error") {
                let error: JsonRpcWireError = serde_json::from_value(error.clone())?;
                return Ok(McpIncomingMessage::Response {
                    id,
                    result: Err(error),
                });
            }
            return Ok(McpIncomingMessage::Response {
                id,
                result: Ok(object.get("result").cloned().unwrap_or(Value::Null)),
            });
        }

        if let Some(method) = method {
            return Ok(McpIncomingMessage::Request {
                id,
                method,
                params: object.get("params").cloned(),
            });
        }

        return Err(McpHostError::Protocol(
            "JSON-RPC message with id must be a response or request".to_string(),
        ));
    }

    if let Some(method) = method {
        return Ok(McpIncomingMessage::Notification {
            method,
            params: object.get("params").cloned(),
        });
    }

    Err(McpHostError::Protocol(
        "JSON-RPC message must include method or id".to_string(),
    ))
}

fn configure_child_environment(command: &mut Command, config: &McpServerConfig) {
    command.env_clear();
    command.envs(child_environment(config, |key| std::env::var_os(key)));
}

pub(super) fn child_environment<F>(
    config: &McpServerConfig,
    inherited: F,
) -> Vec<(OsString, OsString)>
where
    F: Fn(&OsStr) -> Option<OsString>,
{
    let mut variables = HashMap::<String, (OsString, OsString)>::new();
    for key in base_environment_keys()
        .iter()
        .copied()
        .chain(config.env_keys.iter().map(String::as_str))
    {
        let key = OsStr::new(key);
        if let Some(value) = inherited(key) {
            variables.insert(environment_key_identity(key), (key.to_os_string(), value));
        }
    }
    variables.into_values().collect()
}

#[cfg(windows)]
pub(super) fn environment_key_identity(key: &OsStr) -> String {
    key.to_string_lossy().to_ascii_uppercase()
}

#[cfg(not(windows))]
pub(super) fn environment_key_identity(key: &OsStr) -> String {
    key.to_string_lossy().into_owned()
}

#[cfg(windows)]
fn base_environment_keys() -> &'static [&'static str] {
    &[
        "PATH",
        "Path",
        "PATHEXT",
        "SystemRoot",
        "WINDIR",
        "COMSPEC",
        "TEMP",
        "TMP",
    ]
}

#[cfg(not(windows))]
fn base_environment_keys() -> &'static [&'static str] {
    &["PATH", "TMPDIR", "LANG", "LC_ALL"]
}

pub(super) fn empty_object() -> Value {
    json!({})
}

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}
