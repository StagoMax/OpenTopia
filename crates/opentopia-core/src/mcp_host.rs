use crate::execution::{ExecRequest, ExecutionContext, ExecutionEnvironment, StdioSession};
use crate::mcp::{
    mcp_default_input_schema, mcp_public_tool_name, McpCallResult, McpLifecycleStatus,
    McpServerConfig, McpServerStatus, McpToolDescriptor,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::io::{
    duplex, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
};
use tokio::process::{Child, Command};
use tokio::sync::{oneshot, Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tracing::{debug, trace, warn};
use uuid::Uuid;

const MCP_PROTOCOL_VERSION: &str = "2024-11-05";
const MCP_CLIENT_NAME: &str = "opentopia";
const MCP_MAX_TOOL_LIST_PAGES: usize = 128;

type PendingMap = Arc<Mutex<HashMap<u64, PendingRequest>>>;
type StdinWriter = Arc<Mutex<Box<dyn AsyncWrite + Unpin + Send>>>;
type BoxedReader = Box<dyn AsyncRead + Unpin + Send>;
type BoxedWriter = Box<dyn AsyncWrite + Unpin + Send>;
type ExecutionEnvironmentFactory =
    dyn Fn(&McpServerConfig) -> Arc<dyn ExecutionEnvironment> + Send + Sync;

const JSON_RPC_METHOD_NOT_FOUND: i64 = -32601;

#[derive(Debug, Error)]
pub enum McpHostError {
    #[error("MCP server {server_id} is disabled")]
    Disabled { server_id: Uuid },
    #[error("MCP stdio command is empty for server {server_id}")]
    EmptyCommand { server_id: Uuid },
    #[error("failed to spawn MCP server {server_name}: {source}")]
    Spawn {
        server_name: String,
        #[source]
        source: std::io::Error,
    },
    #[error("MCP process spawner rejected server {server_name}: {message}")]
    SpawnRejected {
        server_name: String,
        message: String,
    },
    #[error("MCP server {server_name} did not expose {stream} pipe")]
    MissingPipe {
        server_name: String,
        stream: &'static str,
    },
    #[error("MCP {operation} timed out after {timeout_ms}ms")]
    Timeout { operation: String, timeout_ms: u64 },
    #[error("MCP transport closed: {0}")]
    TransportClosed(String),
    #[error("MCP JSON-RPC error {code}: {message}")]
    JsonRpc {
        code: i64,
        message: String,
        data: Option<Value>,
    },
    #[error("MCP protocol error: {0}")]
    Protocol(String),
    #[error("MCP server not found: {0}")]
    ServerNotFound(Uuid),
    #[error("MCP tool not found: {0}")]
    ToolNotFound(String),
    #[error("duplicate public MCP tool name: {0}")]
    DuplicateToolName(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Clone)]
pub struct McpExtensionHost {
    inner: Arc<RwLock<McpExtensionHostInner>>,
    spawner: Arc<dyn McpProcessSpawner>,
}

impl Default for McpExtensionHost {
    fn default() -> Self {
        Self::new()
    }
}

impl McpExtensionHost {
    pub fn new() -> Self {
        Self::with_spawner(Arc::new(SecureLocalMcpProcessSpawner))
    }

    pub fn with_spawner(spawner: Arc<dyn McpProcessSpawner>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(McpExtensionHostInner::default())),
            spawner,
        }
    }

    pub fn with_execution_environment(environment: Arc<dyn ExecutionEnvironment>) -> Self {
        Self::with_spawner(Arc::new(ExecutionEnvironmentMcpProcessSpawner::new(
            environment,
        )))
    }

    pub fn with_execution_environment_factory<F>(factory: F) -> Self
    where
        F: Fn(&McpServerConfig) -> Arc<dyn ExecutionEnvironment> + Send + Sync + 'static,
    {
        Self::with_spawner(Arc::new(
            ExecutionEnvironmentMcpProcessSpawner::with_factory(factory),
        ))
    }

    pub async fn restart_server(
        &self,
        config: McpServerConfig,
    ) -> Result<McpServerStatus, McpHostError> {
        self.stop_server(config.server_id).await?;

        if !config.enabled {
            let status = McpServerStatus {
                server_id: config.server_id,
                name: config.name.clone(),
                status: McpLifecycleStatus::Disabled,
                message: "MCP server is disabled.".to_string(),
                tools_count: 0,
                updated_at: chrono::Utc::now(),
            };
            self.set_status(status.clone()).await;
            return Ok(status);
        }

        self.set_status(McpServerStatus {
            server_id: config.server_id,
            name: config.name.clone(),
            status: McpLifecycleStatus::Starting,
            message: "Starting MCP stdio server.".to_string(),
            tools_count: 0,
            updated_at: chrono::Utc::now(),
        })
        .await;

        let client = match McpStdioClient::spawn_with(config.clone(), self.spawner.clone()).await {
            Ok(client) => client,
            Err(err) => {
                self.set_error_status(&config, err.to_string()).await;
                return Err(err);
            }
        };

        if let Err(err) = client.initialize().await {
            let message = err.to_string();
            let _ = client.shutdown().await;
            self.set_error_status(&config, message).await;
            return Err(err);
        }

        let raw_tools = match client.list_tools().await {
            Ok(tools) => tools,
            Err(err) => {
                let message = err.to_string();
                let _ = client.shutdown().await;
                self.set_error_status(&config, message).await;
                return Err(err);
            }
        };

        let client = Arc::new(client);
        match self
            .install_ready_client(config.clone(), client.clone(), raw_tools)
            .await
        {
            Ok(status) => Ok(status),
            Err(err) => {
                let message = err.to_string();
                let _ = client.shutdown().await;
                self.set_error_status(&config, message).await;
                Err(err)
            }
        }
    }

    pub async fn stop_server(&self, server_id: Uuid) -> Result<(), McpHostError> {
        let runtime = {
            let mut inner = self.inner.write().await;
            inner
                .tool_routes
                .retain(|_, route| route.server_id != server_id);
            inner.servers.remove(&server_id)
        };

        if let Some(runtime) = runtime {
            runtime.client.shutdown().await?;
        }

        Ok(())
    }

    pub async fn status_for_config(&self, config: &McpServerConfig) -> McpServerStatus {
        let inner = self.inner.read().await;
        inner
            .statuses
            .get(&config.server_id)
            .cloned()
            .unwrap_or_else(|| McpServerStatus::from_config(config))
    }

    pub async fn is_ready(&self, server_id: Uuid) -> bool {
        let inner = self.inner.read().await;
        inner
            .statuses
            .get(&server_id)
            .is_some_and(|status| matches!(status.status, McpLifecycleStatus::Ready))
    }

    pub async fn list_tools(
        &self,
        server_id: Uuid,
    ) -> Result<Vec<McpToolDescriptor>, McpHostError> {
        self.refresh_tools_if_invalidated(server_id).await?;
        let inner = self.inner.read().await;
        inner
            .servers
            .get(&server_id)
            .map(|runtime| runtime.tools.clone())
            .ok_or(McpHostError::ServerNotFound(server_id))
    }

    pub async fn cached_tools(&self, server_id: Uuid) -> Vec<McpToolDescriptor> {
        if let Err(err) = self.refresh_tools_if_invalidated(server_id).await {
            warn!(%server_id, error = %err, "failed to refresh invalidated MCP tool cache");
        }
        let inner = self.inner.read().await;
        inner
            .servers
            .get(&server_id)
            .map(|runtime| runtime.tools.clone())
            .unwrap_or_default()
    }

    pub async fn all_cached_tools(&self) -> Vec<McpToolDescriptor> {
        let server_ids = {
            let inner = self.inner.read().await;
            inner.servers.keys().copied().collect::<Vec<_>>()
        };
        for server_id in server_ids {
            if let Err(err) = self.refresh_tools_if_invalidated(server_id).await {
                warn!(%server_id, error = %err, "failed to refresh invalidated MCP tool cache");
            }
        }
        let inner = self.inner.read().await;
        let mut tools = inner
            .servers
            .values()
            .flat_map(|runtime| runtime.tools.clone())
            .collect::<Vec<_>>();
        tools.sort_by(|left, right| left.public_name.cmp(&right.public_name));
        tools
    }

    pub async fn call_tool(
        &self,
        public_name: &str,
        arguments: Value,
    ) -> Result<McpCallResult, McpHostError> {
        let server_ids = {
            let inner = self.inner.read().await;
            inner.servers.keys().copied().collect::<Vec<_>>()
        };
        for server_id in server_ids {
            self.refresh_tools_if_invalidated(server_id).await?;
        }
        let (route, client) = {
            let inner = self.inner.read().await;
            let route = inner
                .tool_routes
                .get(public_name)
                .cloned()
                .ok_or_else(|| McpHostError::ToolNotFound(public_name.to_string()))?;
            let runtime = inner
                .servers
                .get(&route.server_id)
                .ok_or(McpHostError::ServerNotFound(route.server_id))?;
            (route, runtime.client.clone())
        };

        let response = client.call_tool(&route.tool_name, arguments).await?;
        Ok(call_result_from_response(
            route.server_id,
            route.public_name,
            route.tool_name,
            response,
        ))
    }

    async fn set_status(&self, status: McpServerStatus) {
        let mut inner = self.inner.write().await;
        inner.statuses.insert(status.server_id, status);
    }

    async fn refresh_tools_if_invalidated(&self, server_id: Uuid) -> Result<(), McpHostError> {
        let (config, client, cached_generation) = {
            let inner = self.inner.read().await;
            let runtime = inner
                .servers
                .get(&server_id)
                .ok_or(McpHostError::ServerNotFound(server_id))?;
            (
                runtime.config.clone(),
                runtime.client.clone(),
                runtime.tools_generation,
            )
        };

        if client.tools_generation() <= cached_generation {
            return Ok(());
        }

        let raw_tools = client.list_tools().await?;
        let descriptors = descriptors_from_raw_tools(&config, raw_tools)?;
        let observed_generation = client.tools_generation();

        let mut inner = self.inner.write().await;
        let Some(runtime) = inner.servers.get(&server_id) else {
            return Err(McpHostError::ServerNotFound(server_id));
        };
        if !Arc::ptr_eq(&runtime.client, &client) || runtime.tools_generation >= observed_generation
        {
            return Ok(());
        }

        for descriptor in &descriptors {
            if inner
                .tool_routes
                .get(&descriptor.public_name)
                .is_some_and(|route| route.server_id != server_id)
            {
                return Err(McpHostError::DuplicateToolName(
                    descriptor.public_name.clone(),
                ));
            }
        }

        inner
            .tool_routes
            .retain(|_, route| route.server_id != server_id);
        for descriptor in &descriptors {
            inner.tool_routes.insert(
                descriptor.public_name.clone(),
                McpToolRoute {
                    server_id,
                    public_name: descriptor.public_name.clone(),
                    tool_name: descriptor.tool_name.clone(),
                },
            );
        }

        if let Some(runtime) = inner.servers.get_mut(&server_id) {
            runtime.tools = descriptors;
            runtime.tools_generation = observed_generation;
        }
        let tools_count = inner
            .servers
            .get(&server_id)
            .map_or(0, |runtime| runtime.tools.len());
        if let Some(status) = inner.statuses.get_mut(&server_id) {
            status.tools_count = tools_count;
            status.message = "MCP tool schema cache refreshed.".to_string();
            status.updated_at = chrono::Utc::now();
        }

        Ok(())
    }

    async fn set_error_status(&self, config: &McpServerConfig, message: String) {
        self.set_status(McpServerStatus {
            server_id: config.server_id,
            name: config.name.clone(),
            status: McpLifecycleStatus::Error,
            message,
            tools_count: 0,
            updated_at: chrono::Utc::now(),
        })
        .await;
    }

    async fn install_ready_client(
        &self,
        config: McpServerConfig,
        client: Arc<McpStdioClient>,
        raw_tools: Vec<McpRawTool>,
    ) -> Result<McpServerStatus, McpHostError> {
        let descriptors = descriptors_from_raw_tools(&config, raw_tools)?;

        let status = McpServerStatus {
            server_id: config.server_id,
            name: config.name.clone(),
            status: McpLifecycleStatus::Ready,
            message: "MCP stdio server initialized.".to_string(),
            tools_count: descriptors.len(),
            updated_at: chrono::Utc::now(),
        };

        let mut inner = self.inner.write().await;
        inner
            .tool_routes
            .retain(|_, route| route.server_id != config.server_id);

        for descriptor in &descriptors {
            if inner.tool_routes.contains_key(&descriptor.public_name) {
                return Err(McpHostError::DuplicateToolName(
                    descriptor.public_name.clone(),
                ));
            }
        }

        for descriptor in &descriptors {
            inner.tool_routes.insert(
                descriptor.public_name.clone(),
                McpToolRoute {
                    server_id: config.server_id,
                    public_name: descriptor.public_name.clone(),
                    tool_name: descriptor.tool_name.clone(),
                },
            );
        }

        inner.servers.insert(
            config.server_id,
            McpServerRuntime {
                config,
                tools_generation: client.tools_generation(),
                client,
                tools: descriptors,
            },
        );
        inner.statuses.insert(status.server_id, status.clone());

        Ok(status)
    }

    #[cfg(test)]
    async fn install_client_for_test(
        &self,
        config: McpServerConfig,
        client: Arc<McpStdioClient>,
    ) -> Result<McpServerStatus, McpHostError> {
        let raw_tools = client.list_tools().await?;
        self.install_ready_client(config, client, raw_tools).await
    }
}

