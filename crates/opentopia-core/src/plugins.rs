use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

const MANIFEST_RELATIVE_PATH: &str = ".codex-plugin/plugin.json";
const MAX_DISCOVERY_DEPTH: usize = 7;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_MCP_CONFIG_BYTES: u64 = 1024 * 1024;
const MAX_PLUGIN_FILES: usize = 10_000;
const MAX_PLUGIN_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginScope {
    Workspace,
    User,
    Codex,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginDescriptor {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub version: String,
    pub description: String,
    pub long_description: String,
    pub author: String,
    pub category: String,
    pub path: PathBuf,
    pub manifest_path: PathBuf,
    pub scope: PluginScope,
    pub managed: bool,
    pub skill_root: Option<PathBuf>,
    pub skill_count: usize,
    pub mcp_server_count: usize,
    pub supported_mcp_server_count: usize,
    pub has_apps: bool,
    pub capabilities: Vec<String>,
    pub brand_color: Option<String>,
    pub website_url: Option<String>,
    pub issues: Vec<String>,
}

impl PluginDescriptor {
    pub fn is_compatible(&self) -> bool {
        self.skill_count > 0 || self.supported_mcp_server_count > 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginMcpServerDefinition {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env_keys: Vec<String>,
    pub timeout_ms: u64,
}

#[derive(Debug, Error)]
pub enum PluginError {
    #[error("plugin path does not exist or is not a directory: {0}")]
    InvalidSource(String),
    #[error("plugin manifest was not found under {0}")]
    ManifestNotFound(String),
    #[error("multiple plugin manifests were found under {0}; select one plugin folder")]
    MultipleManifests(String),
    #[error("plugin manifest is too large: {0}")]
    ManifestTooLarge(String),
    #[error("plugin manifest is invalid: {0}")]
    InvalidManifest(String),
    #[error("plugin path escapes its package root: {0}")]
    PathEscape(String),
    #[error("plugin contains a symbolic link, which is not allowed during installation: {0}")]
    SymbolicLink(String),
    #[error(
        "plugin exceeds the installation limit of {maximum_files} files or {maximum_bytes} bytes"
    )]
    InstallLimit {
        maximum_files: usize,
        maximum_bytes: u64,
    },
    #[error("plugin is already installed: {0}")]
    AlreadyInstalled(String),
    #[error("plugin is not managed by OpenTopia and cannot be removed: {0}")]
    NotManaged(String),
    #[error("plugin is not installed: {0}")]
    NotFound(String),
    #[error("plugin I/O failed: {0}")]
    Io(String),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginManifest {
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    author: Option<PluginAuthor>,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    skills: Option<String>,
    #[serde(default)]
    apps: Option<Value>,
    #[serde(default)]
    mcp_servers: Option<String>,
    #[serde(default)]
    interface: PluginInterface,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PluginAuthor {
    Name(String),
    Details { name: String },
}

impl PluginAuthor {
    fn name(self) -> String {
        match self {
            Self::Name(name) | Self::Details { name } => name,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginInterface {
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    short_description: String,
    #[serde(default)]
    long_description: String,
    #[serde(default)]
    developer_name: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    website_url: Option<String>,
    #[serde(default)]
    brand_color: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginMcpFile {
    #[serde(default)]
    mcp_servers: HashMap<String, Value>,
}

pub fn discover_plugins(workspace_root: Option<&Path>) -> Vec<PluginDescriptor> {
    let mut roots = Vec::new();
    if let Some(workspace_root) = workspace_root {
        roots.push((
            workspace_root.join(".opentopia/plugins"),
            PluginScope::Workspace,
        ));
        roots.push((
            workspace_root.join(".agents/plugins"),
            PluginScope::Workspace,
        ));
        roots.push((
            workspace_root.join(".codex/plugins"),
            PluginScope::Workspace,
        ));
    }
    roots.push((user_plugin_root(), PluginScope::User));
    if let Some(codex_home) = codex_home() {
        roots.push((codex_home.join("plugins/cache"), PluginScope::Codex));
    }

    let managed_root = user_plugin_root().canonicalize().ok();
    let mut manifests = Vec::new();
    for (root, scope) in roots {
        collect_manifests(&root, scope, 0, &mut manifests);
    }

    let mut seen = HashSet::new();
    let mut plugins = manifests
        .into_iter()
        .filter_map(|(manifest_path, scope)| {
            let canonical = manifest_path.canonicalize().ok()?;
            if !seen.insert(canonical.clone()) {
                return None;
            }
            descriptor_from_manifest(&canonical, scope, managed_root.as_deref()).ok()
        })
        .collect::<Vec<_>>();
    plugins.sort_by(|left, right| {
        scope_rank(left.scope)
            .cmp(&scope_rank(right.scope))
            .then_with(|| {
                left.display_name
                    .to_lowercase()
                    .cmp(&right.display_name.to_lowercase())
            })
            .then_with(|| left.id.cmp(&right.id))
    });
    plugins
}

pub fn inspect_plugin(source: &Path) -> Result<PluginDescriptor, PluginError> {
    let manifest = locate_source_manifest(source)?;
    descriptor_from_manifest(&manifest, PluginScope::User, None)
}

pub fn install_plugin(source: &Path) -> Result<PluginDescriptor, PluginError> {
    let source_manifest = locate_source_manifest(source)?;
    let source_root = plugin_root_from_manifest(&source_manifest)?;
    let source_descriptor = descriptor_from_manifest(&source_manifest, PluginScope::User, None)?;
    let destination_root = user_plugin_root();
    fs::create_dir_all(&destination_root).map_err(io_error)?;
    let destination_root = destination_root.canonicalize().map_err(io_error)?;
    let destination = destination_root.join(safe_directory_name(&source_descriptor.name)?);
    if destination.exists() {
        return Err(PluginError::AlreadyInstalled(
            source_descriptor.display_name,
        ));
    }

    let staging = destination_root.join(format!(
        ".installing-{}-{}",
        safe_directory_name(&source_descriptor.name)?,
        Uuid::new_v4()
    ));
    let install_result = (|| {
        fs::create_dir(&staging).map_err(io_error)?;
        let mut budget = CopyBudget::default();
        copy_plugin_tree(&source_root, &staging, &mut budget)?;
        let staged_manifest = staging.join(MANIFEST_RELATIVE_PATH);
        descriptor_from_manifest(&staged_manifest, PluginScope::User, Some(&destination_root))?;
        fs::rename(&staging, &destination).map_err(io_error)?;
        descriptor_from_manifest(
            &destination.join(MANIFEST_RELATIVE_PATH),
            PluginScope::User,
            Some(&destination_root),
        )
    })();
    if install_result.is_err() && staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    install_result
}

pub fn uninstall_plugin(
    plugin_id: &str,
    workspace_root: Option<&Path>,
) -> Result<PluginDescriptor, PluginError> {
    let plugin = discover_plugins(workspace_root)
        .into_iter()
        .find(|plugin| plugin.id == plugin_id)
        .ok_or_else(|| PluginError::NotFound(plugin_id.to_string()))?;
    if !plugin.managed {
        return Err(PluginError::NotManaged(plugin.display_name));
    }
    let managed_root = user_plugin_root().canonicalize().map_err(io_error)?;
    let plugin_path = plugin.path.canonicalize().map_err(io_error)?;
    if plugin_path.parent() != Some(managed_root.as_path()) {
        return Err(PluginError::NotManaged(plugin.path.display().to_string()));
    }
    fs::remove_dir_all(&plugin_path).map_err(io_error)?;
    Ok(plugin)
}

pub fn load_plugin_mcp_servers(
    plugin: &PluginDescriptor,
) -> Result<Vec<PluginMcpServerDefinition>, PluginError> {
    let manifest = read_manifest(&plugin.manifest_path)?;
    let Some(relative_path) = manifest.mcp_servers else {
        return Ok(Vec::new());
    };
    let path = resolve_declared_path(&plugin.path, &relative_path, false)?;
    let metadata = fs::metadata(&path).map_err(io_error)?;
    if metadata.len() > MAX_MCP_CONFIG_BYTES {
        return Err(PluginError::InvalidManifest(format!(
            "MCP configuration is too large: {}",
            path.display()
        )));
    }
    let raw = fs::read(&path).map_err(io_error)?;
    let config: PluginMcpFile = serde_json::from_slice(&raw)
        .map_err(|error| PluginError::InvalidManifest(format!("{}: {error}", path.display())))?;
    let mut definitions = Vec::new();
    let mut entries = config.mcp_servers.into_iter().collect::<Vec<_>>();
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    for (name, value) in entries {
        if let Some(definition) = parse_stdio_mcp_server(plugin, name, &value)? {
            definitions.push(definition);
        }
    }
    Ok(definitions)
}

fn descriptor_from_manifest(
    manifest_path: &Path,
    scope: PluginScope,
    managed_root: Option<&Path>,
) -> Result<PluginDescriptor, PluginError> {
    let manifest_path = manifest_path.canonicalize().map_err(io_error)?;
    let plugin_root = plugin_root_from_manifest(&manifest_path)?;
    let manifest = read_manifest(&manifest_path)?;
    validate_plugin_name(&manifest.name)?;

    let managed = managed_root
        .and_then(|root| root.canonicalize().ok())
        .is_some_and(|root| plugin_root.parent() == Some(root.as_path()));
    let mut issues = Vec::new();
    let skill_root = match manifest.skills.as_deref() {
        Some(path) => match resolve_declared_path(&plugin_root, path, true) {
            Ok(path) => Some(path),
            Err(error) => {
                issues.push(error.to_string());
                None
            }
        },
        None => None,
    };
    let skill_count = skill_root
        .as_deref()
        .map(|root| count_named_files(root, "SKILL.md", 0))
        .unwrap_or_default();
    if manifest.skills.is_some() && skill_count == 0 {
        issues.push("The declared Skills directory contains no SKILL.md files.".to_string());
    }

    let (mcp_server_count, supported_mcp_server_count, mcp_issues) =
        inspect_mcp_capability(&plugin_root, manifest.mcp_servers.as_deref());
    issues.extend(mcp_issues);
    let has_apps = manifest.apps.is_some();
    if has_apps {
        issues.push("Plugin app bridges are detected but not supported yet.".to_string());
    }
    if skill_count == 0 && mcp_server_count == 0 && !has_apps {
        issues.push("The plugin does not declare Skills, MCP servers, or apps.".to_string());
    }

    let interface = manifest.interface;
    let display_name = non_empty(interface.display_name, &manifest.name);
    let description = non_empty(interface.short_description, &manifest.description);
    let long_description = non_empty(interface.long_description, &description);
    let manifest_author = manifest.author.map(PluginAuthor::name).unwrap_or_default();
    let author = non_empty(interface.developer_name, &manifest_author);
    let website_url = interface.website_url.or(manifest.homepage);
    let id = plugin_id(scope, &manifest_path);

    Ok(PluginDescriptor {
        id,
        name: manifest.name,
        display_name,
        version: manifest.version,
        description,
        long_description,
        author,
        category: non_empty(interface.category, "Other"),
        path: plugin_root,
        manifest_path,
        scope,
        managed,
        skill_root,
        skill_count,
        mcp_server_count,
        supported_mcp_server_count,
        has_apps,
        capabilities: interface.capabilities,
        brand_color: interface
            .brand_color
            .filter(|color| valid_brand_color(color)),
        website_url,
        issues,
    })
}

fn read_manifest(path: &Path) -> Result<PluginManifest, PluginError> {
    let metadata = fs::metadata(path).map_err(io_error)?;
    if metadata.len() > MAX_MANIFEST_BYTES {
        return Err(PluginError::ManifestTooLarge(path.display().to_string()));
    }
    let bytes = fs::read(path).map_err(io_error)?;
    serde_json::from_slice(&bytes)
        .map_err(|error| PluginError::InvalidManifest(format!("{}: {error}", path.display())))
}

fn inspect_mcp_capability(
    plugin_root: &Path,
    declared_path: Option<&str>,
) -> (usize, usize, Vec<String>) {
    let Some(declared_path) = declared_path else {
        return (0, 0, Vec::new());
    };
    let result = (|| {
        let path = resolve_declared_path(plugin_root, declared_path, false)?;
        let metadata = fs::metadata(&path).map_err(io_error)?;
        if metadata.len() > MAX_MCP_CONFIG_BYTES {
            return Err(PluginError::InvalidManifest(format!(
                "MCP configuration is too large: {}",
                path.display()
            )));
        }
        let config: PluginMcpFile = serde_json::from_slice(&fs::read(&path).map_err(io_error)?)
            .map_err(|error| {
                PluginError::InvalidManifest(format!("{}: {error}", path.display()))
            })?;
        let total = config.mcp_servers.len();
        let supported = config
            .mcp_servers
            .values()
            .filter(|value| mcp_transport(value) == "stdio" && mcp_command(value).is_some())
            .count();
        Ok::<_, PluginError>((total, supported))
    })();
    match result {
        Ok((total, supported)) => {
            let mut issues = Vec::new();
            if supported < total {
                issues.push(format!(
                    "{} MCP server(s) require HTTP/OAuth or have no stdio command; this runtime currently supports stdio only.",
                    total - supported
                ));
            }
            (total, supported, issues)
        }
        Err(error) => (0, 0, vec![error.to_string()]),
    }
}

fn parse_stdio_mcp_server(
    plugin: &PluginDescriptor,
    name: String,
    value: &Value,
) -> Result<Option<PluginMcpServerDefinition>, PluginError> {
    if mcp_transport(value) != "stdio" {
        return Ok(None);
    }
    let Some(command) = mcp_command(value) else {
        return Ok(None);
    };
    let args = value
        .get("args")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let cwd = match value.get("cwd").and_then(Value::as_str) {
        Some(cwd) => resolve_declared_path(&plugin.path, cwd, true)?,
        None => plugin.path.clone(),
    };
    let command = resolve_mcp_command(&plugin.path, command);
    let mut env_keys = value
        .get("envKeys")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .filter(|key| valid_env_key(key))
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if let Some(environment) = value.get("env").and_then(Value::as_object) {
        for (key, source) in environment {
            if valid_env_key(key) && env_value_references_key(source.as_str().unwrap_or(""), key) {
                env_keys.push(key.clone());
            }
        }
    }
    env_keys.sort();
    env_keys.dedup();
    let timeout_ms = value
        .get("timeoutMs")
        .and_then(Value::as_u64)
        .unwrap_or(30_000)
        .clamp(1_000, 300_000);
    Ok(Some(PluginMcpServerDefinition {
        name,
        command,
        args,
        cwd,
        env_keys,
        timeout_ms,
    }))
}

fn mcp_transport(value: &Value) -> &str {
    value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            if value.get("command").is_some() {
                "stdio"
            } else {
                "unknown"
            }
        })
}

fn mcp_command(value: &Value) -> Option<&str> {
    value
        .get("command")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

fn resolve_mcp_command(plugin_root: &Path, command: &str) -> String {
    let path = Path::new(command);
    if (command.starts_with("./") || command.starts_with(".\\")) && path_is_relative_safe(path) {
        return plugin_root.join(path).display().to_string();
    }
    command.to_string()
}

fn env_value_references_key(value: &str, key: &str) -> bool {
    value == format!("${{{key}}}") || value == format!("${key}") || value == format!("%{key}%")
}

fn valid_env_key(key: &str) -> bool {
    !key.is_empty()
        && key.chars().all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        })
}

fn locate_source_manifest(source: &Path) -> Result<PathBuf, PluginError> {
    if !source.exists() {
        return Err(PluginError::InvalidSource(source.display().to_string()));
    }
    let source = source.canonicalize().map_err(io_error)?;
    if source.is_file() {
        if source.file_name().and_then(|name| name.to_str()) == Some("plugin.json")
            && source
                .parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                == Some(".codex-plugin")
        {
            return Ok(source);
        }
        return Err(PluginError::InvalidSource(source.display().to_string()));
    }
    let direct = source.join(MANIFEST_RELATIVE_PATH);
    if direct.is_file() {
        return direct.canonicalize().map_err(io_error);
    }
    if source.file_name().and_then(|name| name.to_str()) == Some(".codex-plugin") {
        let manifest = source.join("plugin.json");
        if manifest.is_file() {
            return manifest.canonicalize().map_err(io_error);
        }
    }
    let mut found = Vec::new();
    collect_manifests(&source, PluginScope::User, 0, &mut found);
    match found.len() {
        0 => Err(PluginError::ManifestNotFound(source.display().to_string())),
        1 => found[0].0.canonicalize().map_err(io_error),
        _ => Err(PluginError::MultipleManifests(source.display().to_string())),
    }
}

fn plugin_root_from_manifest(manifest_path: &Path) -> Result<PathBuf, PluginError> {
    let metadata_dir = manifest_path
        .parent()
        .ok_or_else(|| PluginError::InvalidManifest(manifest_path.display().to_string()))?;
    if metadata_dir.file_name().and_then(|name| name.to_str()) != Some(".codex-plugin") {
        return Err(PluginError::InvalidManifest(format!(
            "manifest must be located at {MANIFEST_RELATIVE_PATH}"
        )));
    }
    metadata_dir
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| PluginError::InvalidManifest(manifest_path.display().to_string()))
}

fn collect_manifests(
    directory: &Path,
    scope: PluginScope,
    depth: usize,
    output: &mut Vec<(PathBuf, PluginScope)>,
) {
    if depth > MAX_DISCOVERY_DEPTH || !directory.is_dir() {
        return;
    }
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => continue,
        };
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_file()
            && entry.file_name().eq_ignore_ascii_case("plugin.json")
            && path
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|name| name.eq_ignore_ascii_case(".codex-plugin"))
        {
            output.push((path, scope));
            continue;
        }
        if file_type.is_dir() && !ignored_directory(&entry.file_name().to_string_lossy()) {
            collect_manifests(&path, scope, depth + 1, output);
        }
    }
}

