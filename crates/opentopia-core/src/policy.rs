use crate::sandbox::{LocalSandboxConfig, SandboxMode};
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPolicy {
    OnRequest,
    Never,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalsReviewer {
    User,
    AutoReview,
}

impl PermissionMode {
    pub fn approval_policy(self) -> ApprovalPolicy {
        match self {
            Self::FullAccess => ApprovalPolicy::Never,
            Self::Chat | Self::ReadOnly | Self::Auto | Self::Approve => ApprovalPolicy::OnRequest,
        }
    }

    pub fn approvals_reviewer(self) -> ApprovalsReviewer {
        match self {
            Self::Auto => ApprovalsReviewer::AutoReview,
            Self::Chat | Self::ReadOnly | Self::Approve | Self::FullAccess => {
                ApprovalsReviewer::User
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("approval required: {reason}")]
pub struct ApprovalRequired {
    reason: String,
}

impl ApprovalRequired {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }
}

pub fn approval_required(error: &anyhow::Error) -> Option<&ApprovalRequired> {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<ApprovalRequired>())
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
            default_effect: PolicyRuleEffect::Ask,
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
    readable_roots: Vec<PathBuf>,
    writable_roots: Vec<PathBuf>,
    unrestricted_file_access: bool,
    mode: PermissionMode,
    config: PolicyConfig,
}

impl BasicPolicyEngine {
    pub fn new(workspace_root: PathBuf, mode: PermissionMode) -> Self {
        Self::new_with_config_and_sandbox(
            workspace_root,
            mode,
            PolicyConfig::default(),
            &LocalSandboxConfig::default(),
        )
    }

    pub fn new_with_sandbox_config(
        workspace_root: PathBuf,
        mode: PermissionMode,
        sandbox_config: &LocalSandboxConfig,
    ) -> Self {
        Self::new_with_config_and_sandbox(
            workspace_root,
            mode,
            PolicyConfig::default(),
            sandbox_config,
        )
    }

    pub fn new_with_config(
        workspace_root: PathBuf,
        mode: PermissionMode,
        config: PolicyConfig,
    ) -> Self {
        Self::new_with_config_and_sandbox(
            workspace_root,
            mode,
            config,
            &LocalSandboxConfig::default(),
        )
    }

    pub fn new_with_config_and_sandbox(
        workspace_root: PathBuf,
        mode: PermissionMode,
        config: PolicyConfig,
        sandbox_config: &LocalSandboxConfig,
    ) -> Self {
        let readable_roots = sandbox_config.effective_readable_roots(&workspace_root);
        let writable_roots = sandbox_config.effective_writable_roots(&workspace_root);
        Self {
            workspace_root,
            readable_roots,
            writable_roots,
            unrestricted_file_access: mode == PermissionMode::FullAccess
                || sandbox_config.sandbox_mode == SandboxMode::DangerFullAccess,
            mode,
            config,
        }
    }

    fn inside_roots(&self, path: &Path, roots: &[PathBuf]) -> bool {
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
        roots.iter().any(|root| {
            let root = canonicalize_existing_ancestor(root);
            path_starts_with(&candidate, &root)
        })
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

fn path_starts_with(candidate: &Path, root: &Path) -> bool {
    #[cfg(windows)]
    {
        windows_comparison_path(candidate).starts_with(windows_comparison_path(root))
    }

    #[cfg(not(windows))]
    {
        candidate.starts_with(root)
    }
}

#[cfg(windows)]
fn windows_comparison_path(path: &Path) -> PathBuf {
    let value = path.to_string_lossy().replace('/', "\\");
    let value = if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = value.strip_prefix(r"\\?\") {
        rest.to_string()
    } else if let Some(rest) = value.strip_prefix(r"\??\") {
        rest.to_string()
    } else {
        value
    };
    PathBuf::from(value.to_lowercase())
}

impl PolicyEngine for BasicPolicyEngine {
    fn inspect_read(&self, path: &Path) -> PolicyDecision {
        if self.mode == PermissionMode::Chat {
            return PolicyDecision::Deny {
                reason: "Chat mode does not allow file access.".to_string(),
            };
        }

        if self.unrestricted_file_access || self.inside_roots(path, &self.readable_roots) {
            PolicyDecision::Allow
        } else {
            PolicyDecision::Ask {
                reason: format!("Reading outside authorized roots: {}", path.display()),
            }
        }
    }

    fn inspect_write(&self, path: &Path) -> PolicyDecision {
        match self.mode {
            PermissionMode::Chat | PermissionMode::ReadOnly => PolicyDecision::Deny {
                reason: "Current permission mode does not allow writes.".to_string(),
            },
            PermissionMode::FullAccess => PolicyDecision::Allow,
            PermissionMode::Auto | PermissionMode::Approve if self.unrestricted_file_access => {
                PolicyDecision::Allow
            }
            PermissionMode::Auto | PermissionMode::Approve
                if self.inside_roots(path, &self.writable_roots) =>
            {
                PolicyDecision::Allow
            }
            PermissionMode::Auto | PermissionMode::Approve => PolicyDecision::Ask {
                reason: format!("Writing outside authorized roots: {}", path.display()),
            },
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
            PermissionMode::Auto | PermissionMode::Approve | PermissionMode::FullAccess => {
                PolicyDecision::Allow
            }
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
        if host.eq_ignore_ascii_case("browser-interaction") && self.mode != PermissionMode::Chat {
            return PolicyDecision::Allow;
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

        match self.mode {
            PermissionMode::Chat => PolicyDecision::Deny {
                reason: "Chat mode does not allow network access.".to_string(),
            },
            PermissionMode::FullAccess => PolicyDecision::Allow,
            PermissionMode::ReadOnly | PermissionMode::Auto | PermissionMode::Approve => self
                .config
                .network
                .default_effect
                .to_decision(format!("Network access requires approval: {host}")),
        }
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
    use uuid::Uuid;

    #[test]
    fn execution_presets_separate_approval_policy_from_reviewer() {
        assert_eq!(
            PermissionMode::Approve.approval_policy(),
            ApprovalPolicy::OnRequest
        );
        assert_eq!(
            PermissionMode::Approve.approvals_reviewer(),
            ApprovalsReviewer::User
        );
        assert_eq!(
            PermissionMode::Auto.approval_policy(),
            ApprovalPolicy::OnRequest
        );
        assert_eq!(
            PermissionMode::Auto.approvals_reviewer(),
            ApprovalsReviewer::AutoReview
        );
        assert_eq!(
            PermissionMode::FullAccess.approval_policy(),
            ApprovalPolicy::Never
        );
    }

    #[test]
    fn auto_and_user_review_share_the_same_network_boundary() {
        let workspace = std::env::temp_dir().join(format!("opentopia-network-{}", Uuid::new_v4()));
        let auto = BasicPolicyEngine::new(workspace.clone(), PermissionMode::Auto);
        let user = BasicPolicyEngine::new(workspace, PermissionMode::Approve);
        assert!(matches!(
            auto.inspect_network("example.com"),
            PolicyDecision::Ask { .. }
        ));
        assert!(matches!(
            user.inspect_network("example.com"),
            PolicyDecision::Ask { .. }
        ));
    }

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

    #[test]
    fn approve_mode_allows_workspace_work_but_still_asks_for_external_risks() {
        let id = Uuid::new_v4();
        let workspace = std::env::temp_dir().join(format!("opentopia-policy-workspace-{id}"));
        let outside = std::env::temp_dir().join(format!("opentopia-policy-outside-{id}"));
        std::fs::create_dir_all(workspace.join("design")).expect("create workspace fixture");
        std::fs::create_dir_all(&outside).expect("create outside fixture");
        let policy = BasicPolicyEngine::new(workspace.clone(), PermissionMode::Approve);

        assert!(matches!(
            policy.inspect_write(&workspace.join("design/requirements.md")),
            PolicyDecision::Allow
        ));
        assert!(matches!(
            policy.inspect_write(&outside.join("requirements.md")),
            PolicyDecision::Ask { .. }
        ));
        assert!(matches!(
            policy.inspect_command("cargo test -p opentopia-core"),
            PolicyDecision::Allow
        ));
        assert!(matches!(
            policy.inspect_command("git reset --hard HEAD~1"),
            PolicyDecision::Ask { .. }
        ));
        assert!(matches!(
            policy.inspect_network("example.com"),
            PolicyDecision::Ask { .. }
        ));

        std::fs::remove_dir_all(workspace).expect("remove workspace fixture");
        std::fs::remove_dir_all(outside).expect("remove outside fixture");
    }

    #[test]
    fn configured_capability_roots_control_read_and_write_access() {
        let id = Uuid::new_v4();
        let workspace = std::env::temp_dir().join(format!("opentopia-policy-workspace-{id}"));
        let readable = std::env::temp_dir().join(format!("opentopia-policy-readable-{id}"));
        let writable = std::env::temp_dir().join(format!("opentopia-policy-writable-{id}"));
        let outside = std::env::temp_dir().join(format!("opentopia-policy-outside-{id}"));
        for root in [&workspace, &readable, &writable, &outside] {
            std::fs::create_dir_all(root).expect("create capability root fixture");
        }
        let mut sandbox = LocalSandboxConfig::default();
        sandbox.read_paths = vec![readable.clone()];
        sandbox.writable_roots = vec![writable.clone()];
        let policy = BasicPolicyEngine::new_with_sandbox_config(
            workspace.clone(),
            PermissionMode::Auto,
            &sandbox,
        );

        assert!(matches!(
            policy.inspect_read(&readable.join("reference.txt")),
            PolicyDecision::Allow
        ));
        assert!(matches!(
            policy.inspect_write(&readable.join("reference.txt")),
            PolicyDecision::Ask { .. }
        ));
        assert!(matches!(
            policy.inspect_read(&writable.join("dependency-cache.bin")),
            PolicyDecision::Allow
        ));
        assert!(matches!(
            policy.inspect_write(&writable.join("dependency-cache.bin")),
            PolicyDecision::Allow
        ));
        assert!(matches!(
            policy.inspect_read(&outside.join("unconfigured.txt")),
            PolicyDecision::Ask { .. }
        ));
        assert!(matches!(
            policy.inspect_write(&outside.join("unconfigured.txt")),
            PolicyDecision::Ask { .. }
        ));

        for root in [workspace, readable, writable, outside] {
            std::fs::remove_dir_all(root).expect("remove capability root fixture");
        }
    }

    #[test]
    fn full_access_modes_bypass_workspace_boundaries_but_not_chat_or_read_only_rules() {
        let outside = std::env::temp_dir().join(format!(
            "opentopia-policy-full-access-outside-{}",
            Uuid::new_v4()
        ));
        let workspace = PathBuf::from(".");

        let permission_full_access =
            BasicPolicyEngine::new(workspace.clone(), PermissionMode::FullAccess);
        assert!(matches!(
            permission_full_access.inspect_read(&outside),
            PolicyDecision::Allow
        ));
        assert!(matches!(
            permission_full_access.inspect_write(&outside),
            PolicyDecision::Allow
        ));

        let sandbox = LocalSandboxConfig::danger_full_access();
        let auto = BasicPolicyEngine::new_with_sandbox_config(
            workspace.clone(),
            PermissionMode::Auto,
            &sandbox,
        );
        assert!(matches!(auto.inspect_read(&outside), PolicyDecision::Allow));
        assert!(matches!(
            auto.inspect_write(&outside),
            PolicyDecision::Allow
        ));

        let chat = BasicPolicyEngine::new_with_sandbox_config(
            workspace.clone(),
            PermissionMode::Chat,
            &sandbox,
        );
        let read_only = BasicPolicyEngine::new_with_sandbox_config(
            workspace,
            PermissionMode::ReadOnly,
            &sandbox,
        );
        assert!(matches!(
            chat.inspect_read(&outside),
            PolicyDecision::Deny { .. }
        ));
        assert!(matches!(
            read_only.inspect_write(&outside),
            PolicyDecision::Deny { .. }
        ));
    }

    #[cfg(windows)]
    #[test]
    fn windows_workspace_comparison_accepts_verbatim_and_case_variants() {
        let workspace = Path::new(r"\\?\J:\Project\OneTree");
        let target = Path::new(r"j:\project\onetree\design\requirements.md");
        let sibling = Path::new(r"J:\Project\OneTree-copy\design\requirements.md");
        let unc_workspace = Path::new(r"\\?\UNC\server\share\OneTree");
        let unc_target = Path::new(r"\\server\share\onetree\design\requirements.md");

        assert!(path_starts_with(target, workspace));
        assert!(!path_starts_with(sibling, workspace));
        assert!(path_starts_with(unc_target, unc_workspace));
    }

    #[cfg(windows)]
    #[test]
    fn approve_mode_allows_missing_file_under_verbatim_workspace_root() {
        let workspace = std::env::temp_dir().join(format!(
            "opentopia-policy-verbatim-workspace-{}",
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(workspace.join("design")).expect("create workspace fixture");
        let verbatim_workspace = workspace.canonicalize().expect("canonical workspace");
        assert!(verbatim_workspace.to_string_lossy().starts_with(r"\\?\"));
        let policy = BasicPolicyEngine::new(verbatim_workspace.clone(), PermissionMode::Approve);

        assert!(matches!(
            policy.inspect_write(&verbatim_workspace.join("design/new-document.md")),
            PolicyDecision::Allow
        ));

        std::fs::remove_dir_all(workspace).expect("remove workspace fixture");
    }
}