#[derive(Default)]
struct McpExtensionHostInner {
    servers: HashMap<Uuid, McpServerRuntime>,
    statuses: HashMap<Uuid, McpServerStatus>,
    tool_routes: HashMap<String, McpToolRoute>,
}

struct McpServerRuntime {
    config: McpServerConfig,
    client: Arc<McpStdioClient>,
    tools: Vec<McpToolDescriptor>,
    tools_generation: u64,
}

#[derive(Debug, Clone)]
pub struct McpToolRoute {
    pub server_id: Uuid,
    pub public_name: String,
    pub tool_name: String,
}

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
                ExecutionContext::with_timeout(Duration::from_millis(config.timeout_ms.max(1))),
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
    fn from_io_for_test<R, W>(config: McpServerConfig, stdout: R, stdin: W) -> Self
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

fn descriptor_from_raw_tool(config: &McpServerConfig, raw_tool: McpRawTool) -> McpToolDescriptor {
    McpToolDescriptor {
        public_name: mcp_public_tool_name(&config.name, &raw_tool.name),
        server_id: config.server_id,
        tool_name: raw_tool.name,
        description: raw_tool.description,
        input_schema: raw_tool.input_schema,
        annotations: raw_tool.annotations.clone(),
        permission_labels: permission_labels_from_annotations(&raw_tool.annotations),
    }
}

