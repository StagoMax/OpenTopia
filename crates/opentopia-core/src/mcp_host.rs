use crate::execution::ExecutionEnvironment;
use crate::mcp::{
    mcp_public_tool_name, McpCallResult, McpLifecycleStatus, McpServerConfig, McpServerStatus,
    McpToolDescriptor,
};
use crate::mcp_operation_fingerprint;
#[cfg(test)]
use async_trait::async_trait;
#[cfg(test)]
use serde_json::json;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
#[cfg(test)]
use std::ffi::OsStr;
#[cfg(test)]
use std::sync::atomic::Ordering;
use std::sync::Arc;
#[cfg(test)]
use std::time::Duration;
use thiserror::Error;
#[cfg(test)]
use tokio::sync::oneshot;
use tokio::sync::{Mutex, RwLock};
#[cfg(test)]
use tokio::task::JoinHandle;
#[cfg(test)]
use tokio::time::timeout;
use tracing::warn;
use uuid::Uuid;

/// Durable mirror of each MCP server's last successful `tools/list`.
///
/// The host keeps the authoritative catalog in memory. This trait only lets that catalog
/// survive a server restart so the workbench can describe a server's tools before its stdio
/// process has finished initializing, or when it fails to start at all. Persisted entries are
/// never used to route a `call_tool`: routing still requires a live, ready client.
pub trait McpToolCatalogStore: Send + Sync {
    fn replace_tools(&self, server_id: Uuid, tools: &[McpToolDescriptor]) -> anyhow::Result<()>;
    fn load_all_tools(&self) -> anyhow::Result<Vec<McpToolDescriptor>>;
}

impl McpToolCatalogStore for crate::store::SqliteSessionStore {
    fn replace_tools(&self, server_id: Uuid, tools: &[McpToolDescriptor]) -> anyhow::Result<()> {
        self.replace_mcp_server_tools(server_id, tools)
    }

    fn load_all_tools(&self) -> anyhow::Result<Vec<McpToolDescriptor>> {
        self.list_all_mcp_server_tools()
    }
}

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
    #[error(
        "MCP tool {tool_name} on server {server_id} changed after its Connection grant was reviewed"
    )]
    ToolFingerprintChanged { server_id: Uuid, tool_name: String },
    #[error("duplicate public MCP tool name: {0}")]
    DuplicateToolName(String),
    #[error("ambiguous public MCP tool name: {0}")]
    AmbiguousToolName(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Clone)]
