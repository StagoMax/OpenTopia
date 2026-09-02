use opentopia_core::mcp_host::McpExtensionHost;
use opentopia_core::{
    discover_plugins, load_plugin_mcp_servers, normalize_workspace_key,
    CapabilityActivationRequest, CapabilityActivationScope, CapabilityActivationSnapshot,
    CapabilityRegistry, ContributionKind, McpServerConfig, PluginActivation,
    PluginActivationRecord, PluginActivationScopeType, PluginContribution, PluginControlScopeType,
    PluginDescriptor, PluginPermission, PluginPermissionGrantRecord, PluginPermissionGrantStatus,
    SqliteSessionStore, Thread,
};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use uuid::Uuid;

/// One resolved view of the installed plugin set for a user/project context.
/// Every runtime surface derives its Skills, MCP servers, native tools, apps,
/// and model-visible catalog from this outcome.
#[derive(Debug, Clone)]
pub(crate) struct PluginLoadOutcome {
    plugins: Vec<LoadedPlugin>,
    capability_snapshot: CapabilityActivationSnapshot,
}

#[derive(Debug, Clone)]
pub(crate) struct LoadedPlugin {
    pub descriptor: PluginDescriptor,
    pub enabled: bool,
    pub granted_permissions: Vec<String>,
}

impl PluginLoadOutcome {
    pub fn plugins(&self) -> impl ExactSizeIterator<Item = &LoadedPlugin> {
        self.plugins.iter()
    }

    pub fn descriptors(&self) -> impl ExactSizeIterator<Item = &PluginDescriptor> {
        self.plugins.iter().map(|plugin| &plugin.descriptor)
    }

    pub fn plugin(&self, plugin_id: &str) -> Option<&LoadedPlugin> {
        self.plugins
            .iter()
            .find(|plugin| plugin.descriptor.id == plugin_id)
    }

    pub fn capability_snapshot(&self) -> &CapabilityActivationSnapshot {
        &self.capability_snapshot
    }

    pub fn active_contributions(&self) -> impl Iterator<Item = &PluginContribution> {
        self.capability_snapshot
            .active
            .iter()
            .map(|active| &active.contribution)
    }

    pub fn active_plugin_ids(&self, kind: ContributionKind) -> BTreeSet<String> {
        self.active_contributions()
            .filter(|contribution| contribution.kind == kind)
            .map(|contribution| contribution.plugin_id.clone())
            .collect()
    }
}

pub(crate) fn load_plugin_outcome(
    store: &SqliteSessionStore,
    workspace_root: Option<&Path>,
    thread_id: Option<Uuid>,
) -> anyhow::Result<PluginLoadOutcome> {
    let descriptors = discover_plugins(workspace_root);
    let mut registry = CapabilityRegistry::new();
    let mut activations = Vec::with_capacity(descriptors.len());
    let mut plugins = Vec::with_capacity(descriptors.len());

    for descriptor in descriptors {
        store.migrate_plugin_identity(&descriptor.id, &descriptor.legacy_ids)?;
        let records = store.list_plugin_activations(&descriptor.id)?;
        let grants = store.list_plugin_permission_grants(&descriptor.id)?;
        let granted_permissions = effective_granted_permissions(&grants, workspace_root, thread_id);
        let enabled = store.plugin_effectively_enabled(
            &descriptor.id,
            descriptor.default_enabled,
            workspace_root,
        )?;
        activations.push(capability_activation(
            &descriptor,
            &records,
            &granted_permissions,
            workspace_root,
        ));
        registry.register_plugin(descriptor.capability_registration())?;
        plugins.push(LoadedPlugin {
            descriptor,
            enabled,
            granted_permissions,
        });
    }

    let capability_snapshot = registry.activation_snapshot(CapabilityActivationRequest {
        scope: CapabilityActivationScope {
            workspace_id: workspace_root.map(normalize_workspace_key),
            thread_id: thread_id.map(|thread_id| thread_id.to_string()),
        },
        host_capabilities: host_capabilities(),
        plugins: activations,
    });
    Ok(PluginLoadOutcome {
        plugins,
        capability_snapshot,
    })
}

pub(crate) fn load_plugin_outcome_for_thread(
    store: &SqliteSessionStore,
    thread: &Thread,
) -> anyhow::Result<PluginLoadOutcome> {
    load_plugin_outcome(store, Some(&thread.workspace_root), Some(thread.id))
}