fn descriptors_from_raw_tools(
    config: &McpServerConfig,
    raw_tools: Vec<McpRawTool>,
) -> Result<Vec<McpToolDescriptor>, McpHostError> {
    let mut seen_public_names = HashSet::new();
    let mut descriptors = Vec::with_capacity(raw_tools.len());
    for raw_tool in raw_tools {
        let descriptor = descriptor_from_raw_tool(config, raw_tool);
        if !seen_public_names.insert(descriptor.public_name.clone()) {
            return Err(McpHostError::DuplicateToolName(descriptor.public_name));
        }
        descriptors.push(descriptor);
    }
    Ok(descriptors)
}

fn permission_labels_from_annotations(annotations: &Value) -> Vec<String> {
    let mut labels = Vec::new();

    if annotations
        .get("permissionLabels")
        .and_then(Value::as_array)
        .is_some()
    {
        for label in annotations
            .get("permissionLabels")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            push_label(&mut labels, label);
        }
    }

    if annotations
        .get("readOnlyHint")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        push_label(&mut labels, "read");
    }
    if annotations
        .get("destructiveHint")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        push_label(&mut labels, "destructive");
    }
    if annotations
        .get("openWorldHint")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        push_label(&mut labels, "network");
    }

    if labels.is_empty() {
        labels.push("unknown".to_string());
    }

    labels
}

