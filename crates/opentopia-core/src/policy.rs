use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    Chat,
    ReadOnly,
    Auto,
    Approve,
    FullAccess,
}

impl FromStr for PermissionMode {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "chat" => Ok(Self::Chat),
            "readonly" | "read_only" | "read-only" => Ok(Self::ReadOnly),
            "auto" => Ok(Self::Auto),
            "approve" => Ok(Self::Approve),
            "fullaccess" | "full_access" | "full-access" => Ok(Self::FullAccess),
            other => anyhow::bail!("unknown permission mode: {other}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum PolicyDecision {
    Allow,
    Deny { reason: String },
    Ask { reason: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PolicyRuleEffect {
    Allow,
    Ask,
    Deny,
}

impl PolicyRuleEffect {
    fn to_decision(self, reason: String) -> PolicyDecision {
        match self {
            Self::Allow => PolicyDecision::Allow,
            Self::Ask => PolicyDecision::Ask { reason },
            Self::Deny => PolicyDecision::Deny { reason },
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandRuleMatch {
    Prefix,
    Contains,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandPolicyRule {
    pub pattern: String,
    pub match_kind: CommandRuleMatch,
    pub effect: PolicyRuleEffect,
    pub reason: String,
    pub case_sensitive: bool,
}

impl CommandPolicyRule {
    pub fn prefix(
        pattern: impl Into<String>,
        effect: PolicyRuleEffect,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            pattern: pattern.into(),
            match_kind: CommandRuleMatch::Prefix,
            effect,
            reason: reason.into(),
            case_sensitive: false,
        }
    }

    pub fn contains(
        pattern: impl Into<String>,
        effect: PolicyRuleEffect,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            pattern: pattern.into(),
            match_kind: CommandRuleMatch::Contains,
            effect,
            reason: reason.into(),
            case_sensitive: false,
        }
    }

    fn matches(&self, command: &str) -> bool {
        let (command, pattern) = if self.case_sensitive {
            (command.to_string(), self.pattern.clone())
        } else {
            (
                command.to_ascii_lowercase(),
                self.pattern.to_ascii_lowercase(),
            )
        };

        match self.match_kind {
            CommandRuleMatch::Prefix => command.trim_start().starts_with(&pattern),
            CommandRuleMatch::Contains => command.contains(&pattern),
        }
    }

    fn decision(&self, command: &str) -> PolicyDecision {
        self.effect
            .to_decision(self.reason.replace("{command}", command))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkPolicyConfig {
    pub default_effect: PolicyRuleEffect,
    pub allowed_hosts: Vec<String>,
}

impl Default for NetworkPolicyConfig {
    fn default() -> Self {
        Self {
            default_effect: PolicyRuleEffect::Allow,
            allowed_hosts: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyConfig {
    pub command_rules: Vec<CommandPolicyRule>,
    pub network: NetworkPolicyConfig,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        let destructive_reason = "Potentially destructive command: {command}";
        Self {
            command_rules: vec![
                CommandPolicyRule::contains("rm -rf", PolicyRuleEffect::Ask, destructive_reason),
                CommandPolicyRule::contains("del /s", PolicyRuleEffect::Ask, destructive_reason),
                CommandPolicyRule::contains("format ", PolicyRuleEffect::Ask, destructive_reason),
                CommandPolicyRule::contains(
                    "git reset --hard",
                    PolicyRuleEffect::Ask,
                    destructive_reason,
                ),
                CommandPolicyRule::contains("sudo ", PolicyRuleEffect::Ask, destructive_reason),
            ],
            network: NetworkPolicyConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolPermissionDescriptor {
    pub source: String,
    pub name: String,
    pub permission_labels: Vec<String>,
    pub annotations: Value,
}

impl ToolPermissionDescriptor {
    pub fn new(
        source: impl Into<String>,
        name: impl Into<String>,
        permission_labels: Vec<String>,
        annotations: Value,
    ) -> Self {
        Self {
            source: source.into(),
            name: name.into(),
            permission_labels: permission_labels
                .into_iter()
                .map(|label| label.trim().to_ascii_lowercase())
                .filter(|label| !label.is_empty())
                .collect(),
            annotations,
        }
    }

    fn has_label(&self, candidates: &[&str]) -> bool {
        self.permission_labels.iter().any(|label| {
            candidates
                .iter()
                .any(|candidate| label.eq_ignore_ascii_case(candidate))
        })
    }

    fn annotation_bool(&self, key: &str) -> bool {
        self.annotations
            .get(key)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    fn labels_display(&self) -> String {
        if self.permission_labels.is_empty() {
            "unknown".to_string()
        } else {
            self.permission_labels.join(", ")
        }
    }
}

pub trait PolicyEngine: Send + Sync {
    fn inspect_read(&self, path: &Path) -> PolicyDecision;
    fn inspect_write(&self, path: &Path) -> PolicyDecision;
    fn inspect_command(&self, command: &str) -> PolicyDecision;
    fn inspect_mcp_tool_call(&self, descriptor: &ToolPermissionDescriptor) -> PolicyDecision {
        PolicyDecision::Ask {
            reason: format!(
                "MCP tool requires approval: {} [{}]",
                descriptor.name,
                descriptor.labels_display()
            ),
        }
    }
    fn inspect_network(&self, _host: &str) -> PolicyDecision {
        PolicyDecision::Allow
    }
}

#[derive(Debug, Clone)]
pub struct BasicPolicyEngine {
    workspace_root: PathBuf,
    mode: PermissionMode,
    config: PolicyConfig,
}

impl BasicPolicyEngine {
    pub fn new(workspace_root: PathBuf, mode: PermissionMode) -> Self {
        Self::new_with_config(workspace_root, mode, PolicyConfig::default())
    }

    pub fn new_with_config(
        workspace_root: PathBuf,
        mode: PermissionMode,
        config: PolicyConfig,
    ) -> Self {
        Self {
            workspace_root,
            mode,
            config,
        }
    }

    fn inside_workspace(&self, path: &Path) -> bool {
        if path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        {
            return false;
        }
        let workspace_root = self
            .workspace_root
            .canonicalize()
            .unwrap_or_else(|_| self.workspace_root.clone());
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            workspace_root.join(path)
        };
        let candidate = canonicalize_existing_ancestor(&candidate);
        candidate.starts_with(&workspace_root)
    }

    fn classify_mcp_tool(&self, descriptor: &ToolPermissionDescriptor) -> McpToolRisk {
        let has_read = descriptor.has_label(&["read", "readonly", "read_only"])
            || descriptor.annotation_bool("readOnlyHint");
        let has_write = descriptor.has_label(&["write", "modify", "mutation"]);
        let has_network = descriptor.has_label(&["network", "open_world", "openworld"])
            || descriptor.annotation_bool("openWorldHint");
        let has_secret = descriptor.has_label(&["secret", "secrets", "credential", "credentials"]);
        let has_destructive = descriptor.has_label(&["destructive", "delete", "dangerous"])
            || descriptor.annotation_bool("destructiveHint");
        let explicit_unknown = descriptor.has_label(&["unknown"]);

        if has_destructive {
            McpToolRisk::Destructive
        } else if has_secret {
            McpToolRisk::Secret
        } else if has_network {
            McpToolRisk::Network
        } else if has_write {
            McpToolRisk::Write
        } else if has_read && !explicit_unknown {
            McpToolRisk::ReadOnly
        } else {
            McpToolRisk::Unknown
        }
    }

    fn mcp_approval_reason(
        &self,
        descriptor: &ToolPermissionDescriptor,
        risk: McpToolRisk,
    ) -> String {
        format!(
            "MCP tool requires approval: {} ({}) [{}]",
            descriptor.name,
            risk.as_str(),
            descriptor.labels_display()
        )
    }
}

fn canonicalize_existing_ancestor(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }

    let mut cursor = path;
    let mut missing = Vec::new();
    while let Some(parent) = cursor.parent() {
        if let Some(name) = cursor.file_name() {
            missing.push(name.to_os_string());
        }
        if let Ok(mut canonical) = parent.canonicalize() {
            for component in missing.iter().rev() {
                canonical.push(component);
            }
            return canonical;
        }
        cursor = parent;
    }
    path.to_path_buf()
}

impl PolicyEngine for BasicPolicyEngine {
    fn inspect_read(&self, path: &Path) -> PolicyDecision {
        if !self.inside_workspace(path) {
            return PolicyDecision::Ask {
                reason: format!("Reading outside the workspace: {}", path.display()),
            };
        }

        match self.mode {
            PermissionMode::Chat => PolicyDecision::Deny {
                reason: "Chat mode does not allow file access.".to_string(),
            },
            _ => PolicyDecision::Allow,
        }
    }

    fn inspect_write(&self, path: &Path) -> PolicyDecision {
        if !self.inside_workspace(path) {
            return PolicyDecision::Ask {
                reason: format!("Writing outside the workspace: {}", path.display()),
            };
        }

        match self.mode {
            PermissionMode::Chat | PermissionMode::ReadOnly => PolicyDecision::Deny {
                reason: "Current permission mode does not allow writes.".to_string(),
            },
            PermissionMode::Approve => PolicyDecision::Ask {
                reason: format!("Write requires approval: {}", path.display()),
            },
            PermissionMode::Auto | PermissionMode::FullAccess => PolicyDecision::Allow,
        }
    }

    fn inspect_command(&self, command: &str) -> PolicyDecision {
        for rule in &self.config.command_rules {
            if rule.matches(command) {
                let decision = rule.decision(command);
                if matches!(decision, PolicyDecision::Ask { .. })
                    && self.mode == PermissionMode::FullAccess
                {
                    return PolicyDecision::Allow;
                }
                return decision;
            }
        }

        match self.mode {
            PermissionMode::Chat | PermissionMode::ReadOnly => PolicyDecision::Deny {
                reason: "Current permission mode does not allow shell commands.".to_string(),
            },
            PermissionMode::Approve => PolicyDecision::Ask {
                reason: format!("Command requires approval: {command}"),
            },
            PermissionMode::Auto | PermissionMode::FullAccess => PolicyDecision::Allow,
        }
    }

    fn inspect_mcp_tool_call(&self, descriptor: &ToolPermissionDescriptor) -> PolicyDecision {
        let risk = self.classify_mcp_tool(descriptor);
        match self.mode {
            PermissionMode::Chat => PolicyDecision::Deny {
                reason: "Chat mode does not allow MCP tool calls.".to_string(),
            },
            PermissionMode::ReadOnly => {
                if risk == McpToolRisk::ReadOnly {
                    PolicyDecision::Allow
                } else {
                    PolicyDecision::Deny {
                        reason: format!(
                            "Read-only mode only allows MCP tools annotated as read-only: {} ({}) [{}]",
                            descriptor.name,
                            risk.as_str(),
                            descriptor.labels_display()
                        ),
                    }
                }
            }
            PermissionMode::Approve | PermissionMode::Auto => {
                if risk == McpToolRisk::ReadOnly {
                    PolicyDecision::Allow
                } else {
                    PolicyDecision::Ask {
                        reason: self.mcp_approval_reason(descriptor, risk),
                    }
                }
            }
            PermissionMode::FullAccess => PolicyDecision::Allow,
        }
    }

    fn inspect_network(&self, host: &str) -> PolicyDecision {
        let host = host.trim();
        if host.is_empty() {
            return PolicyDecision::Deny {
                reason: "Network host cannot be empty.".to_string(),
            };
        }

        if self
            .config
            .network
            .allowed_hosts
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(host))
        {
            return PolicyDecision::Allow;
        }

        self.config
            .network
            .default_effect
            .to_decision(format!("Network access requires approval: {host}"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum McpToolRisk {
    ReadOnly,
    Write,
    Network,
    Secret,
    Destructive,
    Unknown,
}

impl McpToolRisk {
    fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::Write => "write",
            Self::Network => "network",
            Self::Secret => "secret",
            Self::Destructive => "destructive",
            Self::Unknown => "unknown",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn policy(mode: PermissionMode) -> BasicPolicyEngine {
        BasicPolicyEngine::new(PathBuf::from("."), mode)
    }

    fn descriptor(labels: &[&str], annotations: Value) -> ToolPermissionDescriptor {
        ToolPermissionDescriptor::new(
            "mcp",
            "server__tool",
            labels.iter().map(|label| label.to_string()).collect(),
            annotations,
        )
    }

    #[test]
    fn read_only_mcp_tool_is_allowed_in_read_only_mode() {
        let decision = policy(PermissionMode::ReadOnly)
            .inspect_mcp_tool_call(&descriptor(&["read"], json!({ "readOnlyHint": true })));
        assert!(matches!(decision, PolicyDecision::Allow));
    }

    #[test]
    fn unknown_mcp_tool_requires_approval_in_auto_mode() {
        let decision = policy(PermissionMode::Auto)
            .inspect_mcp_tool_call(&descriptor(&["unknown"], json!({})));
        assert!(matches!(decision, PolicyDecision::Ask { .. }));
    }

    #[test]
    fn destructive_mcp_tool_is_denied_in_read_only_mode() {
        let decision = policy(PermissionMode::ReadOnly).inspect_mcp_tool_call(&descriptor(
            &["destructive"],
            json!({ "destructiveHint": true }),
        ));
        assert!(matches!(decision, PolicyDecision::Deny { .. }));
    }
}
