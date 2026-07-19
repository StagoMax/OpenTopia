use crate::model::ModelContentPart;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextItemKind {
    BaseInstructions,
    DeveloperInstructions,
    RepositoryInstructions,
    Environment,
    WorldState,
    Skill,
    Summary,
    Conversation,
    User,
    ToolCall,
    ToolResult,
}

impl ContextItemKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BaseInstructions => "base_instructions",
            Self::DeveloperInstructions => "developer_instructions",
            Self::RepositoryInstructions => "repository_instructions",
            Self::Environment => "environment",
            Self::WorldState => "world_state",
            Self::Skill => "skill",
            Self::Summary => "summary",
            Self::Conversation => "conversation",
            Self::User => "user",
            Self::ToolCall => "tool_call",
            Self::ToolResult => "tool_result",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ContextRole {
    System,
    Developer,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextCacheScope {
    Stable,
    Thread,
    Turn,
    Round,
    None,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextSensitivity {
    Public,
    Workspace,
    Sensitive,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelContextItem {
    pub id: String,
    pub kind: ContextItemKind,
    pub role: ContextRole,
    pub source: String,
    pub content: Vec<ModelContentPart>,
    pub content_hash: String,
    pub token_estimate: usize,
    pub cache_scope: ContextCacheScope,
    pub sensitivity: ContextSensitivity,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub metadata: Value,
}

impl ModelContextItem {
    pub fn text(
        kind: ContextItemKind,
        role: ContextRole,
        source: impl Into<String>,
        text: impl Into<String>,
        cache_scope: ContextCacheScope,
        sensitivity: ContextSensitivity,
    ) -> Self {
        let source = source.into();
        let text = text.into();
        let content_hash = content_fingerprint(text.as_bytes());
        Self {
            id: format!("{}:{content_hash}", kind.as_str()),
            kind,
            role,
            source,
            token_estimate: estimate_tokens(&text),
            content: vec![ModelContentPart::text(text)],
            content_hash,
            cache_scope,
            sensitivity,
            metadata: Value::Null,
        }
    }

    pub fn with_metadata(mut self, metadata: Value) -> Self {
        self.metadata = metadata;
        self
    }

    pub fn text_content(&self) -> String {
        self.content
            .iter()
            .map(|part| match part {
                ModelContentPart::Text { text } => text.clone(),
                ModelContentPart::Json { value } => value.to_string(),
                ModelContentPart::Image { content_type, data } => {
                    format!("[image {content_type}, {} bytes]", data.len())
                }
                ModelContentPart::Resource {
                    uri,
                    content_type,
                    name,
                } => format!(
                    "[resource uri={uri}, type={}, name={}]",
                    content_type.as_deref().unwrap_or("unknown"),
                    name.as_deref().unwrap_or("unnamed")
                ),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CompiledModelContext {
    #[serde(default)]
    pub items: Vec<ModelContextItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
}

impl CompiledModelContext {
    pub fn instructions(&self) -> String {
        let mut rendered = Vec::new();
        for item in self.items.iter().filter(|item| {
            matches!(item.role, ContextRole::System | ContextRole::Developer)
                && !matches!(item.kind, ContextItemKind::Summary)
        }) {
            let content = item.text_content();
            if content.trim().is_empty() {
                continue;
            }
            if item.kind == ContextItemKind::BaseInstructions {
                rendered.push(content);
            } else {
                rendered.push(format!(
                    "<context kind=\"{}\" source=\"{}\">\n{}\n</context>",
                    item.kind.as_str(),
                    escape_attribute(&item.source),
                    content
                ));
            }
        }
        rendered.join("\n\n")
    }

    pub fn content_hash(&self) -> String {
        let mut bytes = Vec::new();
        for item in &self.items {
            bytes.extend_from_slice(item.kind.as_str().as_bytes());
            bytes.push(0);
            bytes.extend_from_slice(item.content_hash.as_bytes());
            bytes.push(b'\n');
        }
        content_fingerprint(&bytes)
    }

    pub fn token_estimate(&self) -> usize {
        self.items.iter().map(|item| item.token_estimate).sum()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstructionSnapshotRef {
    pub scope: String,
    pub path: PathBuf,
    pub content_hash: String,
    pub bytes: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorldStateSnapshot {
    pub cwd: PathBuf,
    #[serde(default)]
    pub workspace_roots: Vec<PathBuf>,
    pub current_date: String,
    pub timezone: String,
    pub platform: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_status: Option<String>,
    #[serde(default)]
    pub skill_catalog: Vec<WorldStateSkill>,
    pub tool_count: usize,
    pub mcp_tool_count: usize,
    pub tool_catalog_hash: String,
    #[serde(default)]
    pub metadata: Value,
}

impl WorldStateSnapshot {
    pub fn content_hash(&self) -> String {
        content_fingerprint(serde_json::to_vec(self).unwrap_or_default().as_slice())
    }

    pub fn changed_keys(&self, previous: Option<&Self>) -> Vec<String> {
        let Some(previous) = previous else {
            return vec![
                "cwd",
                "workspace_roots",
                "date_time",
                "git",
                "skills",
                "tools",
            ]
            .into_iter()
            .map(str::to_string)
            .collect();
        };
        let mut changed = Vec::new();
        if self.cwd != previous.cwd || self.workspace_roots != previous.workspace_roots {
            changed.push("workspace".to_string());
        }
        if self.current_date != previous.current_date || self.timezone != previous.timezone {
            changed.push("date_time".to_string());
        }
        if self.git_branch != previous.git_branch || self.git_status != previous.git_status {
            changed.push("git".to_string());
        }
        if self.skill_catalog != previous.skill_catalog {
            changed.push("skills".to_string());
        }
        if self.tool_catalog_hash != previous.tool_catalog_hash
            || self.tool_count != previous.tool_count
            || self.mcp_tool_count != previous.mcp_tool_count
        {
            changed.push("tools".to_string());
        }
        if self.metadata != previous.metadata {
            changed.push("metadata".to_string());
        }
        changed
    }

    pub fn render_for_model(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorldStateSkill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub scope: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadContextSnapshot {
    pub captured_at: DateTime<Utc>,
    pub provider_id: String,
    pub provider_kind: String,
    pub model: String,
    pub workspace_root: PathBuf,
    pub cwd: PathBuf,
    #[serde(default)]
    pub experience_mode: String,
    pub permission_mode: String,
    pub sandbox_mode: String,
    #[serde(default)]
    pub instructions: Vec<InstructionSnapshotRef>,
    pub tool_catalog_hash: String,
    pub world_state_hash: String,
    pub context_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TurnContextSnapshot {
    pub captured_at: DateTime<Utc>,
    pub cwd: PathBuf,
    #[serde(default)]
    pub workspace_roots: Vec<PathBuf>,
    #[serde(default)]
    pub experience_mode: String,
    pub permission_mode: String,
    pub sandbox_mode: String,
    #[serde(default)]
    pub instructions: Vec<InstructionSnapshotRef>,
    pub world_state: WorldStateSnapshot,
    pub world_state_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_world_state_hash: Option<String>,
    #[serde(default)]
    pub changed_keys: Vec<String>,
    pub context_hash: String,
}

pub fn content_fingerprint(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

pub fn estimate_tokens(text: &str) -> usize {
    (text.len() + 3) / 4
}

pub fn world_state_item(world_state: &WorldStateSnapshot) -> ModelContextItem {
    ModelContextItem::text(
        ContextItemKind::WorldState,
        ContextRole::Developer,
        "opentopia:world_state",
        world_state.render_for_model(),
        ContextCacheScope::Turn,
        ContextSensitivity::Workspace,
    )
    .with_metadata(json!({ "worldStateHash": world_state.content_hash() }))
}

fn escape_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_hash_is_stable_and_order_sensitive() {
        let first = ModelContextItem::text(
            ContextItemKind::BaseInstructions,
            ContextRole::System,
            "base",
            "one",
            ContextCacheScope::Stable,
            ContextSensitivity::Public,
        );
        let second = ModelContextItem::text(
            ContextItemKind::Environment,
            ContextRole::Developer,
            "environment",
            "two",
            ContextCacheScope::Turn,
            ContextSensitivity::Workspace,
        );
        let left = CompiledModelContext {
            items: vec![first.clone(), second.clone()],
            prompt_cache_key: None,
        };
        let right = CompiledModelContext {
            items: vec![second, first],
            prompt_cache_key: None,
        };

        assert_eq!(left.content_hash(), left.content_hash());
        assert_ne!(left.content_hash(), right.content_hash());
    }

    #[test]
    fn instructions_include_only_system_and_developer_layers() {
        let context = CompiledModelContext {
            items: vec![
                ModelContextItem::text(
                    ContextItemKind::BaseInstructions,
                    ContextRole::System,
                    "base",
                    "base text",
                    ContextCacheScope::Stable,
                    ContextSensitivity::Public,
                ),
                ModelContextItem::text(
                    ContextItemKind::User,
                    ContextRole::User,
                    "user",
                    "do not duplicate me",
                    ContextCacheScope::Turn,
                    ContextSensitivity::Workspace,
                ),
            ],
            prompt_cache_key: None,
        };

        assert!(context.instructions().contains("base text"));
        assert!(!context.instructions().contains("do not duplicate me"));
    }
}