fn push_label(labels: &mut Vec<String>, label: &str) {
    let normalized = label.trim().to_ascii_lowercase();
    if !normalized.is_empty() && !labels.iter().any(|existing| existing == &normalized) {
        labels.push(normalized);
    }
}

fn call_result_from_response(
    server_id: Uuid,
    public_name: String,
    tool_name: String,
    response: McpToolCallResponse,
) -> McpCallResult {
    let output = mcp_content_to_text(&response.content, response.structured_content.as_ref());
    McpCallResult {
        server_id,
        public_name,
        tool_name,
        output,
        content: response.content,
        structured_content: response.structured_content,
        is_error: response.is_error,
        raw: response.raw,
    }
}

fn mcp_content_to_text(content: &[Value], structured_content: Option<&Value>) -> String {
    let mut lines = Vec::new();

    for item in content {
        if let Some(text) = item.get("text").and_then(Value::as_str) {
            lines.push(text.to_string());
            continue;
        }

        if let Some(kind) = item.get("type").and_then(Value::as_str) {
            match kind {
                "image" => {
                    let mime_type = item
                        .get("mimeType")
                        .and_then(Value::as_str)
                        .unwrap_or("image");
                    lines.push(format!("[image: {mime_type}]"));
                }
                "resource" => lines.push("[resource]".to_string()),
                _ => lines.push(item.to_string()),
            }
        } else {
            lines.push(item.to_string());
        }
    }

    if lines.is_empty() {
        if let Some(structured_content) = structured_content {
            return serde_json::to_string_pretty(structured_content)
                .unwrap_or_else(|_| structured_content.to_string());
        }
    }

    lines.join("\n")
}

