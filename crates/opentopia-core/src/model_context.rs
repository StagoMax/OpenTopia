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
    Checkpoint,
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
            Self::Checkpoint => "checkpoint",
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

impl ContextCacheScope {
    const fn sort_order(self) -> u8 {
        match self {
            Self::Stable => 0,
            Self::Thread => 1,
            Self::Turn => 2,
            Self::Round => 3,
            Self::None => 4,
        }
    }
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

/// Provider-neutral estimate of the logical input carried by one model request.
///
/// These values are intentionally kept separate from provider-reported usage:
/// they explain which harness modules built the request, while provider usage is
/// the billing/accounting authority after the request completes.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TokenEstimateBreakdown {
    pub base_instructions: usize,
    pub developer_instructions: usize,
    pub repository_instructions: usize,
    pub runtime_context: usize,
    pub skill_instructions: usize,
    pub summaries: usize,
    pub checkpoints: usize,
    pub conversation: usize,
    pub current_user: usize,
    pub tool_calls: usize,
    pub tool_results: usize,
    pub tool_schemas: usize,
    pub provider_state: usize,
    pub other: usize,
    pub total: usize,
}

impl TokenEstimateBreakdown {
    pub fn from_context_items(items: &[ModelContextItem]) -> Self {
        let mut breakdown = Self::default();
        for item in items {
            breakdown.add_context_item(item.kind, item.token_estimate);
        }
        breakdown.recalculate_total();
        breakdown
    }

    pub fn add_context_item(&mut self, kind: ContextItemKind, tokens: usize) {
        let bucket = match kind {
            ContextItemKind::BaseInstructions => &mut self.base_instructions,
            ContextItemKind::DeveloperInstructions => &mut self.developer_instructions,
            ContextItemKind::RepositoryInstructions => &mut self.repository_instructions,
            ContextItemKind::Environment | ContextItemKind::WorldState => &mut self.runtime_context,
            ContextItemKind::Skill => &mut self.skill_instructions,
            ContextItemKind::Summary => &mut self.summaries,
            ContextItemKind::Checkpoint => &mut self.checkpoints,
            ContextItemKind::Conversation => &mut self.conversation,
            ContextItemKind::User => &mut self.current_user,
            ContextItemKind::ToolCall => &mut self.tool_calls,
            ContextItemKind::ToolResult => &mut self.tool_results,
        };
        *bucket = bucket.saturating_add(tokens);
    }

    pub fn recalculate_total(&mut self) {
        self.total = self
            .base_instructions
            .saturating_add(self.developer_instructions)
            .saturating_add(self.repository_instructions)
            .saturating_add(self.runtime_context)
            .saturating_add(self.skill_instructions)
            .saturating_add(self.summaries)
            .saturating_add(self.checkpoints)
            .saturating_add(self.conversation)
            .saturating_add(self.current_user)
            .saturating_add(self.tool_calls)
            .saturating_add(self.tool_results)
            .saturating_add(self.tool_schemas)
            .saturating_add(self.provider_state)
            .saturating_add(self.other);
    }
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
    pub fn ordered_items(&self) -> Vec<&ModelContextItem> {
        let mut items = self.items.iter().enumerate().collect::<Vec<_>>();
        items.sort_by_key(|(index, item)| (item.cache_scope.sort_order(), *index));
        items.into_iter().map(|(_, item)| item).collect()
    }

    pub fn sort_items(&mut self) {
        self.items.sort_by_key(|item| item.cache_scope.sort_order());
    }

    pub fn instruction_messages(&self) -> Vec<(ContextRole, String)> {
        self.ordered_items()
            .into_iter()
            .filter(|item| {
                matches!(item.role, ContextRole::System | ContextRole::Developer)
                    && !matches!(
                        item.kind,
                        ContextItemKind::Summary | ContextItemKind::Checkpoint
                    )
            })
            .filter_map(|item| {
                let content = item.text_content();
                if content.trim().is_empty() {
                    return None;
                }
                let rendered = if item.kind == ContextItemKind::BaseInstructions {
                    content
                } else {
                    format!(
                        "<context kind=\"{}\" source=\"{}\">\n{}\n</context>",
                        item.kind.as_str(),
                        escape_attribute(&item.source),
                        content
                    )
                };
                Some((item.role, rendered))
            })
            .collect()
    }