fn ignored_directory(name: &str) -> bool {
    matches!(name, ".git" | "node_modules" | "target" | "dist" | "build")
}

fn resolve_declared_path(
    plugin_root: &Path,
    declared: &str,
    require_directory: bool,
) -> Result<PathBuf, PluginError> {
    let relative = Path::new(declared);
    if !path_is_relative_safe(relative) {
        return Err(PluginError::PathEscape(declared.to_string()));
    }
    let path = plugin_root.join(relative);
    let canonical = path.canonicalize().map_err(io_error)?;
    let root = plugin_root.canonicalize().map_err(io_error)?;
    if !canonical.starts_with(&root) || (require_directory && !canonical.is_dir()) {
        return Err(PluginError::PathEscape(declared.to_string()));
    }
    Ok(canonical)
}

fn path_is_relative_safe(path: &Path) -> bool {
    !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

fn count_named_files(directory: &Path, name: &str, depth: usize) -> usize {
    if depth > MAX_DISCOVERY_DEPTH || !directory.is_dir() {
        return 0;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(file_type) if file_type.is_symlink() => 0,
            Ok(file_type) if file_type.is_dir() => {
                count_named_files(&entry.path(), name, depth + 1)
            }
            Ok(file_type)
                if file_type.is_file() && entry.file_name().eq_ignore_ascii_case(name) =>
            {
                1
            }
            _ => 0,
        })
        .sum()
}