fn configure_child_environment(command: &mut Command, config: &McpServerConfig) {
    command.env_clear();
    command.envs(child_environment(config, |key| std::env::var_os(key)));
}

fn child_environment<F>(config: &McpServerConfig, inherited: F) -> Vec<(OsString, OsString)>
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
fn environment_key_identity(key: &OsStr) -> String {
    key.to_string_lossy().to_ascii_uppercase()
}

#[cfg(not(windows))]
fn environment_key_identity(key: &OsStr) -> String {
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

fn empty_object() -> Value {
    json!({})
}

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::mcp_public_tool_name;
    use tokio::io::{duplex, AsyncWriteExt};

    #[test]
    fn empty_env_keys_do_not_inherit_application_secrets() {
        let config = McpServerConfig::new("Environment Test".to_string(), "mock".to_string());
        let inherited = HashMap::from([
            ("PATH", OsString::from("safe-path")),
            ("SystemRoot", OsString::from("C:\\Windows")),
            ("OPENTOPIA_API_KEY", OsString::from("must-not-leak")),
            ("OPENAI_API_KEY", OsString::from("must-not-leak-either")),
            ("USERPROFILE", OsString::from("C:\\Users\\private")),
        ]);

        let child = child_environment(&config, |key| {
            inherited.get(key.to_string_lossy().as_ref()).cloned()
        });
        let child = child
            .into_iter()
            .map(|(key, value)| (environment_key_identity(&key), value))
            .collect::<HashMap<_, _>>();

        assert_eq!(
            child.get(&environment_key_identity(OsStr::new("PATH"))),
            Some(&OsString::from("safe-path"))
        );
        assert!(!child.contains_key(&environment_key_identity(OsStr::new("OPENTOPIA_API_KEY"))));
        assert!(!child.contains_key(&environment_key_identity(OsStr::new("OPENAI_API_KEY"))));
        assert!(!child.contains_key(&environment_key_identity(OsStr::new("USERPROFILE"))));
    }

    #[test]
    fn explicit_env_key_is_the_only_way_to_forward_a_secret() {
        let mut config = McpServerConfig::new("Environment Test".to_string(), "mock".to_string());
        config.env_keys = vec!["MCP_EXPLICIT_TOKEN".to_string()];
        let inherited = HashMap::from([
            ("PATH", OsString::from("safe-path")),
            ("MCP_EXPLICIT_TOKEN", OsString::from("forwarded")),
            ("OPENTOPIA_API_KEY", OsString::from("not-forwarded")),
        ]);

        let child = child_environment(&config, |key| {
            inherited.get(key.to_string_lossy().as_ref()).cloned()
        })
        .into_iter()
        .map(|(key, value)| (environment_key_identity(&key), value))
        .collect::<HashMap<_, _>>();

        assert_eq!(
            child.get(&environment_key_identity(OsStr::new("MCP_EXPLICIT_TOKEN"))),
            Some(&OsString::from("forwarded"))
        );
        assert!(!child.contains_key(&environment_key_identity(OsStr::new("OPENTOPIA_API_KEY"))));
    }

    #[test]
    fn parses_json_rpc_response_and_notification() {
        let response = parse_json_rpc_line(r#"{"jsonrpc":"2.0","id":7,"result":{"ok":true}}"#)
            .expect("response should parse");
        assert_eq!(
            response,
            McpIncomingMessage::Response {
                id: 7,
                result: Ok(json!({ "ok": true }))
            }
        );

        let notification =
            parse_json_rpc_line(r#"{"jsonrpc":"2.0","method":"notifications/tools/list_changed"}"#)
                .expect("notification should parse");
        assert_eq!(
            notification,
            McpIncomingMessage::Notification {
                method: "notifications/tools/list_changed".to_string(),
                params: None
            }
        );
    }

    #[test]
    fn public_tool_name_is_stable_and_safe() {
        assert_eq!(
            mcp_public_tool_name("File System", "Read-File!"),
            "file_system__read_file"
        );
    }

    #[tokio::test]
    async fn stdio_client_initializes_lists_and_calls_tools_over_mock_io() {
        let (client_stdin, server_stdin) = duplex(16 * 1024);
        let (server_stdout, client_stdout) = duplex(16 * 1024);
        let server = tokio::spawn(run_mock_mcp_server(server_stdin, server_stdout));

        let mut config = McpServerConfig::new("Mock Server".to_string(), "mock".to_string());
        config.timeout_ms = 5_000;
        let client = McpStdioClient::from_io_for_test(config, client_stdout, client_stdin);

        let initialize = client
            .initialize()
            .await
            .expect("initialize should succeed");
        assert_eq!(
            initialize.get("protocolVersion").and_then(Value::as_str),
            Some(MCP_PROTOCOL_VERSION)
        );

        let tools = client
            .list_tools()
            .await
            .expect("tools/list should succeed");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo");

        let call = client
            .call_tool("echo", json!({ "text": "hello" }))
            .await
            .expect("tools/call should succeed");
        assert_eq!(mcp_content_to_text(&call.content, None), "echo: hello");

        client.shutdown().await.expect("shutdown should succeed");
        server.await.expect("mock server task should finish");
    }

    #[tokio::test]
    async fn stdio_client_rejects_server_capability_request_without_losing_tool_response() {
        let (client_stdin, server_stdin) = duplex(16 * 1024);
        let (server_stdout, client_stdout) = duplex(16 * 1024);
        let server = tokio::spawn(run_mock_server_request_during_tool_call(
            server_stdin,
            server_stdout,
            "sampling/createMessage",
            Some("sampling"),
        ));

        let mut config = McpServerConfig::new("Mock Server".to_string(), "mock".to_string());
        config.timeout_ms = 5_000;
        let client = McpStdioClient::from_io_for_test(config, client_stdout, client_stdin);

        let call = client
            .call_tool("echo", json!({ "text": "after request" }))
            .await
            .expect("tools/call response should remain correlated");
        assert_eq!(
            mcp_content_to_text(&call.content, None),
            "echo: after request"
        );

        client.shutdown().await.expect("shutdown should succeed");
        server.await.expect("mock server task should finish");
    }

    #[tokio::test]
    async fn unknown_server_request_does_not_deadlock_client() {
        let (client_stdin, server_stdin) = duplex(16 * 1024);
        let (server_stdout, client_stdout) = duplex(16 * 1024);
        let server = tokio::spawn(run_mock_server_request_during_tool_call(
            server_stdin,
            server_stdout,
            "unknown/clientRequest",
            None,
        ));

        let mut config = McpServerConfig::new("Mock Server".to_string(), "mock".to_string());
        config.timeout_ms = 5_000;
        let client = McpStdioClient::from_io_for_test(config, client_stdout, client_stdin);

        let call = timeout(
            Duration::from_secs(2),
            client.call_tool("echo", json!({ "text": "not blocked" })),
        )
        .await
        .expect("unknown server request must not deadlock the client")
        .expect("tools/call should succeed after unknown server request");
        assert_eq!(
            mcp_content_to_text(&call.content, None),
            "echo: not blocked"
        );

        client.shutdown().await.expect("shutdown should succeed");
        server.await.expect("mock server task should finish");
    }

    #[tokio::test]
    async fn extension_host_routes_public_tool_calls() {
        let (client_stdin, server_stdin) = duplex(16 * 1024);
        let (server_stdout, client_stdout) = duplex(16 * 1024);
        let server = tokio::spawn(run_mock_mcp_server(server_stdin, server_stdout));

        let mut config = McpServerConfig::new("Mock Server".to_string(), "mock".to_string());
        config.timeout_ms = 5_000;
        let client = Arc::new(McpStdioClient::from_io_for_test(
            config.clone(),
            client_stdout,
            client_stdin,
        ));
        client
            .initialize()
            .await
            .expect("initialize should succeed");

        let host = McpExtensionHost::new();
        let status = host
            .install_client_for_test(config.clone(), client.clone())
            .await
            .expect("client should install");
        assert!(matches!(status.status, McpLifecycleStatus::Ready));

        let public_name = mcp_public_tool_name(&config.name, "echo");
        let tools = host
            .list_tools(config.server_id)
            .await
            .expect("tools should be cached");
        assert_eq!(tools[0].public_name, public_name);
        assert_eq!(tools[0].permission_labels, vec!["read".to_string()]);

        let result = host
            .call_tool(&public_name, json!({ "text": "routed" }))
            .await
            .expect("routed call should succeed");
        assert_eq!(result.output, "echo: routed");
        assert_eq!(result.tool_name, "echo");

        host.stop_server(config.server_id)
            .await
            .expect("stop should succeed");
        server.await.expect("mock server task should finish");
    }

    #[tokio::test]
    async fn tools_list_changed_refreshes_cached_descriptors_and_routes() {
        let (client_stdin, server_stdin) = duplex(16 * 1024);
        let (server_stdout, client_stdout) = duplex(16 * 1024);
        let (change_sender, change_receiver) = oneshot::channel();
        let server = tokio::spawn(run_changing_tool_server(
            server_stdin,
            server_stdout,
            change_receiver,
        ));

        let mut config = McpServerConfig::new("Changing Server".to_string(), "mock".to_string());
        config.timeout_ms = 5_000;
        let client = Arc::new(McpStdioClient::from_io_for_test(
            config.clone(),
            client_stdout,
            client_stdin,
        ));
        client
            .initialize()
            .await
            .expect("initialize should succeed");

        let host = McpExtensionHost::new();
        host.install_client_for_test(config.clone(), client.clone())
            .await
            .expect("initial tool list should install");
        assert_eq!(
            host.list_tools(config.server_id).await.unwrap()[0].tool_name,
            "before"
        );

        change_sender.send(()).expect("change signal should send");
        timeout(Duration::from_secs(2), async {
            while client.tools_generation() == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("list_changed notification should arrive");

        let refreshed = host
            .list_tools(config.server_id)
            .await
            .expect("invalidated cache should refresh");
        assert_eq!(refreshed.len(), 1);
        assert_eq!(refreshed[0].tool_name, "after");
        assert!(matches!(
            host.call_tool(&mcp_public_tool_name(&config.name, "before"), json!({}))
                .await,
            Err(McpHostError::ToolNotFound(_))
        ));

        host.stop_server(config.server_id)
            .await
            .expect("stop should succeed");
        server.await.expect("mock server task should finish");
    }

    async fn run_mock_mcp_server(
        stdin: tokio::io::DuplexStream,
        mut stdout: tokio::io::DuplexStream,
    ) {
        let mut lines = BufReader::new(stdin).lines();
        while let Some(line) = lines.next_line().await.expect("mock read should succeed") {
            let value: Value = serde_json::from_str(&line).expect("client message should be JSON");
            let method = value
                .get("method")
                .and_then(Value::as_str)
                .expect("client message should include method");

            match method {
                "initialize" => {
                    write_json_line(
                        &mut stdout,
                        json!({
                            "jsonrpc": "2.0",
                            "id": value.get("id").cloned().unwrap_or(Value::Null),
                            "result": {
                                "protocolVersion": MCP_PROTOCOL_VERSION,
                                "capabilities": {
                                    "tools": {}
                                },
                                "serverInfo": {
                                    "name": "mock",
                                    "version": "0.0.0"
                                }
                            }
                        }),
                    )
                    .await;
                }
                "notifications/initialized" => {}
                "tools/list" => {
                    write_json_line(
                        &mut stdout,
                        json!({
                            "jsonrpc": "2.0",
                            "id": value.get("id").cloned().unwrap_or(Value::Null),
                            "result": {
                                "tools": [{
                                    "name": "echo",
                                    "description": "Echo text",
                                    "inputSchema": {
                                        "type": "object",
                                        "properties": {
                                            "text": { "type": "string" }
                                        }
                                    },
                                    "annotations": {
                                        "readOnlyHint": true
                                    }
                                }]
                            }
                        }),
                    )
                    .await;
                }
                "tools/call" => {
                    let text = value
                        .pointer("/params/arguments/text")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    write_json_line(
                        &mut stdout,
                        json!({
                            "jsonrpc": "2.0",
                            "id": value.get("id").cloned().unwrap_or(Value::Null),
                            "result": {
                                "content": [{
                                    "type": "text",
                                    "text": format!("echo: {text}")
                                }],
                                "isError": false
                            }
                        }),
                    )
                    .await;
                }
                other => {
                    write_json_line(
                        &mut stdout,
                        json!({
                            "jsonrpc": "2.0",
                            "id": value.get("id").cloned().unwrap_or(Value::Null),
                            "error": {
                                "code": -32601,
                                "message": format!("unknown method: {other}")
                            }
                        }),
                    )
                    .await;
                }
            }
        }
    }

    async fn run_changing_tool_server(
        stdin: tokio::io::DuplexStream,
        mut stdout: tokio::io::DuplexStream,
        mut change: oneshot::Receiver<()>,
    ) {
        let mut lines = BufReader::new(stdin).lines();
        let mut changed = false;
        let mut notification_sent = false;

        loop {
            tokio::select! {
                signal = &mut change, if !notification_sent => {
                    if signal.is_ok() {
                        changed = true;
                        write_json_line(
                            &mut stdout,
                            json!({
                                "jsonrpc": "2.0",
                                "method": "notifications/tools/list_changed"
                            }),
                        )
                        .await;
                    }
                    notification_sent = true;
                }
                line = lines.next_line() => {
                    let Some(line) = line.expect("mock read should succeed") else {
                        break;
                    };
                    let value: Value = serde_json::from_str(&line)
                        .expect("client message should be JSON");
                    match value.get("method").and_then(Value::as_str).unwrap_or("") {
                        "initialize" => {
                            write_json_line(
                                &mut stdout,
                                json!({
                                    "jsonrpc": "2.0",
                                    "id": value.get("id").cloned().unwrap_or(Value::Null),
                                    "result": {
                                        "protocolVersion": MCP_PROTOCOL_VERSION,
                                        "capabilities": { "tools": { "listChanged": true } },
                                        "serverInfo": { "name": "changing", "version": "0.0.0" }
                                    }
                                }),
                            )
                            .await;
                        }
                        "notifications/initialized" => {}
                        "tools/list" => {
                            let tool_name = if changed { "after" } else { "before" };
                            write_json_line(
                                &mut stdout,
                                json!({
                                    "jsonrpc": "2.0",
                                    "id": value.get("id").cloned().unwrap_or(Value::Null),
                                    "result": {
                                        "tools": [{
                                            "name": tool_name,
                                            "description": "Changes after notification",
                                            "inputSchema": { "type": "object" }
                                        }]
                                    }
                                }),
                            )
                            .await;
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    async fn run_mock_server_request_during_tool_call(
        stdin: tokio::io::DuplexStream,
        mut stdout: tokio::io::DuplexStream,
        server_method: &'static str,
        expected_capability: Option<&'static str>,
    ) {
        let mut lines = BufReader::new(stdin).lines();
        let tool_request = read_mock_json_line(&mut lines).await;
        assert_eq!(
            tool_request.get("method").and_then(Value::as_str),
            Some("tools/call")
        );
        let tool_request_id = tool_request
            .get("id")
            .cloned()
            .expect("tool request should include id");

        write_json_line(
            &mut stdout,
            json!({
                "jsonrpc": "2.0",
                "id": "server-ping",
                "method": "ping"
            }),
        )
        .await;
        let ping_response = read_mock_json_line(&mut lines).await;
        assert_eq!(ping_response.get("id"), Some(&json!("server-ping")));
        assert_eq!(ping_response.get("result"), Some(&empty_object()));
        assert!(ping_response.get("error").is_none());

        write_json_line(
            &mut stdout,
            json!({
                "jsonrpc": "2.0",
                "id": tool_request_id.clone(),
                "method": server_method,
                "params": {}
            }),
        )
        .await;
        let error_response = read_mock_json_line(&mut lines).await;
        assert_eq!(error_response.get("id"), Some(&tool_request_id));
        assert_eq!(
            error_response
                .pointer("/error/code")
                .and_then(Value::as_i64),
            Some(JSON_RPC_METHOD_NOT_FOUND)
        );
        assert_eq!(
            error_response
                .pointer("/error/message")
                .and_then(Value::as_str),
            Some("Method not found")
        );
        assert_eq!(
            error_response
                .pointer("/error/data/capability")
                .and_then(Value::as_str),
            expected_capability
        );

        let text = tool_request
            .pointer("/params/arguments/text")
            .and_then(Value::as_str)
            .unwrap_or("");
        write_json_line(
            &mut stdout,
            json!({
                "jsonrpc": "2.0",
                "id": tool_request_id,
                "result": {
                    "content": [{
                        "type": "text",
                        "text": format!("echo: {text}")
                    }],
                    "isError": false
                }
            }),
        )
        .await;
    }

    async fn read_mock_json_line(
        lines: &mut tokio::io::Lines<BufReader<tokio::io::DuplexStream>>,
    ) -> Value {
        let line = timeout(Duration::from_secs(2), lines.next_line())
            .await
            .expect("mock read should not time out")
            .expect("mock read should succeed")
            .expect("mock stream should remain open");
        serde_json::from_str(&line).expect("client message should be JSON")
    }

    async fn write_json_line(stdout: &mut tokio::io::DuplexStream, value: Value) {
        let mut bytes = serde_json::to_vec(&value).expect("mock response should serialize");
        bytes.push(b'\n');
        stdout
            .write_all(&bytes)
            .await
            .expect("mock write should succeed");
        stdout.flush().await.expect("mock flush should succeed");
    }
}