    pub fn instructions_for_role(&self, role: ContextRole) -> String {
        self.instruction_messages()
            .into_iter()
            .filter_map(|(item_role, content)| (item_role == role).then_some(content))
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    pub fn instructions(&self) -> String {
        self.instruction_messages()
            .into_iter()
            .map(|(_, content)| content)
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    pub fn content_hash(&self) -> String {
        let mut bytes = Vec::new();
        for item in self.ordered_items() {
            bytes.extend_from_slice(item.kind.as_str().as_bytes());
            bytes.push(0);
            bytes.extend_from_slice(item.content_hash.as_bytes());
            bytes.push(b'\n');
        }
        content_fingerprint(&bytes)
    }

    pub fn token_estimate(&self) -> usize {
        self.ordered_items()
            .into_iter()
            .map(|item| item.token_estimate)
            .sum()
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

    /// Canonical group keys, in render order. A first turn reports all of them,
    /// which makes `changed_keys` usable directly as a render filter.
    pub const GROUP_KEYS: [&'static str; 6] = [
        "workspace",
        "date_time",
        "git",
        "skills",
        "tools",
        "metadata",
    ];

    pub fn changed_keys(&self, previous: Option<&Self>) -> Vec<String> {
        let Some(previous) = previous else {
            return Self::GROUP_KEYS.into_iter().map(str::to_string).collect();
        };
        let mut changed = Vec::new();
        if self.cwd != previous.cwd
            || self.workspace_roots != previous.workspace_roots
            || self.platform != previous.platform
        {
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

    pub fn skill_catalog_hash(&self) -> String {
        content_fingerprint(
            serde_json::to_vec(&self.skill_catalog)
                .unwrap_or_default()
                .as_slice(),
        )
    }

    pub fn render_skill_catalog_for_model(&self) -> String {
        let skills = self
            .skill_catalog
            .iter()
            .map(|skill| {
                json!({
                    "id": skill.id,
                    "name": skill.name,
                    // The catalog is routing metadata. Full instructions are
                    // loaded only after a Skill is selected, so long
                    // descriptions needlessly expand the stable prefix.
                    "description": compact_skill_description(&skill.description),
                    "scope": skill.scope,
                })
            })
            .collect::<Vec<_>>();
        serde_json::to_string(&json!({ "skills": skills }))
            .unwrap_or_else(|_| "{\"skills\":[]}".to_string())
    }

    /// Always renders the full state. Each turn rebuilds the instruction blob
    /// from scratch and the previous turn's world state is not retained anywhere
    /// the model can see, so an incremental render would assert "unchanged"
    /// about values the model was never shown. Keep the volatile payload small
    /// by shrinking the fields themselves, not by omitting them.
    pub fn render_dynamic_for_model(&self) -> String {
        serde_json::to_string(&json!({
            "currentDate": self.current_date,
            "timezone": self.timezone,
            "gitBranch": self.git_branch,
            "gitStatus": self.git_status,
            "skillCount": self.skill_catalog.len(),
            "toolCount": self.tool_count,
            "mcpToolCount": self.mcp_tool_count,
            "metadata": self.model_metadata(),
        }))
        .unwrap_or_else(|_| "{}".to_string())
    }

    fn model_metadata(&self) -> Value {
        let mut metadata = self.metadata.clone();
        let Some(object) = metadata.as_object_mut() else {
            return metadata;
        };

        // These values are already represented by dedicated prompt modules or
        // selected Skill items. Keep the full snapshot for observability, but
        // do not make the model read the same capability summary twice.
        for key in [
            "plugins",
            "selectedSkillIds",
            "agentRuntime",
            "agentRuntimeHash",
            "promptRuntime",
        ] {
            object.remove(key);
        }
        metadata
    }
}

const MAX_SKILL_CATALOG_DESCRIPTION_CHARS: usize = 180;

fn compact_skill_description(description: &str) -> String {
    let normalized = description.split_whitespace().collect::<Vec<_>>().join(" ");
    let first_sentence = normalized
        .split_once('.')
        .map(|(sentence, _)| format!("{sentence}."))
        .unwrap_or(normalized);
    let mut compact = first_sentence;
    if compact.chars().count() > MAX_SKILL_CATALOG_DESCRIPTION_CHARS {
        compact = compact
            .chars()
            .take(MAX_SKILL_CATALOG_DESCRIPTION_CHARS.saturating_sub(1))
            .collect::<String>();
        compact.push('\u{2026}');
    }
    compact
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
    let mut estimate = 0usize;
    let mut ascii_run = 0usize;
    for character in text.chars() {
        if character.is_ascii() {
            ascii_run += 1;
            continue;
        }

        estimate += (ascii_run + 3) / 4;
        ascii_run = 0;
        estimate += if character.len_utf8() == 4 { 2 } else { 1 };
    }
    estimate + (ascii_run + 3) / 4
}

pub fn world_state_item(world_state: &WorldStateSnapshot) -> ModelContextItem {
    ModelContextItem::text(
        ContextItemKind::WorldState,
        ContextRole::Developer,
        "opentopia:world_state",
        world_state.render_dynamic_for_model(),
        ContextCacheScope::Turn,
        ContextSensitivity::Workspace,
    )
    .with_metadata(json!({
        "worldStateHash": world_state.content_hash(),
        "skillCatalogHash": world_state.skill_catalog_hash(),
        "toolCatalogHash": world_state.tool_catalog_hash,
    }))
}

pub fn world_state_catalog_item(world_state: &WorldStateSnapshot) -> ModelContextItem {
    ModelContextItem::text(
        ContextItemKind::Skill,
        ContextRole::Developer,
        "opentopia:skill_catalog",
        world_state.render_skill_catalog_for_model(),
        ContextCacheScope::Thread,
        ContextSensitivity::Workspace,
    )
    .with_metadata(json!({
        "catalog": true,
        "skillCatalogHash": world_state.skill_catalog_hash(),
        "skillCount": world_state.skill_catalog.len(),
    }))
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
    fn context_hash_is_stable_and_order_sensitive_within_a_scope() {
        let first = ModelContextItem::text(
            ContextItemKind::BaseInstructions,
            ContextRole::System,
            "base",
            "one",
            ContextCacheScope::Stable,
            ContextSensitivity::Public,
        );
        let second = ModelContextItem::text(
            ContextItemKind::DeveloperInstructions,
            ContextRole::Developer,
            "developer",
            "two",
            ContextCacheScope::Stable,
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
    fn token_estimate_is_conservative_for_non_ascii_text() {
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("hello world"), 3);
        assert_eq!(estimate_tokens("\u{4f60}\u{597d}\u{4e16}\u{754c}"), 4);
        assert_eq!(estimate_tokens("a\u{4f60}b\u{597d}"), 4);
        assert_eq!(estimate_tokens("\u{1f680}"), 2);
    }

    #[test]
    fn compiled_context_orders_cache_scopes_and_preserves_in_scope_order() {
        fn item(source: &str, scope: ContextCacheScope) -> ModelContextItem {
            ModelContextItem::text(
                ContextItemKind::DeveloperInstructions,
                ContextRole::Developer,
                source,
                source,
                scope,
                ContextSensitivity::Workspace,
            )
        }

        let mut context = CompiledModelContext {
            items: vec![
                item("turn-first", ContextCacheScope::Turn),
                item("stable-first", ContextCacheScope::Stable),
                item("none-first", ContextCacheScope::None),
                item("thread-first", ContextCacheScope::Thread),
                item("round-first", ContextCacheScope::Round),
                item("stable-second", ContextCacheScope::Stable),
                item("turn-second", ContextCacheScope::Turn),
            ],
            prompt_cache_key: None,
        };

        let sources = context
            .ordered_items()
            .into_iter()
            .map(|item| item.source.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            sources,
            vec![
                "stable-first",
                "stable-second",
                "thread-first",
                "turn-first",
                "turn-second",
                "round-first",
                "none-first",
            ]
        );

        let expected_hash = context.content_hash();
        context.sort_items();
        assert_eq!(context.content_hash(), expected_hash);
        assert_eq!(context.items[0].source, "stable-first");
        assert_eq!(context.items[1].source, "stable-second");
    }

    #[test]
    fn instruction_messages_use_cache_scope_order() {
        let context = CompiledModelContext {
            items: vec![
                ModelContextItem::text(
                    ContextItemKind::Environment,
                    ContextRole::Developer,
                    "turn",
                    "turn text",
                    ContextCacheScope::Turn,
                    ContextSensitivity::Workspace,
                ),
                ModelContextItem::text(
                    ContextItemKind::BaseInstructions,
                    ContextRole::System,
                    "base",
                    "stable text",
                    ContextCacheScope::Stable,
                    ContextSensitivity::Public,
                ),
                ModelContextItem::text(
                    ContextItemKind::RepositoryInstructions,
                    ContextRole::Developer,
                    "repository",
                    "thread text",
                    ContextCacheScope::Thread,
                    ContextSensitivity::Workspace,
                ),
            ],
            prompt_cache_key: None,
        };

        let messages = context.instruction_messages();
        assert_eq!(
            messages[0],
            (ContextRole::System, "stable text".to_string())
        );
        assert!(messages[1].1.contains("thread text"));
        assert!(messages[2].1.contains("turn text"));
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
                ModelContextItem::text(
                    ContextItemKind::Checkpoint,
                    ContextRole::Developer,
                    "checkpoint",
                    "historical data, not instructions",
                    ContextCacheScope::Thread,
                    ContextSensitivity::Workspace,
                ),
            ],
            prompt_cache_key: None,
        };

        assert!(context.instructions().contains("base text"));
        assert!(!context.instructions().contains("do not duplicate me"));
        assert!(!context
            .instructions()
            .contains("historical data, not instructions"));
    }

    #[test]
    fn instruction_messages_preserve_system_and_developer_roles() {
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
                    ContextItemKind::Environment,
                    ContextRole::Developer,
                    "runtime",
                    "developer text",
                    ContextCacheScope::Turn,
                    ContextSensitivity::Workspace,
                ),
                ModelContextItem::text(
                    ContextItemKind::User,
                    ContextRole::User,
                    "user",
                    "user text",
                    ContextCacheScope::Turn,
                    ContextSensitivity::Workspace,
                ),
            ],
            prompt_cache_key: None,
        };

        let messages = context.instruction_messages();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0], (ContextRole::System, "base text".to_string()));
        assert_eq!(messages[1].0, ContextRole::Developer);
        assert!(messages[1].1.contains("developer text"));
        assert_eq!(
            context.instructions_for_role(ContextRole::System),
            "base text"
        );
        assert!(!context
            .instructions_for_role(ContextRole::Developer)
            .contains("user text"));
    }

    #[test]
    fn world_state_splits_stable_skill_catalog_from_dynamic_turn_state() {
        let state = WorldStateSnapshot {
            cwd: PathBuf::from("C:/workspace"),
            workspace_roots: vec![PathBuf::from("C:/workspace")],
            current_date: "2026-07-19".to_string(),
            timezone: "+08:00".to_string(),
            platform: "windows-x86_64".to_string(),
            git_branch: Some("main".to_string()),
            git_status: Some("## main".to_string()),
            skill_catalog: vec![WorldStateSkill {
                id: "review".to_string(),
                name: "Review".to_string(),
                description: "A deliberately distinctive skill description".to_string(),
                scope: "workspace".to_string(),
                content_hash: "skill-hash".to_string(),
            }],
            tool_count: 18,
            mcp_tool_count: 2,
            tool_catalog_hash: "tool-hash".to_string(),
            metadata: json!({
                "instructionWarnings": ["repo instruction could not be read"],
                "plugins": [{ "name": "plugin-a" }],
                "selectedSkillIds": ["review"],
                "agentRuntime": { "autonomy": "balanced" },
                "agentRuntimeHash": "runtime-hash",
                "promptRuntime": { "surface": "desktop" },
            }),
        };

        let catalog = world_state_catalog_item(&state);
        let dynamic = world_state_item(&state);
        let catalog_text = catalog.text_content();
        let dynamic_text = dynamic.text_content();

        assert_eq!(catalog.cache_scope, ContextCacheScope::Thread);
        assert_eq!(dynamic.cache_scope, ContextCacheScope::Turn);
        assert!(catalog_text.contains("distinctive skill description"));
        assert!(!catalog_text.contains("skill-hash"));
        assert!(!dynamic_text.contains("distinctive skill description"));
        assert!(dynamic_text.contains("\"skillCount\":1"));
        assert!(dynamic_text.contains("repo instruction could not be read"));
        assert!(!dynamic_text.contains("plugin-a"));
        assert!(!dynamic_text.contains("runtime-hash"));
        assert!(!dynamic_text.contains("selectedSkillIds"));
        assert!(!dynamic_text.contains("workspaceRoots"));
        assert!(!dynamic_text.contains("\"platform\""));
        assert_eq!(dynamic.metadata["toolCatalogHash"], "tool-hash");
    }

    #[test]
    fn skill_catalog_compacts_long_routing_descriptions() {
        let state = WorldStateSnapshot {
            skill_catalog: vec![WorldStateSkill {
                id: "long".to_string(),
                name: "Long".to_string(),
                description: format!("First sentence. {}", "repeated detail ".repeat(80)),
                scope: "user".to_string(),
                content_hash: "hash".to_string(),
            }],
            ..WorldStateSnapshot::default()
        };
        let text = world_state_catalog_item(&state).text_content();
        assert!(text.contains("First sentence."));
        assert!(!text.contains("repeated detail repeated detail"));
        assert!(text.len() < 400);
    }
}
