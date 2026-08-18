use super::{current_settings, load_bound_agent_context, plugins_api, AppState};
use anyhow::Context;
use opentopia_core::collaboration::{
    AgentCollaborationInvocation, AgentInvocationIdentity, AgentSpawnPolicy, AgentThreadRecord,
    AgentTurnId, AgentTurnRecord as CollaborationTurnRecord, AgentTurnStatus,
    CollaborationRegistry, CollaborationSessionPolicy, CreateCollaborationSession,
    RuntimeSnapshotSeed,
};
use opentopia_core::mcp_host::McpExtensionHost;
use opentopia_core::{
    discover_plugins, AgentCore, AgentProfileRegistry, ContributionKind, ExperienceMode,
    ExperienceSurfaceProfile, LoadedSkill, Message, MessagePart, ProviderSettings, SessionStore,
    SqliteSessionStore, TurnInboxItem, GIT_NONINTERACTIVE_ENVIRONMENT,
};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::path::Path as FsPath;
use tokio::process::Command;
use tracing::{error, warn};
use uuid::Uuid;

pub(super) async fn sync_thread_mcp_tools(
    store: &SqliteSessionStore,
    host: &McpExtensionHost,
    thread_id: Uuid,
    agent: &mut AgentCore,
) {
    let thread = match store.get_thread(thread_id) {
        Ok(Some(thread)) => thread,
        Ok(None) => return,
        Err(err) => {
            error!(?err, %thread_id, "failed to load thread for plugin activation");
            return;
        }
    };
    let active_mcp_plugins = match plugins_api::active_contributions_for_thread(store, &thread) {
        Ok(contributions) => contributions
            .into_iter()
            .filter(|contribution| contribution.kind == ContributionKind::McpServer)
            .map(|contribution| contribution.plugin_id)
            .collect::<BTreeSet<_>>(),
        Err(err) => {
            error!(?err, %thread_id, "failed to resolve MCP capability snapshot");
            BTreeSet::new()
        }
    };
    let enabled_servers = match store.list_mcp_servers() {
        Ok(servers) => servers
            .into_iter()
            .filter(|server| server.enabled)
            .map(|server| (server.server_id, server))
            .collect::<HashMap<_, _>>(),
        Err(err) => {
            error!(?err, %thread_id, "failed to load MCP server configuration");
            return;
        }
    };
    let legacy_bindings = match store.list_thread_mcp_servers(thread_id) {
        Ok(bindings) => bindings
            .into_iter()
            .map(|binding| (binding.server_id, binding.enabled))
            .collect::<HashMap<_, _>>(),
        Err(err) => {
            error!(?err, %thread_id, "failed to load thread MCP bindings");
            return;
        }
    };
    let mut server_ids = Vec::new();
    for server in enabled_servers.values() {
        let enabled = match server.plugin_id.as_ref() {
            Some(plugin_id) => active_mcp_plugins.contains(plugin_id),
            None => legacy_bindings
                .get(&server.server_id)
                .copied()
                .unwrap_or(false),
        };
        if enabled
            && agent
                .capability_projection()
                .allows_mcp_server(&server.server_id.to_string())
        {
            server_ids.push(server.server_id);
        }
    }
    let mut ready_server_ids = Vec::with_capacity(server_ids.len());
    for server_id in server_ids {
        let Some(server) = enabled_servers.get(&server_id) else {
            continue;
        };
        match host.ensure_server(server.clone()).await {
            Ok(_) => ready_server_ids.push(server_id),
            Err(err) => {
                warn!(?err, %thread_id, %server_id, "failed to start thread MCP server");
            }
        }
    }
    agent.sync_mcp_tools_for_servers(&ready_server_ids).await;
}

