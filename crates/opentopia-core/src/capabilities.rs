use crate::bundled_plugins::BundledPluginTrust;
use crate::plugins::PluginSource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};
use thiserror::Error;

pub const OPENTOPIA_MANIFEST_API_VERSION: &str = "1";

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OpenTopiaManifest {
    pub api_version: String,
    #[serde(default)]
    pub requires: ManifestRequirements,
    #[serde(default)]
    pub permissions: PluginPermissions,
    #[serde(default)]
    pub contributes: ManifestContributions,
    #[serde(default)]
    pub configuration: Option<ManifestConfiguration>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ManifestRequirements {
    #[serde(default)]
    pub host_capabilities: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginPermissions {
    #[serde(default)]
    pub filesystem: Vec<String>,
    #[serde(default)]
    pub network: Vec<String>,
    #[serde(default)]
    pub secrets: Vec<String>,
    #[serde(default)]
    pub desktop: Vec<String>,
}

impl PluginPermissions {
    pub fn requirements(&self) -> Vec<PluginPermission> {
        let mut permissions = BTreeSet::new();
        for value in &self.filesystem {
            permissions.insert(PluginPermission::new(
                PluginPermissionKind::Filesystem,
                value,
            ));
        }
        for value in &self.network {
            permissions.insert(PluginPermission::new(PluginPermissionKind::Network, value));
        }
        for value in &self.secrets {
            permissions.insert(PluginPermission::new(PluginPermissionKind::Secret, value));
        }
        for value in &self.desktop {
            permissions.insert(PluginPermission::new(PluginPermissionKind::Desktop, value));
        }
        permissions.into_iter().collect()
    }

    fn normalize(&mut self) -> Result<(), CapabilityManifestError> {
        normalize_non_empty_values("permissions.filesystem", &mut self.filesystem)?;
        normalize_non_empty_values("permissions.network", &mut self.network)?;
        normalize_non_empty_values("permissions.secrets", &mut self.secrets)?;
        normalize_non_empty_values("permissions.desktop", &mut self.desktop)?;
        Ok(())
    }
}

#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(rename_all = "snake_case")]
pub enum PluginPermissionKind {
    Filesystem,
    Network,
    Secret,
    Desktop,
}

#[derive(
    Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(rename_all = "camelCase")]
pub struct PluginPermission {
    pub kind: PluginPermissionKind,
    pub value: String,
}

impl PluginPermission {
    pub fn new(kind: PluginPermissionKind, value: impl Into<String>) -> Self {
        Self {
            kind,
            value: value.into(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ManifestContributions {
    #[serde(default)]
    pub native_tools: Vec<Value>,
    #[serde(default)]
    pub previewers: Vec<Value>,
    #[serde(default)]
    pub context_loaders: Vec<Value>,
    #[serde(default)]
    pub agent_profiles: Vec<Value>,
    #[serde(default)]
    pub scm_connectors: Vec<Value>,
    #[serde(default)]
    pub apps: Vec<Value>,
    #[serde(flatten)]
    pub unsupported: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ManifestConfiguration {
    pub schema: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodexCompatibleContributions<'a> {
    pub skills: Option<&'a str>,
    pub mcp_servers: Option<&'a str>,
    pub apps: Option<&'a Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginCapabilityManifest {
    pub api_version: Option<String>,
    pub required_host_capabilities: Vec<String>,
    pub permissions: PluginPermissions,
    pub configuration_schema: Option<String>,
    pub contributions: Vec<PluginContribution>,
}

impl PluginCapabilityManifest {
    pub fn from_manifests(
        plugin_id: &str,
        opentopia: Option<OpenTopiaManifest>,
        compatible: CodexCompatibleContributions<'_>,
    ) -> Result<Self, CapabilityManifestError> {
        let mut normalized = Self::default();
        if let Some(mut manifest) = opentopia {
            if manifest.api_version != OPENTOPIA_MANIFEST_API_VERSION {
                return Err(CapabilityManifestError::UnsupportedApiVersion(
                    manifest.api_version,
                ));
            }
            normalize_non_empty_values(
                "requires.hostCapabilities",
                &mut manifest.requires.host_capabilities,
            )?;
            manifest.permissions.normalize()?;
            validate_contribution_namespaces(&manifest.contributes.unsupported)?;
            normalized.api_version = Some(manifest.api_version.clone());
            normalized.required_host_capabilities = manifest.requires.host_capabilities;
            normalized.permissions = manifest.permissions;
            normalized.configuration_schema = manifest
                .configuration
                .map(|configuration| normalize_relative_reference(&configuration.schema))
                .transpose()?;

            let contribution_defaults = PluginCapabilityManifest {
                contributions: Vec::new(),
                ..normalized.clone()
            };
            append_contributions(
                plugin_id,
                &mut normalized.contributions,
                ContributionKind::NativeTool,
                manifest.contributes.native_tools,
                &contribution_defaults,
            )?;
            append_contributions(
                plugin_id,
                &mut normalized.contributions,
                ContributionKind::Previewer,
                manifest.contributes.previewers,
                &contribution_defaults,
            )?;
            append_contributions(
                plugin_id,
                &mut normalized.contributions,
                ContributionKind::ContextLoader,
                manifest.contributes.context_loaders,
                &contribution_defaults,
            )?;
            append_contributions(
                plugin_id,
                &mut normalized.contributions,
                ContributionKind::AgentProfile,
                manifest.contributes.agent_profiles,
                &contribution_defaults,
            )?;
            append_contributions(
                plugin_id,
                &mut normalized.contributions,
                ContributionKind::ScmConnector,
                manifest.contributes.scm_connectors,
                &contribution_defaults,
            )?;
            append_contributions(
                plugin_id,
                &mut normalized.contributions,
                ContributionKind::App,
                manifest.contributes.apps,
                &contribution_defaults,
            )?;
        }

        let contribution_defaults = PluginCapabilityManifest {
            contributions: Vec::new(),
            ..normalized.clone()
        };

        if let Some(reference) = compatible.skills {
            append_compatible_contribution(
                plugin_id,
                &mut normalized.contributions,
                ContributionKind::Skill,
                "skills",
                reference,
                &contribution_defaults,
            )?;
        }
        if let Some(reference) = compatible.mcp_servers {
            append_compatible_contribution(
                plugin_id,
                &mut normalized.contributions,
                ContributionKind::McpServer,
                "mcp-servers",
                reference,
                &contribution_defaults,
            )?;
        }
        if normalized
            .contributions
            .iter()
            .all(|contribution| contribution.kind != ContributionKind::App)
        {
            if let Some(apps) = compatible.apps {
                append_legacy_apps(
                    plugin_id,
                    &mut normalized.contributions,
                    apps,
                    &contribution_defaults,
                )?;
            }
        }

        normalized
            .contributions
            .sort_by(|left, right| left.id.cmp(&right.id));
        let mut ids = BTreeSet::new();
        for contribution in &normalized.contributions {
            if !ids.insert(contribution.id.clone()) {
                return Err(CapabilityManifestError::DuplicateContributionId(
                    contribution.id.clone(),
                ));
            }
        }
        Ok(normalized)
    }
}

#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(rename_all = "snake_case")]
pub enum ContributionKind {
    Skill,
    McpServer,
    NativeTool,
    Previewer,
    ContextLoader,
    AgentProfile,
    ScmConnector,
    App,
}

#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(rename_all = "snake_case")]
pub enum ContributionOrigin {
    CodexCompatible,
    OpenTopia,
}

impl ContributionKind {
    fn manifest_name(self) -> &'static str {
        match self {
            Self::Skill => "skills",
            Self::McpServer => "mcpServers",
            Self::NativeTool => "nativeTools",
            Self::Previewer => "previewers",
            Self::ContextLoader => "contextLoaders",
            Self::AgentProfile => "agentProfiles",
            Self::ScmConnector => "scmConnectors",
            Self::App => "apps",
        }
    }

    fn has_exclusive_host_name(self) -> bool {
        matches!(
            self,
            Self::NativeTool | Self::AgentProfile | Self::ScmConnector | Self::App
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginContribution {
    pub id: String,
    pub plugin_id: String,
    pub local_id: String,
    pub kind: ContributionKind,
    pub origin: ContributionOrigin,
    pub api_version: String,
    pub required_host_capabilities: Vec<String>,
    pub permissions: Vec<PluginPermission>,
    pub configuration_schema: Option<String>,
    pub declaration: Value,
}

impl PluginContribution {
    pub fn stable_id(plugin_id: &str, local_id: &str) -> String {
        format!("{plugin_id}/{local_id}")
    }

    pub fn declared_path_reference(&self) -> Option<&str> {
        match &self.declaration {
            Value::String(reference) => Some(reference),
            Value::Object(object) => object
                .get("path")
                .or_else(|| {
                    (!object.contains_key("id"))
                        .then(|| object.get("source"))
                        .flatten()
                })
                .and_then(Value::as_str),
            _ => None,
        }
    }

    fn conflict_key(&self) -> Option<String> {
        self.kind
            .has_exclusive_host_name()
            .then(|| format!("{}:{}", self.kind.manifest_name(), self.local_id))
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CapabilityManifestError {
    #[error("unsupported opentopia manifest API version: {0}")]
    UnsupportedApiVersion(String),
    #[error("{0} contains an empty value")]
    EmptyValue(&'static str),
    #[error("invalid relative manifest reference: {0}")]
    InvalidReference(String),
    #[error("unsupported opentopia contribution namespace: {0}")]
    UnsupportedContribution(String),
    #[error("{kind} contribution must declare an id or a relative file reference")]
    MissingContributionId { kind: &'static str },
    #[error("invalid {kind} contribution id: {id}")]
    InvalidContributionId { kind: &'static str, id: String },
    #[error("duplicate contribution id: {0}")]
    DuplicateContributionId(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredPluginCapabilities {
    pub plugin_id: String,
    pub plugin_name: String,
    pub source: PluginSource,
    pub trust: BundledPluginTrust,
    pub default_enabled: bool,
    pub contributions: Vec<PluginContribution>,
}

#[derive(Debug, Clone, Default)]
pub struct CapabilityRegistry {
    plugins: BTreeMap<String, RegisteredPluginCapabilities>,
    contribution_ids: BTreeSet<String>,
}

impl CapabilityRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_plugin(
        &mut self,
        plugin: RegisteredPluginCapabilities,
    ) -> Result<(), CapabilityRegistryError> {
        if self.plugins.contains_key(&plugin.plugin_id) {
            return Err(CapabilityRegistryError::DuplicatePlugin(
                plugin.plugin_id.clone(),
            ));
        }
        let mut pending_ids = BTreeSet::new();
        for contribution in &plugin.contributions {
            if contribution.plugin_id != plugin.plugin_id {
                return Err(CapabilityRegistryError::ContributionOwnerMismatch {
                    contribution_id: contribution.id.clone(),
                    plugin_id: plugin.plugin_id.clone(),
                });
            }
            if contribution.id
                != PluginContribution::stable_id(&plugin.plugin_id, &contribution.local_id)
            {
                return Err(CapabilityRegistryError::UnstableContributionId(
                    contribution.id.clone(),
                ));
            }
            if self.contribution_ids.contains(&contribution.id)
                || !pending_ids.insert(contribution.id.clone())
            {
                return Err(CapabilityRegistryError::DuplicateContribution(
                    contribution.id.clone(),
                ));
            }
        }
        self.contribution_ids.extend(pending_ids);
        self.plugins.insert(plugin.plugin_id.clone(), plugin);
        Ok(())
    }

    pub fn plugins(&self) -> impl ExactSizeIterator<Item = &RegisteredPluginCapabilities> {
        self.plugins.values()
    }

    pub fn contribution(&self, contribution_id: &str) -> Option<&PluginContribution> {
        self.plugins
            .values()
            .flat_map(|plugin| plugin.contributions.iter())
            .find(|contribution| contribution.id == contribution_id)
    }

    pub fn activation_snapshot(
        &self,
        request: CapabilityActivationRequest,
    ) -> CapabilityActivationSnapshot {
        let available_host_capabilities = request
            .host_capabilities
            .into_iter()
            .collect::<BTreeSet<_>>();
        let activation_by_plugin = request
            .plugins
            .into_iter()
            .map(|activation| (activation.plugin_id.clone(), activation))
            .collect::<BTreeMap<_, _>>();
        let mut active_candidates = Vec::new();
        let mut unavailable = Vec::new();

        for plugin in self.plugins.values() {
            let activation = activation_by_plugin.get(&plugin.plugin_id);
            let enabled = activation
                .map(|activation| activation.is_enabled(plugin.default_enabled))
                .unwrap_or(plugin.default_enabled);
            let granted_permissions: BTreeSet<PluginPermission> = activation
                .map(|activation| activation.granted_permissions.iter().cloned().collect())
                .unwrap_or_default();
            for contribution in &plugin.contributions {
                let missing_host_capabilities = contribution
                    .required_host_capabilities
                    .iter()
                    .filter(|capability| !available_host_capabilities.contains(*capability))
                    .cloned()
                    .collect::<Vec<_>>();
                let missing_permissions = contribution
                    .permissions
                    .iter()
                    .filter(|permission| !granted_permissions.contains(*permission))
                    .cloned()
                    .collect::<Vec<_>>();
                let registered = ActivatedContribution {
                    plugin_name: plugin.plugin_name.clone(),
                    source: plugin.source,
                    trust: plugin.trust,
                    contribution: contribution.clone(),
                };
                if !enabled {
                    unavailable.push(UnavailableContribution {
                        contribution: registered,
                        reason: CapabilityUnavailableReason::Disabled,
                    });
                } else if contribution.kind == ContributionKind::NativeTool
                    && (plugin.source != PluginSource::Bundled
                        || plugin.trust == BundledPluginTrust::Standard)
                {
                    unavailable.push(UnavailableContribution {
                        contribution: registered,
                        reason: CapabilityUnavailableReason::HostTrustRequired,
                    });
                } else if !missing_host_capabilities.is_empty() {
                    unavailable.push(UnavailableContribution {
                        contribution: registered,
                        reason: CapabilityUnavailableReason::MissingHostCapabilities(
                            missing_host_capabilities,
                        ),
                    });
                } else if !missing_permissions.is_empty() {
                    unavailable.push(UnavailableContribution {
                        contribution: registered,
                        reason: CapabilityUnavailableReason::MissingPermissions(
                            missing_permissions,
                        ),
                    });
                } else {
                    active_candidates.push(registered);
                }
            }
        }

        let mut conflicts_by_key: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for contribution in &active_candidates {
            if let Some(key) = contribution.contribution.conflict_key() {
                conflicts_by_key
                    .entry(key)
                    .or_default()
                    .push(contribution.contribution.id.clone());
            }
        }
        let conflicts = conflicts_by_key
            .into_iter()
            .filter_map(|(key, mut contribution_ids)| {
                if contribution_ids.len() < 2 {
                    return None;
                }
                contribution_ids.sort();
                Some(CapabilityConflict {
                    key,
                    contribution_ids,
                })
            })
            .collect::<Vec<_>>();
        let conflicted_ids = conflicts
            .iter()
            .flat_map(|conflict| conflict.contribution_ids.iter().cloned())
            .collect::<BTreeSet<_>>();
        let mut active = Vec::new();
        for candidate in active_candidates {
            if conflicted_ids.contains(&candidate.contribution.id) {
                unavailable.push(UnavailableContribution {
                    contribution: candidate,
                    reason: CapabilityUnavailableReason::Conflict,
                });
            } else {
                active.push(candidate);
            }
        }
        active.sort_by(|left, right| left.contribution.id.cmp(&right.contribution.id));
        unavailable.sort_by(|left, right| {
            left.contribution
                .contribution
                .id
                .cmp(&right.contribution.contribution.id)
        });

        CapabilityActivationSnapshot {
            scope: request.scope,
            active,
            unavailable,
            conflicts,
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CapabilityRegistryError {
    #[error("plugin is already registered: {0}")]
    DuplicatePlugin(String),
    #[error("contribution is already registered: {0}")]
    DuplicateContribution(String),
    #[error("contribution {contribution_id} does not belong to plugin {plugin_id}")]
    ContributionOwnerMismatch {
        contribution_id: String,
        plugin_id: String,
    },
    #[error("contribution does not use its stable plugin/local id: {0}")]
    UnstableContributionId(String),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityActivationScope {
    pub workspace_id: Option<String>,
    pub thread_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginActivation {
    pub plugin_id: String,
    pub global_enabled: Option<bool>,
    pub workspace_enabled: Option<bool>,
    pub thread_enabled: Option<bool>,
    #[serde(default)]
    pub granted_permissions: Vec<PluginPermission>,
}

impl PluginActivation {
    pub fn is_enabled(&self, default_enabled: bool) -> bool {
        self.global_enabled.unwrap_or(default_enabled)
            && self.workspace_enabled.unwrap_or(true)
            && self.thread_enabled.unwrap_or(true)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityActivationRequest {
    pub scope: CapabilityActivationScope,
    #[serde(default)]
    pub host_capabilities: Vec<String>,
    #[serde(default)]
    pub plugins: Vec<PluginActivation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ActivatedContribution {
    pub plugin_name: String,
    pub source: PluginSource,
    pub trust: BundledPluginTrust,
    pub contribution: PluginContribution,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UnavailableContribution {
    pub contribution: ActivatedContribution,
    pub reason: CapabilityUnavailableReason,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityUnavailableReason {
    Disabled,
    HostTrustRequired,
    MissingHostCapabilities(Vec<String>),
    MissingPermissions(Vec<PluginPermission>),
    Conflict,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityConflict {
    pub key: String,
    pub contribution_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityActivationSnapshot {
    pub scope: CapabilityActivationScope,
    pub active: Vec<ActivatedContribution>,
    pub unavailable: Vec<UnavailableContribution>,
    pub conflicts: Vec<CapabilityConflict>,
}

fn append_contributions(
    plugin_id: &str,
    target: &mut Vec<PluginContribution>,
    kind: ContributionKind,
    declarations: Vec<Value>,
    manifest: &PluginCapabilityManifest,
) -> Result<(), CapabilityManifestError> {
    for declaration in declarations {
        let local_id = contribution_local_id(kind, &declaration)?;
        target.push(build_contribution(
            plugin_id,
            local_id,
            kind,
            ContributionOrigin::OpenTopia,
            declaration,
            manifest,
        ));
    }
    Ok(())
}

fn append_compatible_contribution(
    plugin_id: &str,
    target: &mut Vec<PluginContribution>,
    kind: ContributionKind,
    local_id: &str,
    reference: &str,
    manifest: &PluginCapabilityManifest,
) -> Result<(), CapabilityManifestError> {
    target.push(build_contribution(
        plugin_id,
        local_id.to_string(),
        kind,
        ContributionOrigin::CodexCompatible,
        serde_json::json!({ "path": reference }),
        manifest,
    ));
    Ok(())
}

fn append_legacy_apps(
    plugin_id: &str,
    target: &mut Vec<PluginContribution>,
    apps: &Value,
    manifest: &PluginCapabilityManifest,
) -> Result<(), CapabilityManifestError> {
    let declarations = match apps {
        Value::Array(values) => values.clone(),
        value => vec![value.clone()],
    };
    for (index, declaration) in declarations.into_iter().enumerate() {
        let local_id = contribution_local_id(ContributionKind::App, &declaration)
            .unwrap_or_else(|_| format!("apps-{}", index + 1));
        validate_local_id(ContributionKind::App, &local_id)?;
        target.push(build_contribution(
            plugin_id,
            local_id,
            ContributionKind::App,
            ContributionOrigin::CodexCompatible,
            declaration,
            manifest,
        ));
    }
    Ok(())
}

fn build_contribution(
    plugin_id: &str,
    local_id: String,
    kind: ContributionKind,
    origin: ContributionOrigin,
    declaration: Value,
    manifest: &PluginCapabilityManifest,
) -> PluginContribution {
    let api_version = declaration
        .get("apiVersion")
        .or_else(|| declaration.get("version"))
        .and_then(Value::as_str)
        .unwrap_or(OPENTOPIA_MANIFEST_API_VERSION)
        .to_string();
    PluginContribution {
        id: PluginContribution::stable_id(plugin_id, &local_id),
        plugin_id: plugin_id.to_string(),
        local_id,
        kind,
        origin,
        api_version,
        required_host_capabilities: manifest.required_host_capabilities.clone(),
        permissions: manifest.permissions.requirements(),
        configuration_schema: manifest.configuration_schema.clone(),
        declaration,
    }
}

fn contribution_local_id(
    kind: ContributionKind,
    declaration: &Value,
) -> Result<String, CapabilityManifestError> {
    let id = match declaration {
        Value::String(reference) => local_id_from_reference(reference)?,
        Value::Object(object) => {
            if let Some(id) = object.get("id").and_then(Value::as_str) {
                id.to_string()
            } else if let Some(reference) = object
                .get("path")
                .or_else(|| object.get("source"))
                .and_then(Value::as_str)
            {
                local_id_from_reference(reference)?
            } else {
                return Err(CapabilityManifestError::MissingContributionId {
                    kind: kind.manifest_name(),
                });
            }
        }
        _ => {
            return Err(CapabilityManifestError::MissingContributionId {
                kind: kind.manifest_name(),
            })
        }
    };
    validate_local_id(kind, &id)?;
    Ok(id)
}

fn local_id_from_reference(reference: &str) -> Result<String, CapabilityManifestError> {
    let normalized = normalize_relative_reference(reference)?;
    Path::new(&normalized)
        .file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .ok_or(CapabilityManifestError::InvalidReference(normalized))
}

fn validate_local_id(kind: ContributionKind, id: &str) -> Result<(), CapabilityManifestError> {
    if id.is_empty()
        || id.len() > 128
        || !id.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || character == '-'
                || character == '_'
                || character == '.'
        })
    {
        return Err(CapabilityManifestError::InvalidContributionId {
            kind: kind.manifest_name(),
            id: id.to_string(),
        });
    }
    Ok(())
}

fn normalize_relative_reference(reference: &str) -> Result<String, CapabilityManifestError> {
    let path = Path::new(reference);
    if reference.trim().is_empty()
        || path.is_absolute()
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err(CapabilityManifestError::InvalidReference(
            reference.to_string(),
        ));
    }
    Ok(path
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/"))
}

fn normalize_non_empty_values(
    field: &'static str,
    values: &mut Vec<String>,
) -> Result<(), CapabilityManifestError> {
    if values.iter().any(|value| value.trim().is_empty()) {
        return Err(CapabilityManifestError::EmptyValue(field));
    }
    values.sort();
    values.dedup();
    Ok(())
}

fn validate_contribution_namespaces(
    unsupported: &BTreeMap<String, Value>,
) -> Result<(), CapabilityManifestError> {
    if let Some(name) = unsupported
        .iter()
        .find(|(_, value)| !value.is_null())
        .map(|(name, _)| name)
    {
        return Err(CapabilityManifestError::UnsupportedContribution(
            name.clone(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_with_native_tool(plugin_id: &str, local_id: &str) -> PluginCapabilityManifest {
        PluginCapabilityManifest::from_manifests(
            plugin_id,
            Some(OpenTopiaManifest {
                api_version: "1".to_string(),
                requires: ManifestRequirements {
                    host_capabilities: vec!["nativeTool.test.v1".to_string()],
                },
                permissions: PluginPermissions {
                    filesystem: vec!["workspace:read".to_string()],
                    ..PluginPermissions::default()
                },
                contributes: ManifestContributions {
                    native_tools: vec![serde_json::json!({ "id": local_id })],
                    ..ManifestContributions::default()
                },
                configuration: Some(ManifestConfiguration {
                    schema: "./configuration.schema.json".to_string(),
                }),
            }),
            CodexCompatibleContributions::default(),
        )
        .unwrap()
    }

    fn registration(
        plugin_id: &str,
        plugin_name: &str,
        manifest: PluginCapabilityManifest,
    ) -> RegisteredPluginCapabilities {
        RegisteredPluginCapabilities {
            plugin_id: plugin_id.to_string(),
            plugin_name: plugin_name.to_string(),
            source: PluginSource::Bundled,
            trust: BundledPluginTrust::Official,
            default_enabled: true,
            contributions: manifest.contributions,
        }
    }

    #[test]
    fn normalizes_v1_and_codex_compatible_contributions() {
        let manifest = PluginCapabilityManifest::from_manifests(
            "user:example",
            Some(OpenTopiaManifest {
                api_version: "1".to_string(),
                requires: ManifestRequirements {
                    host_capabilities: vec!["workspace.files.v1".to_string()],
                },
                permissions: PluginPermissions {
                    filesystem: vec!["workspace:read".to_string()],
                    ..PluginPermissions::default()
                },
                contributes: ManifestContributions {
                    previewers: vec![serde_json::json!({
                        "id": "xlsx",
                        "mediaTypes": ["application/vnd.test"]
                    })],
                    agent_profiles: vec![Value::String("./agents/reviewer.toml".to_string())],
                    ..ManifestContributions::default()
                },
                configuration: Some(ManifestConfiguration {
                    schema: "./configuration.schema.json".to_string(),
                }),
            }),
            CodexCompatibleContributions {
                skills: Some("./skills"),
                mcp_servers: Some("./.mcp.json"),
                apps: None,
            },
        )
        .unwrap();

        assert_eq!(manifest.api_version.as_deref(), Some("1"));
        assert_eq!(manifest.contributions.len(), 4);
        assert!(manifest
            .contributions
            .iter()
            .any(|contribution| contribution.id == "user:example/reviewer"));
        let previewer = manifest
            .contributions
            .iter()
            .find(|contribution| contribution.kind == ContributionKind::Previewer)
            .unwrap();
        assert_eq!(
            previewer.configuration_schema.as_deref(),
            Some("configuration.schema.json")
        );
        assert_eq!(previewer.permissions.len(), 1);
    }

    #[test]
    fn rejects_unknown_versions_driver_namespaces_and_duplicate_local_ids() {
        let mut manifest = OpenTopiaManifest {
            api_version: "2".to_string(),
            ..OpenTopiaManifest::default()
        };
        assert!(matches!(
            PluginCapabilityManifest::from_manifests(
                "user:test",
                Some(manifest.clone()),
                CodexCompatibleContributions::default()
            ),
            Err(CapabilityManifestError::UnsupportedApiVersion(_))
        ));

        manifest.api_version = "1".to_string();
        manifest.contributes.unsupported.insert(
            "providerDrivers".to_string(),
            serde_json::json!([{ "id": "unsafe" }]),
        );
        assert!(matches!(
            PluginCapabilityManifest::from_manifests(
                "user:test",
                Some(manifest),
                CodexCompatibleContributions::default()
            ),
            Err(CapabilityManifestError::UnsupportedContribution(_))
        ));

        let duplicate = OpenTopiaManifest {
            api_version: "1".to_string(),
            contributes: ManifestContributions {
                native_tools: vec![
                    serde_json::json!({ "id": "same" }),
                    serde_json::json!({ "id": "same" }),
                ],
                ..ManifestContributions::default()
            },
            ..OpenTopiaManifest::default()
        };
        assert!(matches!(
            PluginCapabilityManifest::from_manifests(
                "user:test",
                Some(duplicate),
                CodexCompatibleContributions::default()
            ),
            Err(CapabilityManifestError::DuplicateContributionId(_))
        ));
    }

    #[test]
    fn activation_snapshot_enforces_scope_capabilities_permissions_and_conflicts() {
        let first_manifest = manifest_with_native_tool("user:first", "shared");
        let second_manifest = manifest_with_native_tool("user:second", "shared");
        let mut registry = CapabilityRegistry::new();
        registry
            .register_plugin(registration("user:first", "first", first_manifest))
            .unwrap();
        registry
            .register_plugin(registration("user:second", "second", second_manifest))
            .unwrap();

        let permission = PluginPermission::new(PluginPermissionKind::Filesystem, "workspace:read");
        let snapshot = registry.activation_snapshot(CapabilityActivationRequest {
            scope: CapabilityActivationScope {
                workspace_id: Some("workspace-1".to_string()),
                thread_id: Some("thread-1".to_string()),
            },
            host_capabilities: vec!["nativeTool.test.v1".to_string()],
            plugins: vec![
                PluginActivation {
                    plugin_id: "user:first".to_string(),
                    granted_permissions: vec![permission.clone()],
                    ..PluginActivation::default()
                },
                PluginActivation {
                    plugin_id: "user:second".to_string(),
                    granted_permissions: vec![permission],
                    ..PluginActivation::default()
                },
            ],
        });

        assert!(snapshot.active.is_empty());
        assert_eq!(snapshot.conflicts.len(), 1);
        assert_eq!(snapshot.unavailable.len(), 2);

        let snapshot = registry.activation_snapshot(CapabilityActivationRequest {
            host_capabilities: vec!["nativeTool.test.v1".to_string()],
            plugins: vec![
                PluginActivation {
                    plugin_id: "user:first".to_string(),
                    workspace_enabled: Some(false),
                    granted_permissions: vec![PluginPermission::new(
                        PluginPermissionKind::Filesystem,
                        "workspace:read",
                    )],
                    ..PluginActivation::default()
                },
                PluginActivation {
                    plugin_id: "user:second".to_string(),
                    granted_permissions: vec![],
                    ..PluginActivation::default()
                },
            ],
            ..CapabilityActivationRequest::default()
        });
        assert!(snapshot.active.is_empty());
        assert!(snapshot.conflicts.is_empty());
        assert!(snapshot
            .unavailable
            .iter()
            .any(|item| matches!(item.reason, CapabilityUnavailableReason::Disabled)));
        assert!(snapshot.unavailable.iter().any(|item| matches!(
            item.reason,
            CapabilityUnavailableReason::MissingPermissions(_)
        )));
    }

    #[test]
    fn standard_plugins_cannot_activate_host_native_tools() {
        let manifest = manifest_with_native_tool("user:untrusted", "host-tool");
        let mut plugin = registration("user:untrusted", "untrusted", manifest);
        plugin.source = PluginSource::User;
        plugin.trust = BundledPluginTrust::Standard;
        let mut registry = CapabilityRegistry::new();
        registry.register_plugin(plugin).unwrap();

        let snapshot = registry.activation_snapshot(CapabilityActivationRequest {
            host_capabilities: vec!["nativeTool.test.v1".to_string()],
            plugins: vec![PluginActivation {
                plugin_id: "user:untrusted".to_string(),
                granted_permissions: vec![PluginPermission::new(
                    PluginPermissionKind::Filesystem,
                    "workspace:read",
                )],
                ..PluginActivation::default()
            }],
            ..CapabilityActivationRequest::default()
        });

        assert!(snapshot.active.is_empty());
        assert!(matches!(
            snapshot.unavailable[0].reason,
            CapabilityUnavailableReason::HostTrustRequired
        ));
    }
}
