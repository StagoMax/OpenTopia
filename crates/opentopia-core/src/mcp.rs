use crate::policy::ToolPermissionDescriptor;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerConfig {
    pub server_id: Uuid,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env_keys: Vec<String>,
    pub timeout_ms: u64,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl McpServerConfig {
    pub fn new(name: String, command: String) -> Self {
        let now = Utc::now();
        Self {
            server_id: Uuid::new_v4(),
            name,
            command,
            args: Vec::new(),
            cwd: None,
            env_keys: Vec::new(),
            timeout_ms: 30_000,
            enabled: true,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn refresh_updated_at(&mut self) {
        self.updated_at = Utc::now();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpLifecycleStatus {
    NotStarted,
    Starting,
    Ready,
    Error,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerStatus {
    pub server_id: Uuid,
    pub name: String,
    pub status: McpLifecycleStatus,
    pub message: String,
    pub tools_count: usize,
    pub updated_at: DateTime<Utc>,
}

impl McpServerStatus {
    pub fn from_config(config: &McpServerConfig) -> Self {
        let status = if config.enabled {
            McpLifecycleStatus::NotStarted
        } else {
            McpLifecycleStatus::Disabled
        };
        Self {
            server_id: config.server_id,
            name: config.name.clone(),
            status,
            message: "MCP stdio host boundary is configured; process lifecycle is not started in this skeleton.".to_string(),
            tools_count: 0,
            updated_at: config.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolDescriptor {
    pub public_name: String,
    pub server_id: Uuid,
    pub tool_name: String,
    pub description: Option<String>,
    pub input_schema: Value,
    pub annotations: Value,
    pub permission_labels: Vec<String>,
}

impl From<&McpToolDescriptor> for ToolPermissionDescriptor {
    fn from(value: &McpToolDescriptor) -> Self {
        ToolPermissionDescriptor::new(
            "mcp",
            value.public_name.clone(),
            value.permission_labels.clone(),
            value.annotations.clone(),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpCallResult {
    pub server_id: Uuid,
    pub public_name: String,
    pub tool_name: String,
    pub output: String,
    pub content: Vec<Value>,
    pub structured_content: Option<Value>,
    pub is_error: bool,
    pub raw: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadMcpServer {
    pub thread_id: Uuid,
    pub server_id: Uuid,
    pub enabled: bool,
    pub updated_at: DateTime<Utc>,
}

pub fn mcp_public_tool_name(server_name: &str, tool_name: &str) -> String {
    format!(
        "{}__{}",
        mcp_tool_name_segment(server_name, "server"),
        mcp_tool_name_segment(tool_name, "tool")
    )
}

pub fn mcp_default_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {}
    })
}

fn mcp_tool_name_segment(value: &str, fallback: &str) -> String {
    let mut output = String::new();
    let mut last_was_separator = false;

    for character in value.trim().chars() {
        if character.is_ascii_alphanumeric() {
            output.push(character.to_ascii_lowercase());
            last_was_separator = false;
        } else if character == '_' || character == '-' || character.is_whitespace() {
            if !last_was_separator && !output.is_empty() {
                output.push('_');
                last_was_separator = true;
            }
        }
    }

    while output.ends_with('_') {
        output.pop();
    }

    if output.is_empty() {
        fallback.to_string()
    } else {
        output
    }
}