pub(super) fn sync_thread_bundled_plugin_activations(
    store: &SqliteSessionStore,
    thread_id: Uuid,
    agent: &mut AgentCore,
) {
    let thread = match store.get_thread(thread_id) {
        Ok(Some(thread)) => thread,
        Ok(None) => return,
        Err(err) => {
            error!(?err, %thread_id, "failed to load bundled plugin thread");
            agent.disable_all_bundled_plugins();
            return;
        }
    };
    let active_native_plugins = match plugins_api::active_contributions_for_thread(store, &thread) {
        Ok(contributions) => contributions
            .into_iter()
            .filter(|contribution| contribution.kind == ContributionKind::NativeTool)
            .map(|contribution| contribution.plugin_id)
            .collect::<BTreeSet<_>>(),
        Err(err) => {
            error!(?err, %thread_id, "failed to resolve bundled capability snapshot");
            BTreeSet::new()
        }
    };
    let plugins = discover_plugins(Some(&thread.workspace_root));
    let activations = plugins
        .iter()
        .filter(|plugin| !plugin.native_capabilities.is_empty())
        .map(|plugin| {
            let enabled = active_native_plugins.contains(&plugin.id);
            (plugin.name.clone(), enabled)
        })
        .collect::<HashMap<_, _>>();
    agent.set_bundled_plugin_activations(&activations);

    let allowed_applications = plugins
        .iter()
        .find(|plugin| plugin.name == "computer-use")
        .filter(|plugin| active_native_plugins.contains(&plugin.id))
        .map(|plugin| {
            store
                .effective_plugin_settings(&plugin.id, &thread.workspace_root, thread_id)
                .and_then(|settings| computer_allowed_applications(&settings))
        })
        .transpose();
    match allowed_applications {
        Ok(Some(applications)) => agent.set_computer_allowed_applications(applications),
        Ok(None) => agent.set_computer_allowed_applications(Vec::<String>::new()),
        Err(err) => {
            error!(?err, %thread_id, "failed to resolve Computer Use application allowlist");
            agent.set_computer_allowed_applications(Vec::<String>::new());
        }
    }
}

pub(super) fn computer_allowed_applications(settings: &Value) -> anyhow::Result<Vec<String>> {
    let Some(value) = settings.get("allowedApplications") else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .context("computer-use allowedApplications must be an array")?;
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .context("computer-use allowedApplications entries must be non-empty strings")
        })
        .collect()
}

pub(super) fn attachment_preloaded_tools(messages: &[Message]) -> BTreeSet<&'static str> {
    const PDF: &str = "application/pdf";
    const DOCX: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
    const XLSX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";

    let mut tools = BTreeSet::new();
    for source in messages.iter().rev().flat_map(|message| {
        message.parts.iter().filter_map(|part| match part {
            MessagePart::SourceRef { source } => Some(source),
            _ => None,
        })
    }) {
        let content_type = source
            .content_type
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let extension = source
            .path
            .extension()
            .or_else(|| FsPath::new(&source.name).extension())
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        match (content_type.as_str(), extension.as_str()) {
            (PDF, _) | (_, "pdf") => {
                tools.insert("pdf");
            }
            (DOCX, _) | (_, "docx") => {
                tools.insert("document");
            }
            (XLSX, _) | (_, "xlsx") => {
                tools.insert("spreadsheet");
            }
            _ => {}
        }
        if tools.len() == 3 {
            break;
        }
    }
    tools
}

pub(super) fn sync_thread_attachment_tool_preloads(
    store: &SqliteSessionStore,
    thread_id: Uuid,
    agent: &mut AgentCore,
) {
    match store.list_messages(thread_id) {
        Ok(messages) => agent.set_attachment_preloaded_tools(attachment_preloaded_tools(&messages)),
        Err(err) => {
            error!(?err, %thread_id, "failed to load attachment tool projection");
            agent.set_attachment_preloaded_tools(std::iter::empty::<&str>());
        }
    }
}

pub(super) fn ensure_plugin_skills_enabled(
    store: &SqliteSessionStore,
    thread: &opentopia_core::Thread,
    skills: &[LoadedSkill],
) -> anyhow::Result<()> {
    let active_skill_plugins = plugins_api::active_contributions_for_thread(store, thread)?
        .into_iter()
        .filter(|contribution| contribution.kind == ContributionKind::Skill)
        .map(|contribution| contribution.plugin_id)
        .collect::<BTreeSet<_>>();
    for skill in skills {
        let Some(plugin_id) = skill.descriptor.plugin_id.as_deref() else {
            continue;
        };
        if !active_skill_plugins.contains(plugin_id) {
            anyhow::bail!(
                "plugin capability is unavailable for this thread; its Skill '{}' cannot be used",
                skill.descriptor.name
            );
        }
    }
    Ok(())
}