#[derive(Default)]
struct CopyBudget {
    files: usize,
    bytes: u64,
}

fn copy_plugin_tree(
    source: &Path,
    destination: &Path,
    budget: &mut CopyBudget,
) -> Result<(), PluginError> {
    for entry in fs::read_dir(source).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let file_type = entry.file_type().map_err(io_error)?;
        if file_type.is_symlink() {
            return Err(PluginError::SymbolicLink(
                entry.path().display().to_string(),
            ));
        }
        let target = destination.join(entry.file_name());
        if file_type.is_dir() {
            fs::create_dir(&target).map_err(io_error)?;
            copy_plugin_tree(&entry.path(), &target, budget)?;
        } else if file_type.is_file() {
            let bytes = entry.metadata().map_err(io_error)?.len();
            budget.files = budget.files.saturating_add(1);
            budget.bytes = budget.bytes.saturating_add(bytes);
            if budget.files > MAX_PLUGIN_FILES || budget.bytes > MAX_PLUGIN_BYTES {
                return Err(PluginError::InstallLimit {
                    maximum_files: MAX_PLUGIN_FILES,
                    maximum_bytes: MAX_PLUGIN_BYTES,
                });
            }
            fs::copy(entry.path(), target).map_err(io_error)?;
        }
    }
    Ok(())
}