pub struct McpExtensionHost {
    inner: Arc<RwLock<McpExtensionHostInner>>,
    spawner: Arc<dyn McpProcessSpawner>,
    lifecycle_locks: Arc<Mutex<HashMap<Uuid, Arc<Mutex<()>>>>>,
    catalog: Option<Arc<dyn McpToolCatalogStore>>,
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
            lifecycle_locks: Arc::new(Mutex::new(HashMap::new())),
            catalog: None,
        }
    }

    /// Attaches the durable tool catalog used to warm the cache after a restart.
    pub fn with_tool_catalog_store(mut self, catalog: Arc<dyn McpToolCatalogStore>) -> Self {
        self.catalog = Some(catalog);
        self
    }

    /// Loads the persisted tool catalog into memory.
    ///
    /// Call once during startup, before servers are started. Persisted descriptors only answer
    /// `cached_tools` for servers that have no live runtime yet; they are never added to the
    /// routing table.
    pub async fn warm_tool_cache(&self) -> anyhow::Result<usize> {
        let Some(catalog) = self.catalog.clone() else {
            return Ok(0);
        };
        let tools = catalog.load_all_tools()?;
        let mut persisted: HashMap<Uuid, Vec<McpToolDescriptor>> = HashMap::new();
        for tool in tools {
            persisted.entry(tool.server_id).or_default().push(tool);
        }
        let count = persisted.values().map(Vec::len).sum();
        let mut inner = self.inner.write().await;
        inner.persisted_tools = persisted;
        Ok(count)
    }

    fn persist_tools(&self, server_id: Uuid, tools: &[McpToolDescriptor]) {
        let Some(catalog) = self.catalog.as_ref() else {
            return;
        };
        if let Err(err) = catalog.replace_tools(server_id, tools) {
            // A stale mirror must not fail an otherwise healthy server start.
            warn!(%server_id, error = %err, "failed to persist MCP tool schema cache");
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
        let lifecycle_lock = self.lifecycle_lock(config.server_id).await;
        let _guard = lifecycle_lock.lock().await;
        self.restart_server_locked(config).await
    }

    pub async fn ensure_server(
        &self,
        config: McpServerConfig,
    ) -> Result<McpServerStatus, McpHostError> {
        let lifecycle_lock = self.lifecycle_lock(config.server_id).await;
        let _guard = lifecycle_lock.lock().await;
        let current = {
            let inner = self.inner.read().await;
            inner.servers.get(&config.server_id).and_then(|runtime| {
                (runtime.config.updated_at == config.updated_at)
                    .then(|| inner.statuses.get(&config.server_id).cloned())
                    .flatten()
            })
        };
        if let Some(status) = current.filter(|status| {
            matches!(
                status.status,
                McpLifecycleStatus::Ready | McpLifecycleStatus::Disabled
            )
        }) {
            return Ok(status);
        }

        self.restart_server_locked(config).await
    }

    async fn restart_server_locked(
        &self,
        config: McpServerConfig,
    ) -> Result<McpServerStatus, McpHostError> {
        self.stop_server_locked(config.server_id).await?;

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
        let lifecycle_lock = self.lifecycle_lock(server_id).await;
        let _guard = lifecycle_lock.lock().await;
        self.stop_server_locked(server_id).await
    }

    /// Stops a server and drops its cached catalog.
    ///
    /// Use when the server configuration itself is deleted. A plain `stop_server` deliberately
    /// keeps the last known catalog so a stopped server can still describe its tools. The
    /// persisted rows are removed by the `mcp_servers` foreign key cascade.
    pub async fn forget_server(&self, server_id: Uuid) -> Result<(), McpHostError> {
        let lifecycle_lock = self.lifecycle_lock(server_id).await;
        let _guard = lifecycle_lock.lock().await;
        self.stop_server_locked(server_id).await?;
        let mut inner = self.inner.write().await;
        inner.persisted_tools.remove(&server_id);
        Ok(())
    }

    async fn stop_server_locked(&self, server_id: Uuid) -> Result<(), McpHostError> {
        let runtime = {
            let mut inner = self.inner.write().await;
            remove_server_routes(&mut inner.tool_routes, server_id);
            inner.servers.remove(&server_id)
        };

        if let Some(runtime) = runtime {
            runtime.client.shutdown().await?;
        }

        Ok(())
    }

    async fn lifecycle_lock(&self, server_id: Uuid) -> Arc<Mutex<()>> {
        self.lifecycle_locks
            .lock()
            .await
            .entry(server_id)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
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
            .or_else(|| inner.persisted_tools.get(&server_id).cloned())
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
        // Resolve the already-disclosed route first. A tool call must never scan
        // or refresh unrelated account runtimes on its latency-sensitive path.
        let server_id = {
            let inner = self.inner.read().await;
            unique_public_route(&inner.tool_routes, public_name)?.server_id
        };
        self.refresh_tools_if_invalidated(server_id).await?;
        let (route, client) = {
            let inner = self.inner.read().await;
            let route = unique_public_route(&inner.tool_routes, public_name)?.clone();
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

    /// Calls one provider-native tool on one exact server after confirming the
    /// live descriptor still matches the immutable operation reviewed by the
    /// Agent template. This is the structured Connection execution boundary;
    /// unlike the legacy public-name route, it cannot be redirected by a name
    /// collision or an editable server display name.
    pub async fn call_server_tool(
        &self,
        server_id: Uuid,
        provider_tool_name: &str,
        expected_operation_fingerprint: &str,
        arguments: Value,
    ) -> Result<McpCallResult, McpHostError> {
        self.refresh_tools_if_invalidated(server_id).await?;
        let (descriptor, client) = {
            let inner = self.inner.read().await;
            let runtime = inner
                .servers
                .get(&server_id)
                .ok_or(McpHostError::ServerNotFound(server_id))?;
            let descriptor = runtime
                .tools
                .iter()
                .find(|descriptor| descriptor.tool_name == provider_tool_name)
                .cloned()
                .ok_or_else(|| {
                    McpHostError::ToolNotFound(format!("{server_id}:{provider_tool_name}"))
                })?;
            (descriptor, runtime.client.clone())
        };
        if mcp_operation_fingerprint(&descriptor) != expected_operation_fingerprint {
            return Err(McpHostError::ToolFingerprintChanged {
                server_id,
                tool_name: provider_tool_name.to_string(),
            });
        }

        let response = client.call_tool(provider_tool_name, arguments).await?;
        Ok(call_result_from_response(
            server_id,
            descriptor.public_name,
            descriptor.tool_name,
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

        let invalidated_generation = client.tools_generation();
        if invalidated_generation <= cached_generation {
            return Ok(());
        }

        let raw_tools = client.list_tools().await?;
        let descriptors = descriptors_from_raw_tools(&config, raw_tools)?;
        // Do not absorb a second notification that races with tools/list. A
        // later call must refresh again instead of treating a potentially
        // stale response as the newest generation.
        let observed_generation = invalidated_generation;

        let mut inner = self.inner.write().await;
        let Some(runtime) = inner.servers.get(&server_id) else {
            return Err(McpHostError::ServerNotFound(server_id));
        };
        if !Arc::ptr_eq(&runtime.client, &client) || runtime.tools_generation >= observed_generation
        {
            return Ok(());
        }

        remove_server_routes(&mut inner.tool_routes, server_id);
        for descriptor in &descriptors {
            inner
                .tool_routes
                .entry(descriptor.public_name.clone())
                .or_default()
                .push(McpToolRoute {
                    server_id,
                    public_name: descriptor.public_name.clone(),
                    tool_name: descriptor.tool_name.clone(),
                });
        }

        if let Some(runtime) = inner.servers.get_mut(&server_id) {
            runtime.tools = descriptors.clone();
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
        inner.persisted_tools.insert(server_id, descriptors.clone());
        drop(inner);

        self.persist_tools(server_id, &descriptors);

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
        let server_id = config.server_id;

        let status = McpServerStatus {
            server_id: config.server_id,
            name: config.name.clone(),
            status: McpLifecycleStatus::Ready,
            message: "MCP stdio server initialized.".to_string(),
            tools_count: descriptors.len(),
            updated_at: chrono::Utc::now(),
        };

        let mut inner = self.inner.write().await;
        remove_server_routes(&mut inner.tool_routes, config.server_id);

        for descriptor in &descriptors {
            inner
                .tool_routes
                .entry(descriptor.public_name.clone())
                .or_default()
                .push(McpToolRoute {
                    server_id: config.server_id,
                    public_name: descriptor.public_name.clone(),
                    tool_name: descriptor.tool_name.clone(),
                });
        }

        inner.servers.insert(
            config.server_id,
            McpServerRuntime {
                config,
                tools_generation: client.tools_generation(),
                client,
                tools: descriptors.clone(),
            },
        );
        inner.statuses.insert(status.server_id, status.clone());
        inner.persisted_tools.insert(server_id, descriptors.clone());
        drop(inner);

        self.persist_tools(server_id, &descriptors);

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
    tool_routes: HashMap<String, Vec<McpToolRoute>>,
    /// Last known catalog per server, restored from the durable mirror at startup. Only read
    /// for servers with no live runtime; a live runtime is always authoritative.
    persisted_tools: HashMap<Uuid, Vec<McpToolDescriptor>>,
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

fn remove_server_routes(routes: &mut HashMap<String, Vec<McpToolRoute>>, server_id: Uuid) {
    for candidates in routes.values_mut() {
        candidates.retain(|route| route.server_id != server_id);
    }
    routes.retain(|_, candidates| !candidates.is_empty());
}

fn unique_public_route<'a>(
    routes: &'a HashMap<String, Vec<McpToolRoute>>,
    public_name: &str,
) -> Result<&'a McpToolRoute, McpHostError> {
    let candidates = routes
        .get(public_name)
        .ok_or_else(|| McpHostError::ToolNotFound(public_name.to_string()))?;
    match candidates.as_slice() {
        [route] => Ok(route),
        _ => Err(McpHostError::AmbiguousToolName(public_name.to_string())),
    }
}

mod stdio_transport;

pub use stdio_transport::{
    parse_json_rpc_line, ExecutionEnvironmentMcpProcessSpawner, JsonRpcWireError, McpChildProcess,
    McpIncomingMessage, McpProcessSpawner, McpRawTool, McpSpawnedProcess, McpStdioClient,
    McpToolCallResponse, SecureLocalMcpProcessSpawner,
};

#[cfg(test)]
use stdio_transport::{
    child_environment, empty_object, environment_key_identity, JSON_RPC_METHOD_NOT_FOUND,
    MCP_PROTOCOL_VERSION,
};

fn descriptor_from_raw_tool(config: &McpServerConfig, raw_tool: McpRawTool) -> McpToolDescriptor {
    McpToolDescriptor {
        public_name: mcp_public_tool_name(&config.name, &raw_tool.name),
        server_id: config.server_id,
        tool_name: raw_tool.name,
        description: raw_tool.description,
        input_schema: raw_tool.input_schema,
        annotations: raw_tool.annotations.clone(),
        meta: raw_tool.meta,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::mcp_public_tool_name;
    use std::ffi::OsString;
    use std::sync::atomic::AtomicUsize;
    use tokio::io::{duplex, AsyncBufReadExt, AsyncWriteExt, BufReader};

    struct MockMcpChildProcess {
        task: Option<JoinHandle<()>>,
    }

    #[async_trait]
    impl McpChildProcess for MockMcpChildProcess {
        async fn kill(&mut self) -> Result<(), McpHostError> {
            if let Some(task) = self.task.as_ref() {
                task.abort();
            }
            Ok(())
        }

        async fn wait(&mut self) -> Result<(), McpHostError> {
            if let Some(task) = self.task.take() {
                let _ = task.await;
            }
            Ok(())
        }

        fn start_kill(&mut self) {
            if let Some(task) = self.task.as_ref() {
                task.abort();
            }
        }
    }

    struct CountingMcpSpawner {
        starts: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl McpProcessSpawner for CountingMcpSpawner {
        async fn spawn(
            &self,
            _config: &McpServerConfig,
        ) -> Result<McpSpawnedProcess, McpHostError> {
            self.starts.fetch_add(1, Ordering::SeqCst);
            let (client_stdin, server_stdin) = duplex(16 * 1024);
            let (server_stdout, client_stdout) = duplex(16 * 1024);
            let task = tokio::spawn(run_mock_mcp_server(server_stdin, server_stdout));
            Ok(McpSpawnedProcess::new(
                client_stdin,
                client_stdout,
                None::<tokio::io::DuplexStream>,
                MockMcpChildProcess { task: Some(task) },
            ))
        }
    }

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
    async fn ensure_server_serializes_concurrent_starts_and_reuses_ready_runtime() {
        let starts = Arc::new(AtomicUsize::new(0));
        let host = McpExtensionHost::with_spawner(Arc::new(CountingMcpSpawner {
            starts: starts.clone(),
        }));
        let mut config = McpServerConfig::new("Mock Server".to_string(), "mock".to_string());
        config.timeout_ms = 5_000;

        let (left, right) = tokio::join!(
            host.ensure_server(config.clone()),
            host.ensure_server(config.clone())
        );

        assert!(matches!(left.unwrap().status, McpLifecycleStatus::Ready));
        assert!(matches!(right.unwrap().status, McpLifecycleStatus::Ready));
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        assert_eq!(host.list_tools(config.server_id).await.unwrap().len(), 1);

        host.stop_server(config.server_id)
            .await
            .expect("mock server should stop");
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
        assert_eq!(
            tools[0].meta["com.opentopia/capabilities"][0],
            "fixture.echo/v1"
        );

        let call = client
            .call_tool("echo", json!({ "text": "hello" }))
            .await
            .expect("tools/call should succeed");
        assert_eq!(mcp_content_to_text(&call.content, None), "echo: hello");

        client.shutdown().await.expect("shutdown should succeed");
        server.await.expect("mock server task should finish");
    }

    #[tokio::test]
    async fn rpc_timeout_does_not_terminate_persistent_stdio_transport() {
        let (client_stdin, server_stdin) = duplex(16 * 1024);
        let (server_stdout, client_stdout) = duplex(16 * 1024);
        let server = tokio::spawn(run_slow_first_call_mcp_server(server_stdin, server_stdout));

        let mut config = McpServerConfig::new("Slow Server".to_string(), "mock".to_string());
        config.timeout_ms = 100;
        let client = McpStdioClient::from_io_for_test(config, client_stdout, client_stdin);

        let first = client.call_tool("echo", json!({ "text": "slow" })).await;
        assert!(matches!(first, Err(McpHostError::Timeout { .. })));

        let second = client
            .call_tool("echo", json!({ "text": "still alive" }))
            .await
            .expect("transport must remain usable after one RPC timeout");
        assert_eq!(
            mcp_content_to_text(&second.content, None),
            "echo: still alive"
        );

        client.shutdown().await.expect("shutdown should succeed");
        server.await.expect("mock server task should finish");
    }

    #[tokio::test]
    async fn stdio_client_correlates_concurrent_tool_calls_that_finish_out_of_order() {
        let (client_stdin, server_stdin) = duplex(16 * 1024);
        let (server_stdout, client_stdout) = duplex(16 * 1024);
        let server = tokio::spawn(run_out_of_order_mcp_server(server_stdin, server_stdout));

        let mut config = McpServerConfig::new("Concurrent Server".to_string(), "mock".to_string());
        config.timeout_ms = 5_000;
        let client = McpStdioClient::from_io_for_test(config, client_stdout, client_stdin);

        let (first, second) = tokio::join!(
            client.call_tool("echo", json!({ "text": "first" })),
            client.call_tool("echo", json!({ "text": "second" })),
        );
        assert_eq!(
            mcp_content_to_text(&first.expect("first call should succeed").content, None),
            "echo: first"
        );
        assert_eq!(
            mcp_content_to_text(&second.expect("second call should succeed").content, None),
            "echo: second"
        );

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
    async fn exact_route_disambiguates_same_public_name_across_account_servers() {
        let host = McpExtensionHost::new();
        let mut runtimes = Vec::new();

        for _ in 0..2 {
            let (client_stdin, server_stdin) = duplex(16 * 1024);
            let (server_stdout, client_stdout) = duplex(16 * 1024);
            let server = tokio::spawn(run_mock_mcp_server(server_stdin, server_stdout));
            let mut config = McpServerConfig::new("Shared CRM".to_string(), "mock".to_string());
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
            host.install_client_for_test(config.clone(), client)
                .await
                .expect("same public name on a distinct account server should install");
            runtimes.push((config, server));
        }

        let public_name = mcp_public_tool_name("Shared CRM", "echo");
        assert!(matches!(
            host.call_tool(&public_name, json!({ "text": "legacy" }))
                .await,
            Err(McpHostError::AmbiguousToolName(name)) if name == public_name
        ));

        for (index, (config, _)) in runtimes.iter().enumerate() {
            let descriptor = host
                .list_tools(config.server_id)
                .await
                .expect("exact server catalog")
                .into_iter()
                .next()
                .expect("echo descriptor");
            let result = host
                .call_server_tool(
                    config.server_id,
                    "echo",
                    &mcp_operation_fingerprint(&descriptor),
                    json!({ "text": format!("account-{index}") }),
                )
                .await
                .expect("exact account route should succeed");
            assert_eq!(result.output, format!("echo: account-{index}"));
            assert_eq!(result.server_id, config.server_id);
        }

        for (config, server) in runtimes {
            host.stop_server(config.server_id)
                .await
                .expect("stop should succeed");
            server.await.expect("mock server task should finish");
        }
    }

    #[tokio::test]
    async fn exact_route_refreshes_only_target_and_changed_tool_makes_zero_wire_calls() {
        let host = McpExtensionHost::new();
        let mut runtimes = Vec::new();

        for name in ["Target", "Unrelated"] {
            let (client_stdin, server_stdin) = duplex(16 * 1024);
            let (server_stdout, client_stdout) = duplex(16 * 1024);
            let (change_sender, change_receiver) = oneshot::channel();
            let list_calls = Arc::new(AtomicUsize::new(0));
            let tool_calls = Arc::new(AtomicUsize::new(0));
            let server = tokio::spawn(run_guarded_route_server(
                server_stdin,
                server_stdout,
                change_receiver,
                GuardedRouteChange::Schema,
                list_calls.clone(),
                tool_calls.clone(),
            ));
            let mut config = McpServerConfig::new(name.to_string(), "mock".to_string());
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
            host.install_client_for_test(config.clone(), client.clone())
                .await
                .expect("initial tool list should install");
            let descriptor = host.list_tools(config.server_id).await.unwrap()[0].clone();
            runtimes.push((
                config,
                client,
                descriptor,
                change_sender,
                list_calls,
                tool_calls,
                server,
            ));
        }

        for (_, client, _, sender, _, _, _) in &mut runtimes {
            let sender = std::mem::replace(sender, oneshot::channel().0);
            sender.send(()).expect("change signal should send");
            timeout(Duration::from_secs(2), async {
                while client.tools_generation() == 0 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("list_changed notification should arrive");
        }

        let (target, _, pinned, _, target_lists, target_calls, _) = &runtimes[0];
        let unrelated_lists_before = runtimes[1].4.load(Ordering::SeqCst);
        let err = host
            .call_server_tool(
                target.server_id,
                "echo",
                &mcp_operation_fingerprint(pinned),
                json!({}),
            )
            .await
            .expect_err("changed schema must fail before the provider call");
        assert!(matches!(err, McpHostError::ToolFingerprintChanged { .. }));
        assert_eq!(target_calls.load(Ordering::SeqCst), 0);
        assert_eq!(target_lists.load(Ordering::SeqCst), 2);
        assert_eq!(
            runtimes[1].4.load(Ordering::SeqCst),
            unrelated_lists_before,
            "calling one Connection must not refresh an unrelated account runtime"
        );

        for (config, _, _, _, _, _, server) in runtimes {
            host.stop_server(config.server_id)
                .await
                .expect("stop should succeed");
            server.await.expect("mock server task should finish");
        }
    }

    #[tokio::test]
    async fn removed_exact_route_makes_zero_wire_calls() {
        let (client_stdin, server_stdin) = duplex(16 * 1024);
        let (server_stdout, client_stdout) = duplex(16 * 1024);
        let (change_sender, change_receiver) = oneshot::channel();
        let list_calls = Arc::new(AtomicUsize::new(0));
        let tool_calls = Arc::new(AtomicUsize::new(0));
        let server = tokio::spawn(run_guarded_route_server(
            server_stdin,
            server_stdout,
            change_receiver,
            GuardedRouteChange::Removed,
            list_calls,
            tool_calls.clone(),
        ));
        let mut config = McpServerConfig::new("Removed Tool".to_string(), "mock".to_string());
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
        let pinned = host.list_tools(config.server_id).await.unwrap()[0].clone();

        change_sender.send(()).expect("change signal should send");
        timeout(Duration::from_secs(2), async {
            while client.tools_generation() == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("list_changed notification should arrive");

        let err = host
            .call_server_tool(
                config.server_id,
                "echo",
                &mcp_operation_fingerprint(&pinned),
                json!({}),
            )
            .await
            .expect_err("removed tool must fail before the provider call");
        assert!(matches!(err, McpHostError::ToolNotFound(_)));
        assert_eq!(tool_calls.load(Ordering::SeqCst), 0);

        host.stop_server(config.server_id)
            .await
            .expect("stop should succeed");
        server.await.expect("mock server task should finish");
    }

    #[derive(Default)]
    struct RecordingCatalogStore {
        tools: std::sync::Mutex<HashMap<Uuid, Vec<McpToolDescriptor>>>,
    }

    impl McpToolCatalogStore for RecordingCatalogStore {
        fn replace_tools(
            &self,
            server_id: Uuid,
            tools: &[McpToolDescriptor],
        ) -> anyhow::Result<()> {
            self.tools
                .lock()
                .expect("catalog mutex poisoned")
                .insert(server_id, tools.to_vec());
            Ok(())
        }

        fn load_all_tools(&self) -> anyhow::Result<Vec<McpToolDescriptor>> {
            Ok(self
                .tools
                .lock()
                .expect("catalog mutex poisoned")
                .values()
                .flatten()
                .cloned()
                .collect())
        }
    }

    #[tokio::test]
    async fn ready_server_writes_tool_catalog_through_to_the_store() {
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

        let catalog = Arc::new(RecordingCatalogStore::default());
        let host = McpExtensionHost::new().with_tool_catalog_store(catalog.clone());
        host.install_client_for_test(config.clone(), client.clone())
            .await
            .expect("client should install");

        let persisted = catalog.tools.lock().expect("catalog mutex poisoned");
        let stored = persisted
            .get(&config.server_id)
            .expect("catalog should be persisted on ready");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].tool_name, "echo");
        drop(persisted);

        host.stop_server(config.server_id)
            .await
            .expect("stop should succeed");
        server.await.expect("mock server task should finish");
    }

    #[tokio::test]
    async fn warm_cache_describes_stopped_servers_without_making_them_callable() {
        let server_id = Uuid::new_v4();
        let descriptor = McpToolDescriptor {
            public_name: "mock__echo".to_string(),
            server_id,
            tool_name: "echo".to_string(),
            description: Some("echo text".to_string()),
            input_schema: json!({ "type": "object" }),
            annotations: json!({ "readOnlyHint": true }),
            meta: json!({}),
            permission_labels: vec!["read".to_string()],
        };
        let catalog = Arc::new(RecordingCatalogStore::default());
        catalog
            .replace_tools(server_id, std::slice::from_ref(&descriptor))
            .expect("seed catalog");

        let host = McpExtensionHost::new().with_tool_catalog_store(catalog);
        assert_eq!(host.warm_tool_cache().await.expect("warm cache"), 1);

        // The catalog is visible for a server that was never started in this process.
        let tools = host.cached_tools(server_id).await;
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].public_name, "mock__echo");

        // But a persisted descriptor must never be routable: routing requires a live client.
        let err = host
            .call_tool("mock__echo", json!({}))
            .await
            .expect_err("persisted tools must not be callable");
        assert!(
            matches!(err, McpHostError::ToolNotFound(name) if name == "mock__echo"),
            "expected the persisted tool to be absent from the routing table"
        );

        // Forgetting the server drops the warmed catalog.
        host.forget_server(server_id).await.expect("forget server");
        assert!(host.cached_tools(server_id).await.is_empty());
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
                                    },
                                    "_meta": {
                                        "com.opentopia/capabilities": ["fixture.echo/v1"]
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

    async fn run_slow_first_call_mcp_server(
        stdin: tokio::io::DuplexStream,
        mut stdout: tokio::io::DuplexStream,
    ) {
        let mut lines = BufReader::new(stdin).lines();
        let mut calls = 0usize;
        while let Some(line) = lines.next_line().await.expect("mock read should succeed") {
            let value: Value = serde_json::from_str(&line).expect("client message should be JSON");
            if value.get("method").and_then(Value::as_str) != Some("tools/call") {
                continue;
            }
            calls += 1;
            if calls == 1 {
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
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
                        "content": [{ "type": "text", "text": format!("echo: {text}") }],
                        "isError": false
                    }
                }),
            )
            .await;
        }
    }

    async fn run_out_of_order_mcp_server(
        stdin: tokio::io::DuplexStream,
        mut stdout: tokio::io::DuplexStream,
    ) {
        let mut lines = BufReader::new(stdin).lines();
        let mut calls = Vec::new();
        while calls.len() < 2 {
            let line = lines
                .next_line()
                .await
                .expect("mock read should succeed")
                .expect("client should send both calls");
            let value: Value = serde_json::from_str(&line).expect("client message should be JSON");
            if value.get("method").and_then(Value::as_str) == Some("tools/call") {
                calls.push((
                    value.get("id").cloned().unwrap_or(Value::Null),
                    value
                        .pointer("/params/arguments/text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                ));
            }
        }

        for (id, text) in calls.into_iter().rev() {
            write_json_line(
                &mut stdout,
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{ "type": "text", "text": format!("echo: {text}") }],
                        "isError": false
                    }
                }),
            )
            .await;
        }

        while lines
            .next_line()
            .await
            .expect("mock read should succeed")
            .is_some()
        {}
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

    #[derive(Clone, Copy)]
    enum GuardedRouteChange {
        Schema,
        Removed,
    }

    async fn run_guarded_route_server(
        stdin: tokio::io::DuplexStream,
        mut stdout: tokio::io::DuplexStream,
        mut change: oneshot::Receiver<()>,
        change_kind: GuardedRouteChange,
        list_calls: Arc<AtomicUsize>,
        tool_calls: Arc<AtomicUsize>,
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
                                        "serverInfo": { "name": "guarded", "version": "0.0.0" }
                                    }
                                }),
                            )
                            .await;
                        }
                        "notifications/initialized" => {}
                        "tools/list" => {
                            list_calls.fetch_add(1, Ordering::SeqCst);
                            let tools = if changed && matches!(change_kind, GuardedRouteChange::Removed) {
                                json!([])
                            } else {
                                let schema = if changed {
                                    json!({ "type": "object", "required": ["id"] })
                                } else {
                                    json!({ "type": "object" })
                                };
                                json!([{
                                    "name": "echo",
                                    "description": "Guarded echo",
                                    "inputSchema": schema
                                }])
                            };
                            write_json_line(
                                &mut stdout,
                                json!({
                                    "jsonrpc": "2.0",
                                    "id": value.get("id").cloned().unwrap_or(Value::Null),
                                    "result": { "tools": tools }
                                }),
                            )
                            .await;
                        }
                        "tools/call" => {
                            tool_calls.fetch_add(1, Ordering::SeqCst);
                            write_json_line(
                                &mut stdout,
                                json!({
                                    "jsonrpc": "2.0",
                                    "id": value.get("id").cloned().unwrap_or(Value::Null),
                                    "result": {
                                        "content": [{ "type": "text", "text": "called" }],
                                        "isError": false
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