pub(super) fn ensure_mode_skills_visible(
    mode: ExperienceMode,
    skills: &[LoadedSkill],
) -> anyhow::Result<()> {
    let profile = ExperienceSurfaceProfile::for_mode(mode);
    for skill in skills {
        if !profile.capabilities.allows_skill(&skill.descriptor.id)
            || skill
                .descriptor
                .plugin_id
                .as_ref()
                .is_some_and(|plugin_id| !profile.capabilities.allows_plugin(plugin_id))
        {
            anyhow::bail!(
                "Skill '{}' is not visible in {} mode",
                skill.descriptor.name,
                mode.as_str()
            );
        }
    }
    Ok(())
}

pub(super) fn ensure_bound_agent_skills_visible(
    state: &AppState,
    thread: &opentopia_core::Thread,
    skills: &[LoadedSkill],
) -> anyhow::Result<()> {
    let (instance, _) =
        load_bound_agent_context(state, thread).map_err(|error| anyhow::anyhow!(error.message))?;
    let Some(instance) = instance else {
        return Ok(());
    };
    for skill in skills {
        if !instance
            .execution_context
            .capabilities
            .allows_skill(&skill.descriptor.id)
            || skill
                .descriptor
                .plugin_id
                .as_ref()
                .is_some_and(|plugin_id| {
                    !instance
                        .execution_context
                        .capabilities
                        .allows_plugin(plugin_id)
                })
        {
            anyhow::bail!(
                "Skill '{}' is outside the bound Agent ExecutionContext",
                skill.descriptor.name
            );
        }
    }
    Ok(())
}

pub(super) fn load_agent_profiles_for_thread(
    store: &SqliteSessionStore,
    thread: &opentopia_core::Thread,
) -> anyhow::Result<AgentProfileRegistry> {
    let plugins = discover_plugins(Some(&thread.workspace_root));
    let enabled_plugin_ids = plugins_api::active_contributions_for_thread(store, thread)?
        .into_iter()
        .filter(|contribution| contribution.kind == ContributionKind::AgentProfile)
        .map(|contribution| contribution.plugin_id)
        .collect::<BTreeSet<_>>();
    Ok(AgentProfileRegistry::load_with_plugin_profiles(
        &thread.workspace_root,
        &plugins,
        &enabled_plugin_ids,
    ))
}

pub(super) async fn bind_root_collaboration(
    state: &AppState,
    thread: &opentopia_core::Thread,
    turn_id: Uuid,
    invocation_id: u64,
    task_message: &str,
    selected_provider: &ProviderSettings,
    agent: &mut AgentCore,
) -> anyhow::Result<(AgentThreadRecord, CollaborationTurnRecord)> {
    let collaboration_turn_id = AgentTurnId::from_uuid(turn_id);
    let (runtime_snapshot, spawn_policy) =
        freeze_root_runtime_snapshot(state, thread, selected_provider, agent).await?;
    let (root, collaboration_turn) = match state
        .collaboration_repository
        .find_session_by_user_task_id(thread.id)?
    {
        Some(session) => state.collaboration_repository.create_root_followup_turn(
            session.id,
            collaboration_turn_id,
            task_message,
            runtime_snapshot,
        )?,
        None => {
            let (_, root, turn) = state
                .collaboration_repository
                .create_session(CreateCollaborationSession {
                    user_task_id: thread.id,
                    root_turn_id: collaboration_turn_id,
                    root_task_message: task_message.to_string(),
                    root_agent_type: "default".to_string(),
                    root_runtime_snapshot: runtime_snapshot,
                    session_policy: CollaborationSessionPolicy {
                        max_agents: 16,
                        max_active_runs: 6,
                        max_depth: 4,
                    },
                    root_spawn_policy: spawn_policy,
                })
                .await?;
            (root, turn)
        }
    };
    let collaboration_turn = state
        .collaboration_repository
        .transition_turn(collaboration_turn.id, AgentTurnStatus::Running)
        .await?;
    state.agent_activity.notify(root.id);
    let invocation = AgentCollaborationInvocation::new(
        state.collaboration_runtime.clone(),
        state.agent_activity.clone(),
        state.snapshot_deriver.clone(),
        AgentInvocationIdentity {
            session_id: root.session_id,
            agent_thread_id: root.id,
            agent_turn_id: collaboration_turn.id,
            runtime_snapshot_id: root.runtime_snapshot_id,
        },
    );
    agent.set_agent_execution_identity(collaboration_turn.id, invocation_id, &root.path);
    agent.set_agent_collaboration(invocation.clone());
    for message in invocation.pending_messages(256).await? {
        state.turn_inbox.push(
            collaboration_turn.id.as_uuid(),
            TurnInboxItem::AgentMessage { message },
        );
    }
    Ok((root, collaboration_turn))
}