fn safe_directory_name(name: &str) -> Result<String, PluginError> {
    validate_plugin_name(name)?;
    Ok(name.to_string())
}

fn validate_plugin_name(name: &str) -> Result<(), PluginError> {
    if name.is_empty()
        || name.len() > 80
        || !name.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || character == '-'
                || character == '_'
        })
    {
        return Err(PluginError::InvalidManifest(
            "plugin name must use lowercase ASCII letters, digits, '-' or '_'".to_string(),
        ));
    }
    Ok(())
}

fn valid_brand_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value[1..]
            .chars()
            .all(|character| character.is_ascii_hexdigit())
}

fn non_empty(value: String, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value
    }
}

fn plugin_id(scope: PluginScope, manifest_path: &Path) -> String {
    format!(
        "{}:{}",
        match scope {
            PluginScope::Workspace => "workspace",
            PluginScope::User => "user",
            PluginScope::Codex => "codex",
        },
        manifest_path.to_string_lossy().replace('\\', "/")
    )
}

fn scope_rank(scope: PluginScope) -> u8 {
    match scope {
        PluginScope::Workspace => 0,
        PluginScope::User => 1,
        PluginScope::Codex => 2,
    }
}

fn user_plugin_root() -> PathBuf {
    std::env::var_os("OPENTOPIA_PLUGIN_HOME")
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|home| home.join(".opentopia/plugins")))
        .unwrap_or_else(|| PathBuf::from(".opentopia/plugins"))
}

fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|home| home.join(".codex")))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

fn io_error(error: std::io::Error) -> PluginError {
    PluginError::Io(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("opentopia-plugin-{}", Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn plugin(&self, name: &str, manifest: &str) -> PathBuf {
            let root = self.0.join(name);
            fs::create_dir_all(root.join(".codex-plugin")).unwrap();
            fs::write(root.join(MANIFEST_RELATIVE_PATH), manifest).unwrap();
            root
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn reads_codex_manifest_and_stdio_mcp() {
        let dir = TestDir::new();
        let plugin = dir.plugin(
            "review-tools",
            r##"{
              "name":"review-tools",
              "version":"1.2.3",
              "description":"Review code",
              "skills":"./skills",
              "mcpServers":"./.mcp.json",
              "interface":{"displayName":"Review Tools","category":"Developer Tools","brandColor":"#227755"}
            }"##,
        );
        fs::create_dir_all(plugin.join("skills/review")).unwrap();
        fs::write(plugin.join("skills/review/SKILL.md"), "# Review").unwrap();
        fs::write(
            plugin.join(".mcp.json"),
            r#"{"mcpServers":{"review":{"type":"stdio","command":"node","args":["server.js"],"env":{"TOKEN":"${TOKEN}"}}}}"#,
        )
        .unwrap();

        let descriptor = inspect_plugin(&plugin).unwrap();
        assert_eq!(descriptor.display_name, "Review Tools");
        assert_eq!(descriptor.skill_count, 1);
        assert_eq!(descriptor.supported_mcp_server_count, 1);
        assert!(descriptor.is_compatible());
        let servers = load_plugin_mcp_servers(&descriptor).unwrap();
        assert_eq!(servers[0].name, "review");
        assert_eq!(servers[0].env_keys, vec!["TOKEN"]);
    }

    #[test]
    fn reports_http_mcp_as_detected_but_unsupported() {
        let dir = TestDir::new();
        let plugin = dir.plugin("remote", r#"{"name":"remote","mcpServers":"./.mcp.json"}"#);
        fs::write(
            plugin.join(".mcp.json"),
            r#"{"mcpServers":{"remote":{"type":"http","url":"https://example.com/mcp"}}}"#,
        )
        .unwrap();
        let descriptor = inspect_plugin(&plugin).unwrap();
        assert_eq!(descriptor.mcp_server_count, 1);
        assert_eq!(descriptor.supported_mcp_server_count, 0);
        assert!(descriptor
            .issues
            .iter()
            .any(|issue| issue.contains("stdio only")));
    }

    #[test]
    fn rejects_declared_paths_outside_the_plugin() {
        let dir = TestDir::new();
        let plugin = dir.plugin(
            "unsafe-plugin",
            r#"{"name":"unsafe-plugin","skills":"../skills"}"#,
        );
        let descriptor = inspect_plugin(&plugin).unwrap();
        assert_eq!(descriptor.skill_count, 0);
        assert!(descriptor
            .issues
            .iter()
            .any(|issue| issue.contains("escapes")));
    }
}