/// Materializes manifest-declared MCP servers as a derived runtime cache.
/// The manifest remains authoritative; stale cached rows are removed.
pub(crate) async fn sync_plugin_mcp_configs(
    store: &SqliteSessionStore,
    host: &McpExtensionHost,
    plugin: &PluginDescriptor,
) -> anyhow::Result<Vec<McpServerConfig>> {
    let definitions = load_plugin_mcp_servers(plugin)?;
    let mut existing = store
        .list_plugin_mcp_servers(&plugin.id)?
        .into_iter()
        .filter_map(|server| server.plugin_server_name.clone().map(|name| (name, server)))
        .collect::<HashMap<_, _>>();
    let mut synchronized = Vec::with_capacity(definitions.len());
    for definition in definitions {
        let display_name = format!(
            "{}/{} [{}]",
            plugin.name,
            definition.name,
            short_plugin_identity(&plugin.id)
        );
        let mut server = existing.remove(&definition.name).unwrap_or_else(|| {
            McpServerConfig::new(display_name.clone(), definition.command.clone())
        });
        server.name = display_name;
        server.command = definition.command;
        server.args = definition.args;
        server.cwd = Some(definition.cwd);
        server.env_keys = definition.env_keys;
        server.timeout_ms = definition.timeout_ms;
        server.plugin_id = Some(plugin.id.clone());
        server.plugin_server_name = Some(definition.name);
        server.refresh_updated_at();
        let server = if store.get_mcp_server(server.server_id)?.is_some() {
            store
                .update_mcp_server(server)?
                .ok_or_else(|| anyhow::anyhow!("plugin MCP server disappeared during sync"))?
        } else {
            store.insert_mcp_server(server)?
        };
        synchronized.push(server);
    }
    for stale in existing.into_values() {
        host.stop_server(stale.server_id).await.ok();
        store.delete_mcp_server(stale.server_id)?;
    }
    Ok(synchronized)
}

pub(crate) fn short_plugin_identity(plugin_id: &str) -> String {
    let hash = plugin_id
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
    format!("{hash:08x}").chars().take(8).collect()
}

fn capability_activation(
    plugin: &PluginDescriptor,
    records: &[PluginActivationRecord],
    granted_permissions: &[String],
    workspace_root: Option<&Path>,
) -> PluginActivation {
    let workspace_id = workspace_root.map(normalize_workspace_key);
    let enabled_at = |scope_type, scope_id: Option<&str>| {
        records
            .iter()
            .find(|record| {
                record.scope.scope_type == scope_type
                    && record.scope.scope_id.as_deref() == scope_id
            })
            .map(|record| record.enabled)
    };
    let granted = granted_permissions.iter().collect::<BTreeSet<_>>();
    let granted_permissions = plugin
        .capability_manifest
        .permissions
        .requirements()
        .into_iter()
        .filter(|permission| granted.contains(&permission_key(permission)))
        .collect();
    PluginActivation {
        plugin_id: plugin.id.clone(),
        global_enabled: enabled_at(PluginActivationScopeType::Global, None),
        workspace_enabled: workspace_id.as_deref().and_then(|workspace_id| {
            enabled_at(PluginActivationScopeType::Workspace, Some(workspace_id))
        }),
        granted_permissions,
    }
}

pub(crate) fn effective_granted_permissions(
    records: &[PluginPermissionGrantRecord],
    workspace_root: Option<&Path>,
    thread_id: Option<Uuid>,
) -> Vec<String> {
    let workspace_id = workspace_root.map(normalize_workspace_key);
    let thread_id = thread_id.map(|thread_id| thread_id.to_string());
    let relevant = |record: &&PluginPermissionGrantRecord| match record.scope.scope_type {
        PluginControlScopeType::Global => true,
        PluginControlScopeType::Workspace => {
            record.scope.scope_id.as_deref() == workspace_id.as_deref()
        }
        PluginControlScopeType::Thread => record.scope.scope_id.as_deref() == thread_id.as_deref(),
    };
    let permissions = records
        .iter()
        .filter(relevant)
        .map(|record| record.permission.clone())
        .collect::<BTreeSet<_>>();
    permissions
        .into_iter()
        .filter(|permission| {
            let matching = records
                .iter()
                .filter(relevant)
                .filter(|record| record.permission == *permission)
                .collect::<Vec<_>>();
            matching
                .iter()
                .any(|record| record.status == PluginPermissionGrantStatus::Granted)
                && !matching
                    .iter()
                    .any(|record| record.status == PluginPermissionGrantStatus::Revoked)
        })
        .collect()
}

fn permission_key(permission: &PluginPermission) -> String {
    let category = match permission.kind {
        opentopia_core::PluginPermissionKind::Filesystem => "filesystem",
        opentopia_core::PluginPermissionKind::Network => "network",
        opentopia_core::PluginPermissionKind::Secret => "secrets",
        opentopia_core::PluginPermissionKind::Desktop => "desktop",
    };
    format!("{category}:{}", permission.value)
}

fn host_capabilities() -> Vec<String> {
    [
        "workspace.files.v1",
        "artifact.runtime.v1",
        "artifact.preview.v1",
        "nativeTool.pdf.v1",
        "nativeTool.document.v1",
        "nativeTool.spreadsheet.v1",
        "browser.runtime.v1",
        "policy.network.v1",
        "nativeTool.browser.v1",
        "computer.driver.v1",
        "policy.approval.v1",
        "nativeTool.computer.v1",
        "localGit.read.v1",
        "localGit.mutate.v1",
        "previewer.v1",
        "contextLoader.v1",
        "agentProfile.v1",
        "appView.v1",
        "scmConnector.v1",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}