pub(super) async fn freeze_root_runtime_snapshot(
    state: &AppState,
    thread: &opentopia_core::Thread,
    selected_provider: &ProviderSettings,
    agent: &AgentCore,
) -> anyhow::Result<(RuntimeSnapshotSeed, AgentSpawnPolicy)> {
    let profiles = load_agent_profiles_for_thread(&state.store, thread)?;
    let agent_profiles = profiles.list();
    let allowed_agent_types = agent_profiles
        .iter()
        .map(|profile| profile.name.clone())
        .collect::<Vec<_>>();
    let spawn_policy = AgentSpawnPolicy::allows_children(4, 6);
    let git_base_commit = frozen_git_head(&thread.workspace_root).await;
    let tool_catalog = agent.provider_tool_catalog();
    let tool_names = tool_catalog
        .iter()
        .map(|tool| tool.name.clone())
        .collect::<Vec<_>>();
    let plugin_contributions = plugins_api::active_contributions_for_thread(&state.store, thread)?;
    let attachment_references = state
        .store
        .list_messages(thread.id)?
        .into_iter()
        .flat_map(|message| message.parts)
        .filter_map(|part| match part {
            MessagePart::SourceRef { source } => Some(source),
            _ => None,
        })
        .collect::<Vec<_>>();
    let settings = current_settings(state);
    let runtime_snapshot = RuntimeSnapshotSeed::new(
        None,
        json!({
            "schemaVersion": 1,
            "agentType": "default",
            "allowedAgentTypes": allowed_agent_types,
            "agentProfiles": agent_profiles,
            "workspaceRoot": thread.workspace_root,
            "workspaceMode": "shared_coordinated",
            "workspaceAssignment": {
                "mode": "shared_coordinated",
                "root": thread.workspace_root,
            },
            "gitBaseCommit": git_base_commit,
            "forkTurns": "all",
            "provider": selected_provider,
            "permissionMode": settings.permission_mode,
            "sandbox": settings.sandbox,
            "agentRuntime": settings.agent_runtime,
            "capabilityProjection": agent.capability_projection(),
            "tools": tool_names,
            "toolCatalog": tool_catalog,
            "pluginContributions": plugin_contributions,
            "attachmentReferences": attachment_references,
            "spawnPolicy": {
                "allowChildSpawns": spawn_policy.allow_child_spawns,
                "maxDepth": spawn_policy.max_depth,
                "maxDirectChildren": spawn_policy.max_direct_children,
            }
        }),
    );
    runtime_snapshot.validate()?;
    Ok((runtime_snapshot, spawn_policy))
}

pub(super) async fn frozen_git_head(workspace_root: &FsPath) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_root)
        .args(["rev-parse", "HEAD"])
        .envs(GIT_NONINTERACTIVE_ENVIRONMENT)
        .output()
        .await
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub(super) async fn bind_existing_collaboration_turn(
    state: &AppState,
    turn_id: Uuid,
    invocation_id: u64,
    agent: &mut AgentCore,
) -> anyhow::Result<()> {
    let turn = state
        .collaboration_repository
        .get_turn(AgentTurnId::from_uuid(turn_id))
        .await?;
    let thread = state
        .collaboration_repository
        .get_thread(turn.agent_thread_id)
        .await?;
    anyhow::ensure!(
        turn.status == AgentTurnStatus::Running,
        "canonical AgentTurn must be running before the root execution is rebound"
    );
    anyhow::ensure!(
        turn.invocation_id == invocation_id,
        "root product projection invocation does not match canonical AgentTurn"
    );
    let invocation = AgentCollaborationInvocation::new(
        state.collaboration_runtime.clone(),
        state.agent_activity.clone(),
        state.snapshot_deriver.clone(),
        AgentInvocationIdentity {
            session_id: thread.session_id,
            agent_thread_id: thread.id,
            agent_turn_id: turn.id,
            runtime_snapshot_id: thread.runtime_snapshot_id,
        },
    );
    agent.set_agent_execution_identity(turn.id, invocation_id, &thread.path);
    agent.set_agent_collaboration(invocation.clone());
    for message in invocation.pending_messages(256).await? {
        state
            .turn_inbox
            .push(turn.id.as_uuid(), TurnInboxItem::AgentMessage { message });
    }
    state.agent_activity.notify(thread.id);
    Ok(())
}
