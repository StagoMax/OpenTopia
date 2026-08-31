use anyhow::Context;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use clap::Parser;
#[cfg(test)]
use futures_util::StreamExt;
use opentopia_core::collaboration::{
    AgentListItem, AgentPath, AgentRunCommand, AgentRunScheduler, AgentTurnId,
    CollaborationRegistry,
};
#[cfg(test)]
use opentopia_core::collaboration::{
    AgentMailboxNotifier, AgentSpawnPolicy, AgentTurnStatus, CollaborationSessionPolicy,
    CreateCollaborationSession, RuntimeSnapshotSeed, SqliteAgentActivitySource,
    SqliteCollaborationRepository,
};
use opentopia_core::mcp_host::McpExtensionHost;
use opentopia_core::AgentResumeSignal;
#[cfg(test)]
use opentopia_core::PreviewTarget;
use opentopia_core::{
    agent_model_context_with_runtime, browser_handoff_for_node, configured_provider_from_settings,
    content_fingerprint, current_office_runtime_status, current_shell_runtime_status,
    discover_plugins, discover_skills, execute_git_workflow, experience_mode_module,
    install_plugin, load_plugin_mcp_servers, negotiate_provider_settings, permission_policy_module,
    redact_model_observation, remove_windows_sandbox, resolve_instruction_documents,
    setup_windows_sandbox, tool_result_is_error, uninstall_plugin, windows_sandbox_setup_status,
    world_state_catalog_item, AgentContextBudget, AgentContinuation, AgentContinuationState,
    AgentCore, AgentEvent, AgentEventPayload, AgentInstanceStatusV1, AgentInstanceV1,
    AgentRunConfig, AgentRunIdentity, AgentRuntimeSettings, AgentTemplateVersionV1, AgentTurnInput,
    AgentTurnOutcome, AppSettings, Approval, ApprovalStatus, Artifact, ArtifactMetadata,
    BasicPolicyEngine, BrowserAction, BrowserActionReceipt, BrowserContent, BrowserDownloadRequest,
    BrowserNavigateRequest, BrowserNodeRef, BrowserObservation, BrowserObservationId,
    BrowserObserveOptions, BrowserOutput, BrowserRuntimeRoute, BrowserSelector, BrowserSessionId,
    BrowserSessionSpec, BrowserTargetRef, BrowserWaitCondition, BrowserWaitRequest,
    CapabilityProjection, CodexAccountStatus, CodexLoginStart, CollaborationMode,
    CompiledModelContext, ComputerSessionId, ContextCacheScope, ContextCheckpoint,
    ContextCheckpointCoverage, ContextCheckpointMode, ContextCompactionDetails,
    ContextCompactionMetrics, ContextItemKind, ContextProjection, ContextRole, ContextSensitivity,
    ContextSummary, ContributionKind, ExecutionAuthority, ExecutionContext, ExperienceMode,
    ExperienceSurfaceProfile, GitWorkflowAction, GitWorkflowRequest, GoalRecord, GoalStatus,
    LoadedSkill, LocalExecutionEnvironment, McpCallResult, McpServerConfig, McpServerStatus,
    McpToolDescriptor, MediaHandlerSelection, Message, MessagePart, MessageRole, ModelCallPurpose,
    ModelContentPart, ModelContextItem, ModelConversationMessage, ModelConversationRole,
    ModelGateway, ModelStreamDelta, ObserveOptions, OfficeRuntimeStatus, PermissionMode,
    PluginControlScope, PluginDescriptor, PluginError, PolicyDecision, PolicyEngine,
    PreviewDescriptor, PreviewError, PreviewKind, PreviewRange, PreviewRangeRequest,
    PreviewWorkbook, ProviderAdapterKind, ProviderAuthKind, ProviderConversationCursor,
    ProviderConversationState, ProviderDriverDescriptor, ProviderDriverRegistry, ProviderHealth,
    ProviderHealthCheck, ProviderKind, ProviderModelGateway, ProviderSettings, ProviderToolCall,
    ProviderToolResult, ProviderTransportEvent, ProviderTransportKind, ResolvedPreview,
    ResourceLimit, RuntimeSurface, SandboxDescriptor, SandboxSettings, SessionStore,
    ShellRuntimeStatus, SkillDescriptor, SqliteSessionStore, StoreError, ThreadContextSnapshot,
    ThreadMcpServer, ThreadModelSelection, ToolCall, ToolPermissionDescriptor, ToolResult,
    TurnChangeSet, TurnChangeSetStatus, TurnContextSnapshot, TurnInboxItem, TurnRecord, TurnStatus,
    UserInputResponse, UserInputStatus, WindowsSandboxSetupStatus, WorkspaceDiff, WorldStateSkill,
    WorldStateSnapshot, CONTEXT_CHECKPOINT_SCHEMA_VERSION, MAX_PREVIEW_CONTENT_BYTES,
    MIN_PROVIDER_CONTEXT_WINDOW_TOKENS,
};
#[cfg(test)]
use opentopia_core::{ContextSourceRef, UserInputRequest};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
#[cfg(test)]
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tracing::{error, warn};
use uuid::Uuid;

mod agent_connection_access;
mod agent_factory;
mod agent_runs;
mod agent_templates_api;
mod agent_turn_coordinator;
mod app_state;
mod auth;
mod bootstrap;
mod browser_api;
mod connection_operation_runtime;
mod connections_api;
mod context_api;
mod contributions_api;
mod conversation_api;
mod desktop_http_contract;
mod event_bus;
mod events_api;
mod flow_cases_api;
mod flow_cases_service;
mod flow_library_runtime;
mod flows_api;
mod human_tasks_api;
mod interaction_api;
mod library_api;
mod mcp_api;
mod message_api;
mod plugins_api;
mod provider_api;
mod provider_runtime_health;
mod resource_api;
mod resource_registry;
mod routes;
mod runtime_api;
mod runtime_shutdown;
mod scm_api;
mod send_trace;
mod terminal_api;
mod thread_runtime;
mod turn_changes;
mod turns;
mod workflow_compiler;
mod workflow_delivery;
mod workspace_api;

use agent_turn_coordinator::{drive_agent_turn, resume_agent_turn};
use app_state::AppState;
use browser_api::BrowserRuntimeStatus;
use connection_operation_runtime::connection_authority_for_context;
use context_api::{
    build_turn_model_context, checkpoint_token_estimate, context_compaction_details,
    estimate_tokens, latest_active_work_form_event, latest_context_summary_event,
    prepare_turn_context, project_model_conversation, render_context_checkpoint,
    render_message_for_summary, summary_message_cursor, truncate_chars, truncate_with_flag,
    turn_context_reservation, ContextStatusResponse, ServerRoundContextCompactor,
};
#[cfg(test)]
use context_api::{
    message_model_content_parts, model_conversation_message,
    model_conversation_message_token_estimate, model_user_message_with_attachment_manifest,
    prior_messages_for_turn, recent_conversation_tail, referenced_image_message_model_content,
    thread_context_snapshot_changed,
};
use conversation_api::{
    ensure_experience_mode_enabled, provider_settings_for_thread, GenerateThreadTitleResponse,
};
#[cfg(test)]
use conversation_api::{
    local_thread_title, CreateThreadRequest, PatchValue, UpdateProjectRequest, UpdateThreadRequest,
    MAX_THREAD_TITLE_CHARS,
};
use event_bus::EventBus;
use events_api::project_conversation_payload;
#[cfg(test)]
use events_api::{project_conversation_event, replay_then_live_events};
#[cfg(test)]
use interaction_api::validate_user_input_response;
use interaction_api::{
    ApprovalDecisionResponse, ExternalActionResumeResponse, UserInputResponseAccepted,
};
use mcp_api::{ensure_mcp_server_status, McpServerView, ThreadMcpServerView};
use message_api::{launch_next_queued_turn, message_library_provider};
#[cfg(test)]
use message_api::{
    legacy_direct_tool_command, validate_inline_image_attachments, InlineImageAttachmentRequest,
    InlineMessageContentPartRequest,
};
use provider_api::{current_settings, ProviderModelSyncResult};
#[cfg(test)]
use provider_api::{
    extract_model_catalog, provider_model_catalog_rate_limit_delay, provider_model_catalog_url,
    validate_provider_settings,
};
use provider_runtime_health::{provider_failure_is_quota_exhausted, QUOTA_EXHAUSTED_MESSAGE};
use resource_api::ResourceReleaseResponse;
#[cfg(test)]
use resource_api::{
    bundled_plugin_enabled_for_thread, resolve_preview_target, ResourceResolveRequest,
};
use resource_registry::{
    parse_resource_preview_id, resource_preview_id, ResourceLease, ResourceLocator,
};
use send_trace::ConversationSendTrace;
use terminal_api::{PtyManager, TerminalBus};
#[cfg(test)]
use thread_runtime::{attachment_preloaded_tools, computer_allowed_applications};
use thread_runtime::{
    bind_existing_collaboration_turn, bind_root_collaboration_with_connection_authority,
    ensure_bound_agent_skills_visible, ensure_mode_skills_visible, ensure_plugin_skills_enabled,
    load_agent_profiles_for_thread, sync_runtime_connection_tools,
    sync_thread_attachment_tool_preloads, sync_thread_bundled_plugin_activations,
};
use turn_changes::{TurnFileDiffPreview, TurnUndoPreview, TurnUndoResult};
use turns::{TurnCancelResult, TurnHandle};
use workspace_api::{get_workspace_diff_inner, run_git, WorkspaceDiffActionResponse};

#[derive(Debug, Parser)]
#[command(name = "opentopia-server")]
struct Args {
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value_t = 8787)]
    port: u16,
    #[arg(long, env = "OPENTOPIA_DB", default_value = ".opentopia/opentopia.db")]
    db: PathBuf,
    /// Create a compact verified database copy and exit. The source is never replaced.
    #[arg(long)]
    compact_db_output: Option<PathBuf>,
    /// ZIP archive for raw trace rows rewritten during compaction.
    #[arg(long, requires = "compact_db_output")]
    compact_trace_archive: Option<PathBuf>,
    #[arg(long, env = "OPENTOPIA_PERMISSION", default_value = "auto")]
    permission: PermissionMode,
    /// Generate the Rust-owned Desktop HTTP contract into this directory and exit.
    #[arg(long)]
    desktop_contracts_output: Option<PathBuf>,
    /// Check generated Desktop HTTP contracts instead of writing them.
    #[arg(long, requires = "desktop_contracts_output")]
    check_desktop_contracts: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "opentopia_server=info,tower_http=info".into()),
        )
        .init();

    let args = Args::parse();
    if let Some(output) = args.desktop_contracts_output.as_ref() {
        desktop_http_contract::generate(output, args.check_desktop_contracts)?;
        return Ok(());
    }
    bootstrap::run(args).await
}

fn agent_result_text(events: &[AgentEventPayload]) -> String {
    let messages = events
        .iter()
        .filter_map(|event| match event {
            AgentEventPayload::AssistantMessage { message } => Some(message),
            _ => None,
        })
        .flat_map(|message| message.parts.iter())
        .filter_map(|part| match part {
            MessagePart::Text { text } | MessagePart::ProposedPlan { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    if messages.is_empty() {
        events
            .iter()
            .rev()
            .find_map(|event| match event {
                AgentEventPayload::TurnFinished { summary } => Some(summary.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "Agent completed without a text result.".to_string())
    } else {
        messages.join("\n\n")
    }
}

const GIT_OUTPUT_BYTES_LIMIT: usize = 8 * 1024 * 1024;

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        service: "opentopia-server",
        api_version: 2,
        shell_runtime: current_shell_runtime_status(),
        office_runtime: current_office_runtime_status(),
    })
}

async fn list_skills(
    State(state): State<AppState>,
    Query(query): Query<SkillsQuery>,
) -> Result<Json<Vec<SkillDescriptor>>, ApiError> {
    let thread = query
        .thread_id
        .map(|thread_id| ensure_thread(&state, thread_id))
        .transpose()?;
    if let (Some(thread), Some(workspace_root)) = (&thread, &query.workspace_root) {
        if &thread.workspace_root != workspace_root {
            return Err(ApiError::bad_request(
                "workspaceRoot does not match the thread workspace",
            ));
        }
    }
    if let (Some(thread), Some(mode)) = (&thread, query.experience_mode) {
        if thread.experience_mode != mode {
            return Err(ApiError::bad_request(
                "experienceMode does not match the thread mode",
            ));
        }
    }
    let workspace_root = match query
        .workspace_root
        .or_else(|| thread.as_ref().map(|thread| thread.workspace_root.clone()))
    {
        Some(workspace_root) => {
            if state
                .store
                .find_project_by_workspace(&workspace_root)?
                .is_none()
            {
                return Err(ApiError::bad_request(
                    "workspace is not registered as a project",
                ));
            }
            Some(workspace_root)
        }
        None => None,
    };
    let mut skills = discover_skills(workspace_root.as_deref());
    let surface_profile = ExperienceSurfaceProfile::for_mode(
        thread
            .as_ref()
            .map(|thread| thread.experience_mode)
            .or(query.experience_mode)
            .unwrap_or(ExperienceMode::Code),
    );
    let mut effective_capabilities = surface_profile.capabilities;
    if let Some(thread) = &thread {
        if let (Some(instance), _) = load_bound_agent_context(&state, thread)? {
            effective_capabilities =
                effective_capabilities.intersect(&instance.execution_context.capabilities);
        }
    }
    skills.retain(|skill| effective_capabilities.allows_skill(&skill.id));
    if let Some(thread) = &thread {
        let active_skill_plugins =
            plugins_api::active_contributions_for_thread(&state.store, thread)
                .map_err(|error| ApiError::bad_request(error.to_string()))?
                .into_iter()
                .filter(|contribution| contribution.kind == ContributionKind::Skill)
                .map(|contribution| contribution.plugin_id)
                .collect::<BTreeSet<_>>();
        skills.retain(|skill| {
            let Some(plugin_id) = skill.plugin_id.as_ref() else {
                return true;
            };
            active_skill_plugins.contains(plugin_id)
                && effective_capabilities.allows_plugin(plugin_id)
        });
    }
    Ok(Json(skills))
}

async fn list_plugins(
    State(state): State<AppState>,
    Query(query): Query<PluginsQuery>,
) -> Result<Json<Vec<PluginView>>, ApiError> {
    let (workspace_root, thread_id) = resolve_plugin_context(&state, query)?;
    let discovery_root = workspace_root.clone();
    let plugins = tokio::task::spawn_blocking(move || discover_plugins(discovery_root.as_deref()))
        .await
        .map_err(|error| ApiError::internal(format!("plugin discovery failed: {error}")))?;
    let skills = discover_skills(workspace_root.as_deref());
    let bindings = match thread_id {
        Some(thread_id) => state.store.list_thread_mcp_servers(thread_id)?,
        None => Vec::new(),
    };
    let bindings = bindings
        .into_iter()
        .map(|binding| (binding.server_id, binding.enabled))
        .collect::<HashMap<_, _>>();
    let activations = match thread_id {
        Some(thread_id) => Some(state.store.list_thread_plugin_activations(thread_id)?),
        None => None,
    };
    let mut views = Vec::with_capacity(plugins.len());
    for plugin in plugins {
        views.push(plugin_view(&state, plugin, &skills, &bindings, activations.as_ref()).await?);
    }
    Ok(Json(views))
}

async fn install_local_plugin(
    State(state): State<AppState>,
    Json(request): Json<InstallPluginRequest>,
) -> Result<Json<PluginView>, ApiError> {
    let source = request.path;
    let plugin = tokio::task::spawn_blocking(move || install_plugin(&source))
        .await
        .map_err(|error| ApiError::internal(format!("plugin installation failed: {error}")))?
        .map_err(plugin_bad_request)?;
    sync_plugin_mcp_configs(&state, &plugin).await?;
    let skills = discover_skills(None);
    let view = plugin_view(&state, plugin, &skills, &HashMap::new(), None).await?;
    Ok(Json(view))
}

async fn uninstall_local_plugin(
    State(state): State<AppState>,
    Json(request): Json<UninstallPluginRequest>,
) -> Result<Json<DeleteResponse>, ApiError> {
    let workspace_root = validate_plugin_workspace(&state, request.workspace_root)?;
    let plugin_id = request.plugin_id;
    let plugin_servers = state.store.list_plugin_mcp_servers(&plugin_id)?;
    let uninstall_id = plugin_id.clone();
    let uninstall_root = workspace_root.clone();
    tokio::task::spawn_blocking(move || uninstall_plugin(&uninstall_id, uninstall_root.as_deref()))
        .await
        .map_err(|error| ApiError::internal(format!("plugin removal failed: {error}")))?
        .map_err(plugin_bad_request)?;
    for server in plugin_servers {
        state.mcp_host.stop_server(server.server_id).await.ok();
        state.store.delete_mcp_server(server.server_id)?;
    }
    Ok(Json(DeleteResponse { deleted: true }))
}

async fn set_thread_plugin(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(request): Json<ThreadPluginRequest>,
) -> Result<Json<PluginView>, ApiError> {
    let thread = ensure_thread(&state, thread_id)?;
    let plugins = discover_plugins(Some(&thread.workspace_root));
    let plugin = plugins
        .into_iter()
        .find(|plugin| plugin.id == request.plugin_id)
        .ok_or_else(|| ApiError::not_found("plugin is not available in this workspace"))?;
    let servers = sync_plugin_mcp_configs(&state, &plugin).await?;
    state.store.set_plugin_activation(
        &plugin.id,
        &PluginControlScope::thread(thread_id),
        request.enabled,
    )?;
    if !plugin.native_capabilities.is_empty() {
        state
            .store
            .set_thread_plugin_activation(thread_id, &plugin.name, request.enabled)?;
    }
    for server in &servers {
        state
            .store
            .set_thread_mcp_server(thread_id, server.server_id, request.enabled)?;
        if request.enabled && server.enabled {
            let _ = ensure_mcp_server_status(&state.mcp_host, server).await;
        }
    }
    let bindings = state
        .store
        .list_thread_mcp_servers(thread_id)?
        .into_iter()
        .map(|binding| (binding.server_id, binding.enabled))
        .collect::<HashMap<_, _>>();
    let activations = state.store.list_thread_plugin_activations(thread_id)?;
    let skills = discover_skills(Some(&thread.workspace_root));
    Ok(Json(
        plugin_view(&state, plugin, &skills, &bindings, Some(&activations)).await?,
    ))
}

async fn plugin_view(
    state: &AppState,
    plugin: PluginDescriptor,
    skills: &[SkillDescriptor],
    bindings: &HashMap<Uuid, bool>,
    activations: Option<&HashMap<String, bool>>,
) -> Result<PluginView, ApiError> {
    let skill_ids = skills
        .iter()
        .filter(|skill| skill.plugin_id.as_deref() == Some(plugin.id.as_str()))
        .map(|skill| skill.id.clone())
        .collect::<Vec<_>>();
    let servers = state.store.list_plugin_mcp_servers(&plugin.id)?;
    let has_native_tools = !plugin.native_capabilities.is_empty();
    let native_tools_enabled = activations.is_some_and(|activations| {
        activations
            .get(&plugin.name)
            .copied()
            .unwrap_or(plugin.default_enabled)
    });
    let has_mcp_tools = !servers.is_empty();
    let mcp_tools_enabled = servers
        .iter()
        .all(|server| bindings.get(&server.server_id).copied().unwrap_or(false));
    let thread_enabled = activations.is_some()
        && (has_native_tools || has_mcp_tools)
        && (!has_native_tools || native_tools_enabled)
        && (!has_mcp_tools || mcp_tools_enabled);
    let mut mcp_servers = Vec::with_capacity(servers.len());
    for server in servers {
        let status = state.mcp_host.status_for_config(&server).await;
        mcp_servers.push(McpServerView { server, status });
    }
    Ok(PluginView {
        compatible: plugin.is_compatible(),
        plugin,
        skill_ids,
        mcp_servers,
        thread_enabled,
    })
}

async fn sync_plugin_mcp_configs(
    state: &AppState,
    plugin: &PluginDescriptor,
) -> Result<Vec<McpServerConfig>, ApiError> {
    let definitions = load_plugin_mcp_servers(plugin).map_err(plugin_bad_request)?;
    let mut existing = state
        .store
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
        server.enabled = true;
        server.plugin_id = Some(plugin.id.clone());
        server.plugin_server_name = Some(definition.name);
        server.refresh_updated_at();
        let server = if state.store.get_mcp_server(server.server_id)?.is_some() {
            state
                .store
                .update_mcp_server(server)?
                .ok_or_else(|| ApiError::internal("plugin MCP server disappeared during sync"))?
        } else {
            state.store.insert_mcp_server(server)?
        };
        synchronized.push(server);
    }
    for stale in existing.into_values() {
        state.mcp_host.stop_server(stale.server_id).await.ok();
        state.store.delete_mcp_server(stale.server_id)?;
    }
    Ok(synchronized)
}

fn resolve_plugin_context(
    state: &AppState,
    query: PluginsQuery,
) -> Result<(Option<PathBuf>, Option<Uuid>), ApiError> {
    if let Some(thread_id) = query.thread_id {
        let thread = ensure_thread(state, thread_id)?;
        if query
            .workspace_root
            .as_ref()
            .is_some_and(|root| root != &thread.workspace_root)
        {
            return Err(ApiError::bad_request(
                "plugin workspace does not match the selected thread",
            ));
        }
        return Ok((Some(thread.workspace_root), Some(thread_id)));
    }
    Ok((
        validate_plugin_workspace(state, query.workspace_root)?,
        None,
    ))
}

fn validate_plugin_workspace(
    state: &AppState,
    workspace_root: Option<PathBuf>,
) -> Result<Option<PathBuf>, ApiError> {
    if let Some(workspace_root) = workspace_root {
        if state
            .store
            .find_project_by_workspace(&workspace_root)?
            .is_none()
        {
            return Err(ApiError::bad_request(
                "workspace is not registered as a project",
            ));
        }
        Ok(Some(workspace_root))
    } else {
        Ok(None)
    }
}

fn plugin_bad_request(error: PluginError) -> ApiError {
    ApiError::bad_request(error.to_string())
}

fn short_plugin_identity(plugin_id: &str) -> String {
    let hash = plugin_id
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
    format!("{hash:08x}").chars().take(8).collect()
}

async fn get_turn_status(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<Option<TurnRecord>>, ApiError> {
    ensure_thread(&state, thread_id)?;
    Ok(Json(state.turns.status(thread_id)?))
}

async fn get_turn_changes(
    State(state): State<AppState>,
    Path((thread_id, turn_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<TurnChangeSet>, ApiError> {
    ensure_thread(&state, thread_id)?;
    Ok(Json(turn_change_set_for_thread(
        &state, thread_id, turn_id,
    )?))
}

async fn get_turn_file_diff_preview(
    State(state): State<AppState>,
    Path((thread_id, turn_id)): Path<(Uuid, Uuid)>,
    Query(query): Query<TurnFileDiffPreviewQuery>,
) -> Result<Json<TurnFileDiffPreview>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let change_set = turn_change_set_for_thread(&state, thread_id, turn_id)?;
    Ok(Json(
        state
            .turn_changes
            .preview_file_diff(&change_set, &query.path, query.offset.unwrap_or_default())
            .await?,
    ))
}

async fn preview_turn_undo(
    State(state): State<AppState>,
    Path((thread_id, turn_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<TurnUndoPreview>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let change_set = turn_change_set_for_thread(&state, thread_id, turn_id)?;
    Ok(Json(state.turn_changes.preview_undo(change_set).await?))
}

async fn undo_turn_changes(
    State(state): State<AppState>,
    Path((thread_id, turn_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<TurnUndoRequest>,
) -> Result<Json<TurnUndoResult>, ApiError> {
    if !request.confirm {
        return Err(ApiError::bad_request(
            "confirm must be true to undo a turn change set",
        ));
    }
    ensure_thread(&state, thread_id)?;
    let change_set = turn_change_set_for_thread(&state, thread_id, turn_id)?;
    let result = state.turn_changes.undo(change_set).await?;
    if result.applied {
        publish_payload(
            &state,
            thread_id,
            Some(turn_id),
            AgentEventPayload::TurnUndoCompleted {
                target_turn_id: turn_id,
                files_changed: result.files_changed,
            },
        );
    }
    Ok(Json(result))
}

fn turn_change_set_for_thread(
    state: &AppState,
    thread_id: Uuid,
    turn_id: Uuid,
) -> Result<TurnChangeSet, ApiError> {
    let turn = state
        .store
        .get_turn(turn_id)?
        .ok_or_else(|| ApiError::not_found(format!("turn not found: {turn_id}")))?;
    if turn.thread_id != thread_id {
        return Err(ApiError::not_found(format!("turn not found: {turn_id}")));
    }
    state
        .store
        .get_turn_change_set(turn_id)?
        .ok_or_else(|| ApiError::not_found(format!("turn change set not found: {turn_id}")))
}

async fn cancel_user_turn(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(request): Json<CancelAgentTurnRequest>,
) -> Result<Json<TurnCancelResult>, ApiError> {
    Ok(Json(
        cancel_thread_turn(&state, thread_id, request.turn_id).await?,
    ))
}

pub(crate) async fn cancel_thread_turn(
    state: &AppState,
    thread_id: Uuid,
    requested_turn_id: Option<Uuid>,
) -> Result<TurnCancelResult, ApiError> {
    ensure_thread(state, thread_id)?;
    let latest = state.turns.status(thread_id)?;
    let parent_turn_id = requested_turn_id.or_else(|| latest.as_ref().map(|turn| turn.turn_id));
    let mut cancelled_waiting = false;
    let mut result = state.turns.cancel(thread_id, requested_turn_id)?;
    if !result.cancelled {
        if let Some(waiting) = latest.as_ref().filter(|turn| {
            requested_turn_id
                .map(|requested| requested == turn.turn_id)
                .unwrap_or(true)
                && matches!(
                    turn.status,
                    TurnStatus::WaitingApproval
                        | TurnStatus::WaitingUserInput
                        | TurnStatus::WaitingUserAction
                )
        }) {
            result = cancel_waiting_agent_turn(state, thread_id, waiting).await?;
            cancelled_waiting = result.cancelled;
        }
    }
    if result.cancelled {
        if let Some(turn_id) = result.turn_id {
            state.turn_inbox.push(turn_id, TurnInboxItem::Cancel);
            if cancelled_waiting {
                let _ = state.turn_inbox.drain(turn_id);
                let _ = state.turn_queue.send(thread_id);
            }
        }
        if let Some(parent_turn_id) = parent_turn_id {
            cancel_collaboration_descendants(state, thread_id, parent_turn_id).await;
        }
    }
    Ok(result)
}

async fn cancel_collaboration_descendants(state: &AppState, thread_id: Uuid, _root_turn_id: Uuid) {
    let Some(session) = state
        .collaboration_repository
        .find_session_by_user_task_id(thread_id)
        .ok()
        .flatten()
    else {
        return;
    };
    let Ok(Some(root)) = state
        .collaboration_repository
        .resolve_path(
            session.id,
            &opentopia_core::collaboration::AgentPath::root(),
        )
        .await
    else {
        return;
    };
    let Ok(agents) = state
        .collaboration_repository
        .list_threads(session.id)
        .await
    else {
        return;
    };
    for agent in agents
        .into_iter()
        .filter(|agent| agent.path.is_descendant_of(&root.path))
    {
        let Ok(Some(turn)) = state.collaboration_repository.latest_turn(agent.id).await else {
            continue;
        };
        if turn.status.is_terminal() {
            continue;
        }
        let _ = state
            .agent_run_scheduler
            .submit(AgentRunCommand::Cancel {
                session_id: session.id,
                agent_thread_id: agent.id,
                agent_turn_id: turn.id,
            })
            .await;
    }
}

async fn cancel_waiting_agent_turn(
    state: &AppState,
    thread_id: Uuid,
    turn: &TurnRecord,
) -> Result<TurnCancelResult, ApiError> {
    let (wait_kind, checkpoint) = state
        .store
        .get_turn_checkpoint(turn.turn_id, thread_id)?
        .ok_or_else(|| ApiError::conflict("waiting Turn checkpoint is not available"))?;
    let continuation = decode_turn_checkpoint(&state.store, &wait_kind, checkpoint)
        .map_err(|error| ApiError::internal(format!("invalid waiting Turn checkpoint: {error}")))?;
    if continuation.turn_id != turn.turn_id || continuation.thread_id != thread_id {
        return Err(ApiError::conflict(
            "waiting Turn checkpoint does not belong to this Turn",
        ));
    }
    if state
        .turns
        .finish(thread_id, turn.turn_id, TurnStatus::Cancelled, None)?
        .is_none()
    {
        return Err(ApiError::conflict("waiting Turn is no longer available"));
    }

    match wait_kind.as_str() {
        "approval" => {
            for approval in state
                .store
                .list_approvals(thread_id, Some(ApprovalStatus::Pending))?
            {
                if let Err(error) = state
                    .store
                    .update_approval_status(approval.approval_id, ApprovalStatus::Denied)
                {
                    warn!(?error, approval_id = %approval.approval_id, "failed to close approval for cancelled Turn");
                }
                if let Err(error) = state
                    .store
                    .delete_approval_continuation(approval.approval_id, thread_id)
                {
                    warn!(?error, approval_id = %approval.approval_id, "failed to delete approval continuation for cancelled Turn");
                }
            }
        }
        "user_input" => {
            let cancelled = UserInputResponse {
                answers: Vec::new(),
                skipped: false,
                cancelled: true,
            };
            for input in state
                .store
                .list_user_input_requests(thread_id, Some(UserInputStatus::Pending))?
            {
                if let Err(error) = state.store.resolve_user_input_request(
                    input.request.request_id,
                    thread_id,
                    &cancelled,
                ) {
                    warn!(?error, request_id = %input.request.request_id, "failed to close user decision for cancelled Turn");
                }
            }
        }
        "external_action" => {}
        other => {
            return Err(ApiError::internal(format!(
                "unsupported waiting Turn checkpoint kind: {other}"
            )))
        }
    }
    if let Err(error) = state.store.delete_turn_checkpoint(turn.turn_id, thread_id) {
        warn!(?error, turn_id = %turn.turn_id, "failed to delete checkpoint for cancelled waiting Turn");
    }
    publish_payload(
        state,
        thread_id,
        Some(turn.turn_id),
        AgentEventPayload::TurnCancelled {
            reason: "Cancelled by user while waiting.".to_string(),
        },
    );
    finalize_turn_change_capture(state, thread_id, turn.turn_id, TurnStatus::Cancelled).await;
    finalize_goal_after_turn(
        state,
        thread_id,
        continuation.collaboration_mode,
        continuation.goal.as_ref().map(|goal| goal.id),
        TurnStatus::Cancelled,
    );
    Ok(TurnCancelResult {
        turn_id: Some(turn.turn_id),
        cancelled: true,
        message: "waiting agent Turn cancelled".to_string(),
    })
}

async fn run_git_workflow(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(action): Json<GitWorkflowAction>,
) -> Result<Json<GitWorkflowResponse>, ApiError> {
    let thread = ensure_thread(&state, thread_id)?;
    let action_kind = action.kind();
    let mut config = current_settings(&state).sandbox.to_local_sandbox_config();
    if action_kind.writes_metadata() {
        // A structured Git mutation is an explicit user action. Grant only the
        // repository metadata root for this one request; arbitrary shell calls
        // still go through the normal approval flow.
        config.grant_write_path(thread.workspace_root.join(".git"));
    }
    if action_kind.requires_network() {
        config.network = opentopia_core::NetworkPolicy::Allow;
    }
    let environment =
        LocalExecutionEnvironment::with_sandbox_config(thread.workspace_root.clone(), config);
    let request = GitWorkflowRequest {
        repository: thread.workspace_root,
        action,
    };
    let result = execute_git_workflow(
        &environment,
        &request,
        ExecutionContext::with_timeout(if action_kind.is_mutation() {
            Duration::from_secs(120)
        } else {
            Duration::from_secs(15)
        })
        .with_resource_limits(ResourceLimit {
            max_output_bytes: Some(GIT_OUTPUT_BYTES_LIMIT),
            ..ResourceLimit::default()
        }),
    )
    .await
    .map_err(|error| {
        let detail = error
            .failed_result()
            .map(|result| String::from_utf8_lossy(&result.stderr).trim().to_string())
            .filter(|detail| !detail.is_empty());
        ApiError::bad_request(detail.unwrap_or_else(|| error.to_string()))
    })?;
    Ok(Json(GitWorkflowResponse {
        action: result.action,
        stdout: String::from_utf8_lossy(&result.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&result.stderr).into_owned(),
        exit_code: result.exit_code,
        success: result.success,
        truncated: result.truncated,
    }))
}

fn ensure_thread(state: &AppState, thread_id: Uuid) -> Result<opentopia_core::Thread, ApiError> {
    let thread = state
        .store
        .get_thread(thread_id)?
        .ok_or_else(|| ApiError::not_found(format!("thread not found: {thread_id}")))?;
    if thread.experience_mode == ExperienceMode::Flow && !current_settings(state).enterprise.enabled
    {
        return Err(ApiError::not_found(format!(
            "thread not found: {thread_id}"
        )));
    }
    Ok(thread)
}

fn publish_payload(
    state: &AppState,
    thread_id: Uuid,
    turn_id: Option<Uuid>,
    payload: AgentEventPayload,
) {
    publish_payloads(state, thread_id, turn_id, vec![payload]);
}

fn publish_payloads(
    state: &AppState,
    thread_id: Uuid,
    turn_id: Option<Uuid>,
    payloads: Vec<AgentEventPayload>,
) {
    publish_payloads_with_messages(state, thread_id, turn_id, Vec::new(), payloads);
}

fn publish_payloads_with_messages(
    state: &AppState,
    thread_id: Uuid,
    turn_id: Option<Uuid>,
    messages: Vec<Message>,
    payloads: Vec<AgentEventPayload>,
) {
    if payloads.is_empty() {
        return;
    }
    // The Agent activity ledger is the durable execution event source. The
    // product conversation event stream is a root-only UI/SSE projection and
    // is therefore appended after the canonical event batch.
    let mut canonical_activity_persisted = false;
    if let Some(turn_id) = turn_id {
        let collaboration_turn_id = AgentTurnId::from_uuid(turn_id);
        match state
            .collaboration_repository
            .find_turn(collaboration_turn_id)
        {
            Ok(Some(turn)) => match state.collaboration_repository.append_activity_events(
                turn.session_id,
                turn.agent_thread_id,
                turn.id,
                turn.invocation_id,
                payloads.clone(),
                None,
            ) {
                Ok(_) => {
                    canonical_activity_persisted = true;
                    state.agent_activity.notify(turn.agent_thread_id);
                }
                Err(error) => error!(
                    ?error,
                    agent_turn_id = %turn.id,
                    "failed to persist canonical AgentTurn activity"
                ),
            },
            Ok(None) => {}
            Err(error) => error!(
                ?error,
                %turn_id,
                "failed to resolve canonical AgentTurn for product event projection"
            ),
        }
    }
    let events = payloads
        .into_iter()
        .filter_map(|payload| {
            if canonical_activity_persisted {
                project_conversation_payload(payload)
            } else {
                Some(payload)
            }
        })
        .map(|payload| AgentEvent::new(thread_id, turn_id, 0, payload))
        .collect();
    match state.store.append_conversation_batch(messages, events) {
        Ok(events) => {
            for event in events {
                state.events.publish(event);
            }
        }
        Err(err) => error!(?err, "failed to persist event"),
    }
}

fn finish_turn(
    state: &AppState,
    thread_id: Uuid,
    turn_id: Uuid,
    status: TurnStatus,
    turn_error: Option<String>,
) {
    match state.turns.finish(thread_id, turn_id, status, turn_error) {
        Ok(Some(record)) => {
            if record.status.is_terminal() {
                let _ = state.turn_inbox.drain(turn_id);
            }
            if record.status.is_terminal() {
                let _ = state.turn_queue.send(thread_id);
            }
        }
        Ok(None) => warn!(%thread_id, %turn_id, ?status, "turn was no longer active at finish"),
        Err(error) => {
            error!(?error, %thread_id, %turn_id, ?status, "failed to persist turn status")
        }
    }
}

fn finalize_goal_after_turn(
    state: &AppState,
    thread_id: Uuid,
    mode: CollaborationMode,
    goal_id: Option<Uuid>,
    turn_status: TurnStatus,
) {
    let Some(goal_id) = goal_id else {
        return;
    };
    let target = match (mode, turn_status) {
        (CollaborationMode::Goal, TurnStatus::Cancelled) => Some(GoalStatus::Paused),
        _ => None,
    };
    let Some(target) = target else {
        return;
    };
    let current = match state.store.get_goal(goal_id) {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => return,
        Err(error) => {
            warn!(?error, %goal_id, "failed to load goal while finalizing turn");
            return;
        }
    };
    if current.status().is_terminal() || current.status() == target {
        return;
    }
    match state.store.update_goal_status(thread_id, goal_id, target) {
        Ok(Some(snapshot)) => publish_payload(
            state,
            thread_id,
            None,
            AgentEventPayload::GoalUpdated { snapshot },
        ),
        Ok(None) => {}
        Err(error) => warn!(?error, %goal_id, "failed to finalize goal status"),
    }
}

async fn begin_turn_change_capture(
    state: &AppState,
    thread_id: Uuid,
    turn_id: Uuid,
    workspace_root: &FsPath,
) {
    match state
        .turn_changes
        .begin_capture(turn_id, thread_id, workspace_root)
        .await
    {
        Ok(change_set) if change_set.status == TurnChangeSetStatus::Failed => {
            warn!(error = ?change_set.error, %thread_id, %turn_id, "turn change capture is unavailable");
        }
        Ok(_) => {}
        Err(error) => {
            warn!(?error, %thread_id, %turn_id, "failed to start turn change capture");
        }
    }
}

async fn resume_turn_change_capture(
    state: &AppState,
    thread_id: Uuid,
    turn_id: Uuid,
    workspace_root: &FsPath,
) {
    if let Err(error) = state
        .turn_changes
        .resume_capture(turn_id, thread_id, workspace_root)
        .await
    {
        warn!(?error, %thread_id, %turn_id, "failed to resume turn change capture");
    }
}

pub(crate) async fn finalize_turn_change_capture(
    state: &AppState,
    thread_id: Uuid,
    turn_id: Uuid,
    status: TurnStatus,
) {
    if !status.is_terminal() {
        return;
    }
    let changed_paths = turn_reported_changed_paths(state, thread_id, turn_id);
    match state
        .turn_changes
        .finalize_capture_for_paths(turn_id, &changed_paths)
        .await
    {
        Ok(change_set) => {
            if change_set.status == TurnChangeSetStatus::Failed {
                warn!(error = ?change_set.error, %thread_id, %turn_id, "turn change capture could not be finalized");
            }
            publish_payload(
                state,
                thread_id,
                Some(turn_id),
                AgentEventPayload::TurnChangesRecorded { change_set },
            );
        }
        Err(error) => {
            warn!(?error, %thread_id, %turn_id, "failed to finalize turn change capture");
        }
    }
}

fn turn_reported_changed_paths(state: &AppState, thread_id: Uuid, turn_id: Uuid) -> Vec<PathBuf> {
    let events = match state.store.list_turn_tool_result_events(thread_id, turn_id) {
        Ok(events) => events,
        Err(error) => {
            warn!(?error, %thread_id, %turn_id, "failed to load turn-owned changed paths");
            return Vec::new();
        }
    };
    let mut seen = HashSet::new();
    let mut paths = Vec::new();
    for event in events {
        let AgentEventPayload::ToolCallFinished { result } = event.payload else {
            continue;
        };
        let reported = result
            .metadata
            .get("changedPath")
            .and_then(Value::as_str)
            .into_iter()
            .chain(
                result
                    .metadata
                    .get("changedPaths")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str),
            );
        for path in reported {
            if path.trim().is_empty() || !seen.insert(path.to_string()) {
                continue;
            }
            paths.push(PathBuf::from(path));
        }
    }
    paths
}

fn load_bound_agent_context(
    state: &AppState,
    thread: &opentopia_core::Thread,
) -> Result<(Option<AgentInstanceV1>, Option<AgentTemplateVersionV1>), ApiError> {
    let Some(instance) = state.store.get_bound_thread_agent_instance(thread.id)? else {
        return Ok((None, None));
    };
    if instance.status != AgentInstanceStatusV1::Active
        || instance.thread_id != thread.id
        || instance.execution_context.thread_id != thread.id
        || instance.execution_context.mode != thread.experience_mode
        || instance.execution_context.agent_id != instance.id
        || instance.execution_context.template_id != instance.template_id
        || instance.execution_context.template_version != instance.template_version
    {
        return Err(ApiError::conflict(
            "bound Agent instance has an invalid or inactive execution context",
        ));
    }
    let template = state
        .store
        .get_agent_template_version(&instance.template_id, instance.template_version)?
        .ok_or_else(|| ApiError::internal("bound Agent template version is missing"))?;
    let resolved_connection_bindings =
        agent_connection_access::resolve_agent_template_connection_access(
            &state.store,
            &template.spec,
        )?
        .require_valid()
        .map_err(ApiError::conflict)?;
    instance
        .validate_execution_boundary_with_connections(
            &template,
            &ExperienceSurfaceProfile::for_mode(thread.experience_mode).capabilities,
            &resolved_connection_bindings,
        )
        .map_err(|error| ApiError::conflict(error.to_string()))?;
    Ok((Some(instance), Some(template)))
}

async fn run_new_agent_turn(
    state: AppState,
    thread: opentopia_core::Thread,
    user_message: Message,
    content: String,
    user_content: Vec<ModelContentPart>,
    selected_skills: Vec<LoadedSkill>,
    turn: TurnHandle,
    collaboration_mode: CollaborationMode,
    goal: Option<GoalRecord>,
    library_provider: Option<library_api::LibraryProviderId>,
    send_trace: Option<ConversationSendTrace>,
) {
    let thread_id = thread.id;
    let turn_id = turn.turn_id;
    if let Some(trace) = send_trace {
        trace.phase("agent_task_started", thread_id, Some(turn_id));
    }
    let settings = current_settings(&state);
    if let Err(error) = ensure_experience_mode_enabled(&settings, thread.experience_mode) {
        let message = error.message;
        publish_payload(
            &state,
            thread_id,
            Some(turn_id),
            AgentEventPayload::Error {
                message: message.clone(),
            },
        );
        finalize_goal_after_turn(
            &state,
            thread_id,
            collaboration_mode,
            goal.as_ref().map(|goal| goal.id),
            TurnStatus::Failed,
        );
        finish_turn(
            &state,
            thread_id,
            turn_id,
            TurnStatus::Failed,
            Some(message),
        );
        return;
    }
    let selected_provider =
        provider_settings_for_thread(&settings, thread.model_selection.as_ref());
    if let Some(message) = state
        .provider_runtime_health
        .blocked_message(&selected_provider.id)
    {
        let message = message.to_string();
        publish_payload(
            &state,
            thread_id,
            Some(turn_id),
            AgentEventPayload::Error {
                message: message.clone(),
            },
        );
        finalize_goal_after_turn(
            &state,
            thread_id,
            collaboration_mode,
            goal.as_ref().map(|goal| goal.id),
            TurnStatus::Failed,
        );
        finish_turn(
            &state,
            thread_id,
            turn_id,
            TurnStatus::Failed,
            Some(message),
        );
        return;
    }
    if selected_provider.effective_transport() == ProviderTransportKind::Http
        && selected_provider.active_adapter_profile().is_none()
    {
        let message = format!(
            "Provider model {}:{} has no negotiated adapter capability profile; test or resync this connection before starting a conversation",
            selected_provider.id, selected_provider.model
        );
        publish_payload(
            &state,
            thread_id,
            Some(turn_id),
            AgentEventPayload::Error {
                message: message.clone(),
            },
        );
        finalize_goal_after_turn(
            &state,
            thread_id,
            collaboration_mode,
            goal.as_ref().map(|goal| goal.id),
            TurnStatus::Failed,
        );
        finish_turn(
            &state,
            thread_id,
            turn_id,
            TurnStatus::Failed,
            Some(message),
        );
        return;
    }
    let (bound_agent_instance, bound_agent_template) =
        match load_bound_agent_context(&state, &thread) {
            Ok(context) => context,
            Err(error) => {
                let message = error.message;
                publish_payload(
                    &state,
                    thread_id,
                    Some(turn_id),
                    AgentEventPayload::Error {
                        message: message.clone(),
                    },
                );
                finalize_goal_after_turn(
                    &state,
                    thread_id,
                    collaboration_mode,
                    goal.as_ref().map(|goal| goal.id),
                    TurnStatus::Failed,
                );
                finish_turn(
                    &state,
                    thread_id,
                    turn_id,
                    TurnStatus::Failed,
                    Some(message),
                );
                return;
            }
        };
    if let Some(instance) = bound_agent_instance.as_ref() {
        if !instance
            .execution_context
            .model_policy
            .allows(&selected_provider.id, &selected_provider.model)
        {
            let message = format!(
                "Agent template {}@{} does not allow model {}:{}",
                instance.template_id,
                instance.template_version,
                selected_provider.id,
                selected_provider.model
            );
            publish_payload(
                &state,
                thread_id,
                Some(turn_id),
                AgentEventPayload::Error {
                    message: message.clone(),
                },
            );
            finalize_goal_after_turn(
                &state,
                thread_id,
                collaboration_mode,
                goal.as_ref().map(|goal| goal.id),
                TurnStatus::Failed,
            );
            finish_turn(
                &state,
                thread_id,
                turn_id,
                TurnStatus::Failed,
                Some(message),
            );
            return;
        }
    }
    let workspace_root = thread.workspace_root.clone();
    let _workspace_guard = state
        .turn_changes
        .lock_workspace_shared(&workspace_root)
        .await;
    if let Some(trace) = send_trace {
        trace.phase("workspace_lock_acquired", thread_id, Some(turn_id));
    }
    begin_turn_change_capture(&state, thread_id, turn_id, &workspace_root).await;
    let surface_profile = ExperienceSurfaceProfile::for_mode(thread.experience_mode);
    let effective_capabilities = if thread.experience_mode == ExperienceMode::Flow {
        let mut flow_capabilities = bound_agent_instance
            .as_ref()
            .map(|instance| instance.execution_context.capabilities.clone())
            .unwrap_or_else(|| {
                ExperienceSurfaceProfile::flow_runtime_baseline(workspace_root.clone())
            });
        if !flow_capabilities.allow_all_tools {
            flow_capabilities
                .tools
                .extend(ExperienceSurfaceProfile::flow_control_tools());
            if library_provider.is_some() && bound_agent_instance.is_none() {
                flow_capabilities.tools.insert("library_search".to_string());
            }
        }
        flow_capabilities
    } else {
        bound_agent_instance
            .as_ref()
            .map(|instance| {
                surface_profile
                    .capabilities
                    .intersect(&instance.execution_context.capabilities)
            })
            .unwrap_or(surface_profile.capabilities)
    };
    let prepared_draft = ExecutionAuthority::new(
        workspace_root.clone(),
        settings.permission_mode,
        settings.sandbox.to_local_sandbox_config(),
        effective_capabilities,
    )
    .and_then(|authority| {
        let config = AgentRunConfig::from_settings(
            &settings,
            thread.model_selection.as_ref(),
            authority,
            AgentRunIdentity::root(turn_id, turn.invocation_id),
        )
        .with_experience_mode(thread.experience_mode)
        .with_collaboration_mode(collaboration_mode, goal.clone());
        state
            .agent
            .read()
            .expect("agent lock poisoned")
            .begin_run(config)
    });
    let mut agent = match prepared_draft {
        Ok(agent) => agent,
        Err(error) => {
            let message = error.to_string();
            publish_payload(
                &state,
                thread_id,
                Some(turn_id),
                AgentEventPayload::Error {
                    message: message.clone(),
                },
            );
            finalize_turn_change_capture(&state, thread_id, turn_id, TurnStatus::Failed).await;
            finalize_goal_after_turn(
                &state,
                thread_id,
                collaboration_mode,
                goal.as_ref().map(|goal| goal.id),
                TurnStatus::Failed,
            );
            finish_turn(
                &state,
                thread_id,
                turn_id,
                TurnStatus::Failed,
                Some(message),
            );
            return;
        }
    };
    agent.set_round_context_compactor(Arc::new(ServerRoundContextCompactor::new(
        state.clone(),
        selected_provider.clone(),
    )));
    agent.set_mcp_host(state.mcp_host.clone());
    if agent.capability_projection().allow_all_plugins
        || !agent.capability_projection().plugins.is_empty()
    {
        sync_thread_bundled_plugin_activations(&state.store, thread_id, &mut agent);
    } else {
        agent.disable_all_bundled_plugins();
    }
    sync_thread_attachment_tool_preloads(&state.store, thread_id, &mut agent);
    let connection_authority = connection_authority_for_context(
        thread.experience_mode,
        bound_agent_instance.as_ref(),
        bound_agent_template.as_ref(),
        agent.capability_projection(),
    );
    if let Err(error) = sync_runtime_connection_tools(
        &state.store,
        &state.mcp_host,
        thread_id,
        &connection_authority,
        &mut agent,
    )
    .await
    {
        let message = format!("failed to prepare Connection operations: {error}");
        publish_payload(
            &state,
            thread_id,
            Some(turn_id),
            AgentEventPayload::Error {
                message: message.clone(),
            },
        );
        finalize_turn_change_capture(&state, thread_id, turn_id, TurnStatus::Failed).await;
        finish_turn(
            &state,
            thread_id,
            turn_id,
            TurnStatus::Failed,
            Some(message),
        );
        return;
    }
    if let Some(binding) = bound_agent_instance
        .as_ref()
        .and_then(|instance| instance.execution_context.knowledge_binding.as_ref())
    {
        let namespaces = binding.namespaces.iter().cloned().collect::<Vec<_>>();
        agent.set_library_namespaces(namespaces.clone());
        agent.register_runtime_tool(Arc::new(library_api::LibrarySearchTool::scoped(
            state.library_providers.clone(),
            namespaces,
        )));
    } else if let Some(provider) = library_provider {
        agent.register_runtime_tool(Arc::new(library_api::LibrarySearchTool::new(
            state.library_providers.clone(),
            provider,
        )));
    }
    if let Err(error) = bind_root_collaboration_with_connection_authority(
        &state,
        &thread,
        turn_id,
        turn.invocation_id,
        &content,
        &selected_provider,
        connection_authority,
        &mut agent,
    )
    .await
    {
        let message = format!("failed to bind root Agent Turn: {error}");
        publish_payload(
            &state,
            thread_id,
            Some(turn_id),
            AgentEventPayload::Error {
                message: message.clone(),
            },
        );
        finalize_turn_change_capture(&state, thread_id, turn_id, TurnStatus::Failed).await;
        finish_turn(
            &state,
            thread_id,
            turn_id,
            TurnStatus::Failed,
            Some(message),
        );
        return;
    }
    let agent = match agent.finalize() {
        Ok(agent) => agent,
        Err(error) => {
            let message = format!("failed to prepare Agent Turn: {error}");
            publish_payload(
                &state,
                thread_id,
                Some(turn_id),
                AgentEventPayload::Error {
                    message: message.clone(),
                },
            );
            finalize_turn_change_capture(&state, thread_id, turn_id, TurnStatus::Failed).await;
            finish_turn(
                &state,
                thread_id,
                turn_id,
                TurnStatus::Failed,
                Some(message),
            );
            return;
        }
    };
    if let Some(trace) = send_trace {
        trace.phase("agent_runtime_prepared", thread_id, Some(turn_id));
    }
    let built_context = build_turn_model_context(
        &state,
        &settings,
        &selected_provider,
        thread_id,
        &workspace_root,
        thread.experience_mode,
        &selected_skills,
        &agent,
        bound_agent_instance.as_ref(),
        bound_agent_template.as_ref(),
    )
    .await;
    if let Some(trace) = send_trace {
        trace.phase("model_context_built", thread_id, Some(turn_id));
        trace.phase("history_preparation_started", thread_id, Some(turn_id));
    }
    let tool_schema_tokens = agent.provider_tool_token_estimate();
    let context_reservation = turn_context_reservation(
        &selected_provider,
        &built_context.context,
        tool_schema_tokens,
        &content,
        &user_content,
    );
    let prepared_result = tokio::select! {
        _ = turn.cancel.cancelled() => {
            publish_payload(
                &state,
                thread_id,
                Some(turn_id),
                AgentEventPayload::TurnCancelled {
                    reason: "Cancelled by user.".to_string(),
                },
            );
            finalize_turn_change_capture(&state, thread_id, turn_id, TurnStatus::Cancelled).await;
            finalize_goal_after_turn(
                &state,
                thread_id,
                collaboration_mode,
                goal.as_ref().map(|goal| goal.id),
                TurnStatus::Cancelled,
            );
            finish_turn(
                &state,
                thread_id,
                turn_id,
                TurnStatus::Cancelled,
                None,
            );
            return;
        }
        prepared = prepare_turn_context(
            &state,
            thread_id,
            turn_id,
            user_message.id,
            context_reservation,
            &selected_provider,
        ) => prepared,
    };
    let prepared = match prepared_result {
        Ok(prepared) => prepared,
        Err(err) => {
            let message = err.message;
            publish_payload(
                &state,
                thread_id,
                Some(turn_id),
                AgentEventPayload::Error {
                    message: message.clone(),
                },
            );
            finalize_turn_change_capture(&state, thread_id, turn_id, TurnStatus::Failed).await;
            finalize_goal_after_turn(
                &state,
                thread_id,
                collaboration_mode,
                goal.as_ref().map(|goal| goal.id),
                TurnStatus::Failed,
            );
            finish_turn(
                &state,
                thread_id,
                turn_id,
                TurnStatus::Failed,
                Some(message),
            );
            return;
        }
    };
    if let Some(trace) = send_trace {
        trace.phase_with_count(
            "history_prepared",
            thread_id,
            Some(turn_id),
            prepared.conversation.len(),
        );
    }
    publish_payload(
        &state,
        thread_id,
        Some(turn_id),
        AgentEventPayload::ContextProjectionBuilt {
            projection: prepared.projection.clone(),
        },
    );
    let input = AgentTurnInput {
        thread_id,
        user_message_id: user_message.id,
        workspace_root: workspace_root.clone(),
        content,
        user_content,
        context_summary: prepared.summary,
        conversation: prepared.conversation,
        permission_mode: settings.permission_mode,
        context_budget: Some(prepared.budget),
        provider_cursor: match load_provider_cursor(
            &state.store,
            &selected_provider,
            thread_id,
            "/root",
        ) {
            Ok(taken) => {
                if let Some(invalidation) = taken.invalidation {
                    publish_payload(
                        &state,
                        thread_id,
                        Some(turn_id),
                        AgentEventPayload::ProviderContextStateInvalidated {
                            provider_id: Some(invalidation.provider_id),
                            model: Some(invalidation.model),
                            reason: invalidation.reason,
                        },
                    );
                }
                taken.cursor
            }
            Err(error) => {
                let message = format!("failed to load provider conversation state: {error}");
                publish_payload(
                    &state,
                    thread_id,
                    Some(turn_id),
                    AgentEventPayload::Error {
                        message: message.clone(),
                    },
                );
                finalize_turn_change_capture(&state, thread_id, turn_id, TurnStatus::Failed).await;
                finalize_goal_after_turn(
                    &state,
                    thread_id,
                    collaboration_mode,
                    goal.as_ref().map(|goal| goal.id),
                    TurnStatus::Failed,
                );
                finish_turn(
                    &state,
                    thread_id,
                    turn_id,
                    TurnStatus::Failed,
                    Some(message),
                );
                return;
            }
        },
        store: Some(state.store.clone()),
        cancellation: Some(turn.cancel.clone()),
    };

    if built_context.emit_thread_snapshot {
        publish_payload(
            &state,
            thread_id,
            Some(turn_id),
            AgentEventPayload::ThreadContextSnapshot {
                snapshot: built_context.thread_snapshot,
            },
        );
    }
    publish_payload(
        &state,
        thread_id,
        Some(turn_id),
        AgentEventPayload::TurnContextSnapshot {
            snapshot: built_context.turn_snapshot,
        },
    );
    let (sender, mut receiver) = mpsc::unbounded_channel();
    if let Some(trace) = send_trace {
        trace.phase("agent_drive_started", thread_id, Some(turn_id));
    }
    let future = drive_agent_turn(&agent, input, Some(built_context.context), Some(sender));
    tokio::pin!(future);
    let mut deferred_wait_events = Vec::new();

    let result = loop {
        tokio::select! {
            biased;
            _ = turn.cancel.cancelled() => {
                publish_payload(
                    &state,
                    thread_id,
                    Some(turn_id),
                    AgentEventPayload::TurnCancelled {
                        reason: "Cancelled by user.".to_string(),
                    },
                );
                let _ = timeout(Duration::from_secs(2), &mut future).await;
                loop {
                    let payloads = take_available_payload_batch(&mut receiver, None);
                    if payloads.is_empty() {
                        break;
                    }
                    persist_received_payload_batch(
                        &state,
                        &selected_provider,
                        thread_id,
                        turn_id,
                        payloads,
                        &mut deferred_wait_events,
                        true,
                    )
                    .await;
                }
                finalize_turn_change_capture(&state, thread_id, turn_id, TurnStatus::Cancelled)
                    .await;
                finalize_goal_after_turn(
                    &state,
                    thread_id,
                    collaboration_mode,
                    goal.as_ref().map(|goal| goal.id),
                    TurnStatus::Cancelled,
                );
                finish_turn(
                    &state,
                    thread_id,
                    turn_id,
                    TurnStatus::Cancelled,
                    None,
                );
                return;
            }
            result = &mut future => {
                if let Some(trace) = send_trace {
                    trace.phase("agent_drive_finished", thread_id, Some(turn_id));
                }
                break result;
            },
            payload = receiver.recv() => {
                if let Some(payload) = payload {
                    let payloads = take_available_payload_batch(&mut receiver, Some(payload));
                    persist_received_payload_batch(
                        &state,
                        &selected_provider,
                        thread_id,
                        turn_id,
                        payloads,
                        &mut deferred_wait_events,
                        false,
                    )
                    .await;
                }
            }
        }
    };
    loop {
        let payloads = take_available_payload_batch(&mut receiver, None);
        if payloads.is_empty() {
            break;
        }
        persist_received_payload_batch(
            &state,
            &selected_provider,
            thread_id,
            turn_id,
            payloads,
            &mut deferred_wait_events,
            false,
        )
        .await;
    }
    let approval_persistence =
        persist_deferred_approval_records(&state, thread_id, &deferred_wait_events);
    let continuation_persistence =
        persist_suspended_continuation(&state, thread_id, turn_id, &result);
    let provider_state_persistence = persist_provider_cursor(
        &state.store,
        &selected_provider,
        thread_id,
        "/root",
        &result,
    );
    if let Ok(Some(persisted)) = &provider_state_persistence {
        publish_persisted_provider_context(&state, thread_id, turn_id, persisted);
    }
    if approval_persistence.is_ok() && continuation_persistence.is_ok() {
        for payload in deferred_wait_events {
            publish_payload(&state, thread_id, Some(turn_id), payload);
        }
    } else {
        rollback_unpublished_wait_boundary(&state, thread_id, turn_id, &deferred_wait_events);
    }
    let (mut status, mut turn_error) = finish_agent_result(
        &state,
        thread_id,
        turn_id,
        &selected_provider.id,
        result,
        None,
    );
    if let Some(error) = approval_persistence
        .err()
        .or_else(|| continuation_persistence.err())
        .or_else(|| provider_state_persistence.err())
    {
        let message = error.to_string();
        publish_payload(
            &state,
            thread_id,
            Some(turn_id),
            AgentEventPayload::Error {
                message: message.clone(),
            },
        );
        status = TurnStatus::Failed;
        turn_error = Some(message);
    }
    finalize_turn_change_capture(&state, thread_id, turn_id, status).await;
    finalize_goal_after_turn(
        &state,
        thread_id,
        collaboration_mode,
        goal.as_ref().map(|goal| goal.id),
        status,
    );
    finish_turn(&state, thread_id, turn_id, status, turn_error);
}

async fn run_resumed_agent_turn(
    state: AppState,
    signal: AgentResumeSignal,
    continuation: AgentContinuation,
    turn: TurnHandle,
) {
    let thread_id = continuation.thread_id;
    let turn_id = turn.turn_id;
    let mut settings = current_settings(&state);
    let workspace_root = continuation.workspace_root.clone();
    let collaboration_mode = continuation.collaboration_mode;
    let goal = continuation.goal.clone();
    let _workspace_guard = state
        .turn_changes
        .lock_workspace_shared(&workspace_root)
        .await;
    resume_turn_change_capture(&state, thread_id, turn_id, &workspace_root).await;
    let prepared_draft =
        async {
            let authority = continuation.execution_authority.clone().ok_or_else(|| {
                anyhow::anyhow!("continuation is missing its execution authority")
            })?;
            authority.validate_workspace(&workspace_root)?;
            let thread = state
                .store
                .get_thread(thread_id)?
                .ok_or_else(|| anyhow::anyhow!("resume target task no longer exists"))?;
            let collaboration_turn = state
                .collaboration_repository
                .get_turn(AgentTurnId::from_uuid(turn_id))
                .await?;
            let collaboration_thread = state
                .collaboration_repository
                .get_thread(collaboration_turn.agent_thread_id)
                .await?;
            anyhow::ensure!(
                collaboration_thread.path == AgentPath::root(),
                "resumed product Turn is not bound to the root Agent"
            );
            let snapshot = state
                .collaboration_repository
                .get_runtime_snapshot(collaboration_thread.runtime_snapshot_id)
                .await?
                .decode()?;
            anyhow::ensure!(
                snapshot.workspace_root == workspace_root,
                "continuation workspace does not match the frozen runtime snapshot"
            );
            let frozen_provider: ProviderSettings = serde_json::from_value(
                snapshot
                    .provider
                    .ok_or_else(|| anyhow::anyhow!("runtime snapshot is missing provider"))?,
            )?;
            settings.active_provider_id = frozen_provider.id.clone();
            settings.providers = vec![frozen_provider.clone()];
            settings.permission_mode =
                serde_json::from_value(snapshot.permission_mode.ok_or_else(|| {
                    anyhow::anyhow!("runtime snapshot is missing permissionMode")
                })?)?;
            settings.sandbox = serde_json::from_value(
                snapshot
                    .sandbox
                    .ok_or_else(|| anyhow::anyhow!("runtime snapshot is missing sandbox"))?,
            )?;
            if let Some(agent_runtime) = snapshot.agent_runtime {
                settings.agent_runtime = serde_json::from_value(agent_runtime)?;
            }
            let frozen_projection: CapabilityProjection =
                serde_json::from_value(snapshot.capability_projection.ok_or_else(|| {
                    anyhow::anyhow!("runtime snapshot is missing capabilityProjection")
                })?)?;
            anyhow::ensure!(
                authority.permission_mode() == settings.permission_mode
                    && authority.sandbox_config() == &settings.sandbox.to_local_sandbox_config()
                    && authority.capability_projection() == &frozen_projection,
                "continuation authority does not match the frozen runtime snapshot"
            );
            let config = AgentRunConfig::from_settings(
                &settings,
                None,
                authority,
                AgentRunIdentity::root(turn_id, turn.invocation_id),
            )
            .with_experience_mode(thread.experience_mode)
            .with_collaboration_mode(collaboration_mode, goal.clone());
            let draft = state
                .agent
                .read()
                .expect("agent lock poisoned")
                .begin_run(config)?;
            let connection_authority = snapshot.connection_authority.clone();
            Ok::<_, anyhow::Error>((draft, frozen_provider, connection_authority))
        }
        .await;
    let (mut agent, selected_provider, connection_authority) = match prepared_draft {
        Ok(prepared) => prepared,
        Err(error) => {
            let message = error.to_string();
            publish_payload(
                &state,
                thread_id,
                Some(turn_id),
                AgentEventPayload::Error {
                    message: message.clone(),
                },
            );
            finalize_turn_change_capture(&state, thread_id, turn_id, TurnStatus::Failed).await;
            finalize_goal_after_turn(
                &state,
                thread_id,
                collaboration_mode,
                goal.as_ref().map(|goal| goal.id),
                TurnStatus::Failed,
            );
            finish_turn(
                &state,
                thread_id,
                turn_id,
                TurnStatus::Failed,
                Some(message),
            );
            return;
        }
    };
    if let Some(message) = state
        .provider_runtime_health
        .blocked_message(&selected_provider.id)
    {
        let message = message.to_string();
        publish_payload(
            &state,
            thread_id,
            Some(turn_id),
            AgentEventPayload::Error {
                message: message.clone(),
            },
        );
        finalize_turn_change_capture(&state, thread_id, turn_id, TurnStatus::Failed).await;
        finalize_goal_after_turn(
            &state,
            thread_id,
            collaboration_mode,
            goal.as_ref().map(|goal| goal.id),
            TurnStatus::Failed,
        );
        finish_turn(
            &state,
            thread_id,
            turn_id,
            TurnStatus::Failed,
            Some(message),
        );
        return;
    }
    agent.set_mcp_host(state.mcp_host.clone());
    if agent.capability_projection().allow_all_plugins
        || !agent.capability_projection().plugins.is_empty()
    {
        sync_thread_bundled_plugin_activations(&state.store, thread_id, &mut agent);
    } else {
        agent.disable_all_bundled_plugins();
    }
    sync_thread_attachment_tool_preloads(&state.store, thread_id, &mut agent);
    if let Err(error) = sync_runtime_connection_tools(
        &state.store,
        &state.mcp_host,
        thread_id,
        &connection_authority,
        &mut agent,
    )
    .await
    {
        let message = format!("failed to restore Connection operations: {error}");
        publish_payload(
            &state,
            thread_id,
            Some(turn_id),
            AgentEventPayload::Error {
                message: message.clone(),
            },
        );
        finalize_turn_change_capture(&state, thread_id, turn_id, TurnStatus::Failed).await;
        finish_turn(
            &state,
            thread_id,
            turn_id,
            TurnStatus::Failed,
            Some(message),
        );
        return;
    }
    /* authority-bound setup ends here; no live setting may widen the draft */
    let library_provider = state
        .store
        .list_messages(thread_id)
        .ok()
        .and_then(|messages| {
            messages
                .into_iter()
                .find(|message| message.id == continuation.user_message_id)
        })
        .as_ref()
        .and_then(message_library_provider);
    let resumed_thread = match state.store.get_thread(thread_id) {
        Ok(Some(thread)) => thread,
        Ok(None) => {
            let message =
                "failed to restore bound Agent knowledge scope: task no longer exists".to_string();
            publish_payload(
                &state,
                thread_id,
                Some(turn_id),
                AgentEventPayload::Error {
                    message: message.clone(),
                },
            );
            finalize_turn_change_capture(&state, thread_id, turn_id, TurnStatus::Failed).await;
            finish_turn(
                &state,
                thread_id,
                turn_id,
                TurnStatus::Failed,
                Some(message),
            );
            return;
        }
        Err(error) => {
            let message = format!("failed to restore bound Agent knowledge scope: {error}");
            publish_payload(
                &state,
                thread_id,
                Some(turn_id),
                AgentEventPayload::Error {
                    message: message.clone(),
                },
            );
            finalize_turn_change_capture(&state, thread_id, turn_id, TurnStatus::Failed).await;
            finish_turn(
                &state,
                thread_id,
                turn_id,
                TurnStatus::Failed,
                Some(message),
            );
            return;
        }
    };
    let resumed_bound_agent = match load_bound_agent_context(&state, &resumed_thread) {
        Ok((instance, _)) => instance,
        Err(error) => {
            let message = format!(
                "failed to restore bound Agent knowledge scope: {}",
                error.message
            );
            publish_payload(
                &state,
                thread_id,
                Some(turn_id),
                AgentEventPayload::Error {
                    message: message.clone(),
                },
            );
            finalize_turn_change_capture(&state, thread_id, turn_id, TurnStatus::Failed).await;
            finish_turn(
                &state,
                thread_id,
                turn_id,
                TurnStatus::Failed,
                Some(message),
            );
            return;
        }
    };
    if let Some(binding) = resumed_bound_agent
        .as_ref()
        .and_then(|instance| instance.execution_context.knowledge_binding.as_ref())
    {
        let namespaces = binding.namespaces.iter().cloned().collect::<Vec<_>>();
        agent.set_library_namespaces(namespaces.clone());
        agent.register_runtime_tool(Arc::new(library_api::LibrarySearchTool::scoped(
            state.library_providers.clone(),
            namespaces,
        )));
    } else if let Some(provider) = library_provider {
        agent.register_runtime_tool(Arc::new(library_api::LibrarySearchTool::new(
            state.library_providers.clone(),
            provider,
        )));
    }
    if let Err(error) =
        bind_existing_collaboration_turn(&state, turn_id, turn.invocation_id, &mut agent).await
    {
        let message = format!("failed to resume collaboration Agent Turn: {error}");
        publish_payload(
            &state,
            thread_id,
            Some(turn_id),
            AgentEventPayload::Error {
                message: message.clone(),
            },
        );
        finalize_turn_change_capture(&state, thread_id, turn_id, TurnStatus::Failed).await;
        finish_turn(
            &state,
            thread_id,
            turn_id,
            TurnStatus::Failed,
            Some(message),
        );
        return;
    }
    let agent = match agent.finalize() {
        Ok(agent) => agent,
        Err(error) => {
            let message = format!("failed to prepare resumed Agent Turn: {error}");
            publish_payload(
                &state,
                thread_id,
                Some(turn_id),
                AgentEventPayload::Error {
                    message: message.clone(),
                },
            );
            finalize_turn_change_capture(&state, thread_id, turn_id, TurnStatus::Failed).await;
            finish_turn(
                &state,
                thread_id,
                turn_id,
                TurnStatus::Failed,
                Some(message),
            );
            return;
        }
    };
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let resolved_approval_id = match &signal {
        AgentResumeSignal::Approval { approval_id, .. } => *approval_id,
        AgentResumeSignal::UserInput { .. } | AgentResumeSignal::ExternalAction { .. } => None,
    };
    let future = resume_agent_turn(
        &agent,
        continuation,
        signal,
        Some(state.store.clone()),
        Some(turn.cancel.clone()),
        Some(sender),
    );
    tokio::pin!(future);
    let mut deferred_wait_events = Vec::new();

    let result = loop {
        tokio::select! {
            biased;
            _ = turn.cancel.cancelled() => {
                publish_payload(
                    &state,
                    thread_id,
                    Some(turn_id),
                    AgentEventPayload::TurnCancelled {
                        reason: "Cancelled by user.".to_string(),
                    },
                );
                let _ = timeout(Duration::from_secs(2), &mut future).await;
                loop {
                    let payloads = take_available_payload_batch(&mut receiver, None);
                    if payloads.is_empty() {
                        break;
                    }
                    persist_received_payload_batch(
                        &state,
                        &selected_provider,
                        thread_id,
                        turn_id,
                        payloads,
                        &mut deferred_wait_events,
                        true,
                    )
                    .await;
                }
                finalize_turn_change_capture(&state, thread_id, turn_id, TurnStatus::Cancelled)
                    .await;
                finalize_goal_after_turn(
                    &state,
                    thread_id,
                    collaboration_mode,
                    goal.as_ref().map(|goal| goal.id),
                    TurnStatus::Cancelled,
                );
                finish_turn(
                    &state,
                    thread_id,
                    turn_id,
                    TurnStatus::Cancelled,
                    None,
                );
                return;
            }
            result = &mut future => break result,
            payload = receiver.recv() => {
                if let Some(payload) = payload {
                    let payloads = take_available_payload_batch(&mut receiver, Some(payload));
                    persist_received_payload_batch(
                        &state,
                        &selected_provider,
                        thread_id,
                        turn_id,
                        payloads,
                        &mut deferred_wait_events,
                        false,
                    )
                    .await;
                }
            }
        }
    };
    loop {
        let payloads = take_available_payload_batch(&mut receiver, None);
        if payloads.is_empty() {
            break;
        }
        persist_received_payload_batch(
            &state,
            &selected_provider,
            thread_id,
            turn_id,
            payloads,
            &mut deferred_wait_events,
            false,
        )
        .await;
    }
    let approval_persistence =
        persist_deferred_approval_records(&state, thread_id, &deferred_wait_events);
    let continuation_persistence =
        persist_suspended_continuation(&state, thread_id, turn_id, &result);
    let provider_state_persistence = persist_provider_cursor(
        &state.store,
        &selected_provider,
        thread_id,
        "/root",
        &result,
    );
    if let Ok(Some(persisted)) = &provider_state_persistence {
        publish_persisted_provider_context(&state, thread_id, turn_id, persisted);
    }
    if approval_persistence.is_ok() && continuation_persistence.is_ok() {
        for payload in deferred_wait_events {
            publish_payload(&state, thread_id, Some(turn_id), payload);
        }
    } else {
        rollback_unpublished_wait_boundary(&state, thread_id, turn_id, &deferred_wait_events);
    }
    let (mut status, mut turn_error) = finish_agent_result(
        &state,
        thread_id,
        turn_id,
        &selected_provider.id,
        result,
        resolved_approval_id,
    );
    if let Some(error) = approval_persistence
        .err()
        .or_else(|| continuation_persistence.err())
        .or_else(|| provider_state_persistence.err())
    {
        let message = error.to_string();
        publish_payload(
            &state,
            thread_id,
            Some(turn_id),
            AgentEventPayload::Error {
                message: message.clone(),
            },
        );
        status = TurnStatus::Failed;
        turn_error = Some(message);
    }
    finalize_turn_change_capture(&state, thread_id, turn_id, status).await;
    finalize_goal_after_turn(
        &state,
        thread_id,
        collaboration_mode,
        goal.as_ref().map(|goal| goal.id),
        status,
    );
    finish_turn(&state, thread_id, turn_id, status, turn_error);
}

fn finish_agent_result(
    state: &AppState,
    thread_id: Uuid,
    turn_id: Uuid,
    provider_id: &str,
    result: anyhow::Result<opentopia_core::AgentTurnResult>,
    resolved_approval_id: Option<Uuid>,
) -> (TurnStatus, Option<String>) {
    let (mut status, mut turn_error) = match result {
        Ok(result) => match result.outcome {
            AgentTurnOutcome::Completed => (TurnStatus::Succeeded, None),
            AgentTurnOutcome::Cancelled { .. } => (TurnStatus::Cancelled, None),
            AgentTurnOutcome::Partial { .. } | AgentTurnOutcome::Blocked { .. } => {
                // Partial and Blocked are semantic WorkForm outcomes, not
                // runtime failures. Goal state remains owned by its WorkForm.
                (TurnStatus::Succeeded, None)
            }
            AgentTurnOutcome::Stopped { reason } => (TurnStatus::Failed, Some(reason)),
            AgentTurnOutcome::Suspended { .. } => (TurnStatus::WaitingApproval, None),
            AgentTurnOutcome::AwaitingInput { .. } => (TurnStatus::WaitingUserInput, None),
            AgentTurnOutcome::WaitingUserAction { .. } => (TurnStatus::WaitingUserAction, None),
        },
        Err(err) => {
            let message = if provider_failure_is_quota_exhausted(&err) {
                warn!(%provider_id, "blocking provider after permanent quota failure");
                state.provider_runtime_health.block_for_quota(provider_id);
                QUOTA_EXHAUSTED_MESSAGE.to_string()
            } else {
                err.to_string()
            };
            publish_payload(
                state,
                thread_id,
                Some(turn_id),
                AgentEventPayload::Error {
                    message: message.clone(),
                },
            );
            (TurnStatus::Failed, Some(message))
        }
    };
    if let Some(approval_id) = resolved_approval_id {
        if let Err(err) = state
            .store
            .delete_approval_continuation(approval_id, thread_id)
        {
            error!(?err, %approval_id, "failed to remove resolved continuation");
            let message = format!("failed to remove resolved approval continuation: {err}");
            publish_payload(
                state,
                thread_id,
                Some(turn_id),
                AgentEventPayload::Error {
                    message: message.clone(),
                },
            );
            status = TurnStatus::Failed;
            turn_error = Some(message);
        }
    }
    (status, turn_error)
}

fn provider_state_enabled(provider: &ProviderSettings) -> bool {
    let capabilities = provider.capabilities();
    provider.effective_transport() == ProviderTransportKind::Http
        && (provider.resolved_adapter_for_model(&provider.model) == ProviderAdapterKind::OpenAiChat
            || capabilities.supports_response_state
            || capabilities.supports_native_compaction)
}

struct TakenProviderCursor {
    cursor: Option<ProviderConversationCursor>,
    invalidation: Option<ProviderStateInvalidation>,
}

struct ProviderStateInvalidation {
    provider_id: String,
    model: String,
    reason: String,
}

fn load_provider_cursor(
    store: &SqliteSessionStore,
    provider: &ProviderSettings,
    thread_id: Uuid,
    agent_path: &str,
) -> anyhow::Result<TakenProviderCursor> {
    // Loading is intentionally non-destructive. A cancelled or failed turn has
    // no successor cursor to save; deleting the prior state up front would make
    // Chat lose replay-critical assistant reasoning and grouping permanently.
    // Successful turns overwrite this record atomically at the storage boundary.
    let state = store.get_provider_conversation_state(thread_id, agent_path)?;
    let Some(state) = state else {
        return Ok(TakenProviderCursor {
            cursor: None,
            invalidation: None,
        });
    };
    if !provider_state_enabled(provider) {
        store.clear_provider_conversation_state(thread_id, agent_path)?;
        return Ok(TakenProviderCursor {
            cursor: None,
            invalidation: Some(ProviderStateInvalidation {
                provider_id: state.provider_id,
                model: state.model,
                reason: "active provider protocol does not support persisted response state; rebuilt from the local checkpoint and recent history".to_string(),
            }),
        });
    }
    let adapter_identity = provider.resolved_route().adapter_identity();
    if state.provider_id != provider.id
        || state.model != provider.model
        || (!state.adapter_identity.is_empty() && state.adapter_identity != adapter_identity)
    {
        store.clear_provider_conversation_state(thread_id, agent_path)?;
        return Ok(TakenProviderCursor {
            cursor: None,
            invalidation: Some(ProviderStateInvalidation {
                provider_id: state.provider_id,
                model: state.model,
                reason: format!(
                    "provider, model, or adapter changed to '{}'/'{}'/{}; rebuilt from the local checkpoint and recent history",
                    provider.id,
                    provider.model,
                    adapter_identity
                ),
            }),
        });
    }
    Ok(TakenProviderCursor {
        cursor: Some(ProviderConversationCursor {
            response_id: state.response_id,
            compatibility_hash: state.compatibility_hash,
            response_items: state.response_items,
            state_kind: state.state_kind,
            compaction_item_count: state.compaction_item_count,
        }),
        invalidation: None,
    })
}

fn persist_provider_request_checkpoints(
    store: &SqliteSessionStore,
    provider: &ProviderSettings,
    thread_id: Uuid,
    agent_path: &str,
    payloads: &mut [AgentEventPayload],
) -> anyhow::Result<()> {
    for payload in payloads {
        if !matches!(payload, AgentEventPayload::ProviderRequestSent { .. }) {
            continue;
        }
        let Some(checkpoint) = payload.take_provider_request_checkpoint() else {
            continue;
        };
        if !provider_state_enabled(provider) {
            continue;
        }
        let cursor = ProviderConversationCursor::from_request_checkpoint(checkpoint);
        let adapter_identity = provider.resolved_route().adapter_identity();
        let checkpoint_id = store
            .get_provider_conversation_state(thread_id, agent_path)?
            .filter(|state| {
                state.provider_id == provider.id
                    && state.model == provider.model
                    && (state.adapter_identity.is_empty()
                        || state.adapter_identity == adapter_identity)
            })
            .and_then(|state| state.checkpoint_id);
        store.save_provider_conversation_state(&ProviderConversationState {
            thread_id,
            agent_path: agent_path.to_string(),
            provider_id: provider.id.clone(),
            model: provider.model.clone(),
            adapter_identity: adapter_identity.clone(),
            response_id: cursor.response_id,
            compatibility_hash: cursor.compatibility_hash,
            response_items: cursor.response_items,
            state_kind: cursor.state_kind,
            compaction_item_count: cursor.compaction_item_count,
            checkpoint_id,
            updated_at: Utc::now(),
        })?;
    }
    Ok(())
}

fn persist_provider_cursor(
    store: &SqliteSessionStore,
    provider: &ProviderSettings,
    thread_id: Uuid,
    agent_path: &str,
    result: &anyhow::Result<opentopia_core::AgentTurnResult>,
) -> anyhow::Result<Option<PersistedProviderCursor>> {
    if !provider_state_enabled(provider) {
        return Ok(None);
    }
    let Ok(result) = result else {
        return Ok(None);
    };
    let Some(cursor) = result.provider_cursor.as_ref() else {
        return Ok(None);
    };
    if provider_cursor_misses_async_result(store, thread_id, agent_path)? {
        store.clear_provider_conversation_state(thread_id, agent_path)?;
        return Ok(None);
    }
    let native_checkpoint = build_native_provider_checkpoint(store, provider, thread_id, cursor)?;
    let checkpoint_id = native_checkpoint
        .as_ref()
        .and_then(|summary| summary.checkpoint.as_ref())
        .map(|checkpoint| checkpoint.id)
        .or_else(|| {
            store
                .list_events(thread_id, None)
                .ok()
                .and_then(|events| latest_context_summary_event(&events))
                .and_then(|summary| summary.checkpoint.map(|checkpoint| checkpoint.id))
        });
    let state = ProviderConversationState {
        thread_id,
        agent_path: agent_path.to_string(),
        provider_id: provider.id.clone(),
        model: provider.model.clone(),
        adapter_identity: provider.resolved_route().adapter_identity(),
        response_id: cursor.response_id.clone(),
        compatibility_hash: cursor.compatibility_hash.clone(),
        response_items: cursor.response_items.clone(),
        state_kind: cursor.state_kind.clone(),
        compaction_item_count: cursor.compaction_item_count,
        checkpoint_id,
        updated_at: Utc::now(),
    };
    store.save_provider_conversation_state(&state)?;
    // The background completion can race the save above. Its sink clears the
    // previous state, and this post-save check prevents us from immediately
    // restoring a cursor that predates the appended async result.
    if provider_cursor_misses_async_result(store, thread_id, agent_path)? {
        store.clear_provider_conversation_state(thread_id, agent_path)?;
        return Ok(None);
    }
    Ok(Some(PersistedProviderCursor {
        state,
        native_checkpoint,
    }))
}

fn provider_cursor_misses_async_result(
    store: &SqliteSessionStore,
    thread_id: Uuid,
    agent_path: &str,
) -> anyhow::Result<bool> {
    let events = store.list_events(thread_id, None)?;
    let latest_model_request_seq = events
        .iter()
        .filter(|event| matches!(&event.payload, AgentEventPayload::ModelRequest { .. }))
        .map(|event| event.seq)
        .max();
    let latest_async_result_seq = events
        .iter()
        .filter(|event| {
            let AgentEventPayload::ToolCallFinished { result } = &event.payload else {
                return false;
            };
            result
                .metadata
                .get("asyncToolResult")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                && result.metadata.get("agentPath").and_then(Value::as_str) == Some(agent_path)
        })
        .map(|event| event.seq)
        .max();
    Ok(latest_async_result_seq
        .is_some_and(|async_seq| latest_model_request_seq.map_or(true, |seq| async_seq > seq)))
}

struct PersistedProviderCursor {
    state: ProviderConversationState,
    native_checkpoint: Option<ContextSummary>,
}

fn publish_persisted_provider_context(
    app_state: &AppState,
    thread_id: Uuid,
    turn_id: Uuid,
    persisted: &PersistedProviderCursor,
) {
    if let Some(summary) = &persisted.native_checkpoint {
        publish_payload(
            app_state,
            thread_id,
            Some(turn_id),
            AgentEventPayload::ContextCompacted {
                summary: summary.clone(),
                details: Some(context_compaction_details(app_state, thread_id, summary)),
            },
        );
    }
    let provider_state = &persisted.state;
    publish_payload(
        app_state,
        thread_id,
        Some(turn_id),
        AgentEventPayload::ProviderContextStateUpdated {
            provider_id: provider_state.provider_id.clone(),
            model: provider_state.model.clone(),
            state_kind: provider_state.state_kind.as_str().to_string(),
            response_item_count: provider_state.response_items.len(),
            compaction_item_count: provider_state.compaction_item_count,
        },
    );
}

fn build_native_provider_checkpoint(
    store: &SqliteSessionStore,
    provider: &ProviderSettings,
    thread_id: Uuid,
    cursor: &ProviderConversationCursor,
) -> anyhow::Result<Option<ContextSummary>> {
    if cursor.compaction_item_count == 0 {
        return Ok(None);
    }
    let events = store.list_events(thread_id, None)?;
    let previous_summary = latest_context_summary_event(&events);
    let provider_state_fingerprint = content_fingerprint(&serde_json::to_vec(&(
        &cursor.response_id,
        &cursor.response_items,
        &cursor.compatibility_hash,
    ))?);
    let prior_native_fingerprint = previous_summary
        .as_ref()
        .and_then(|summary| summary.metadata.get("nativeProviderStateFingerprint"))
        .and_then(Value::as_str);
    if prior_native_fingerprint == Some(provider_state_fingerprint.as_str()) {
        return Ok(None);
    }

    let messages = store.list_messages(thread_id)?;
    let previous_checkpoint = previous_summary
        .as_ref()
        .and_then(|summary| summary.checkpoint.as_ref());
    let fallback_goal = previous_summary
        .as_ref()
        .map(|summary| summary.summary.clone())
        .or_else(|| {
            messages
                .iter()
                .rev()
                .find(|message| message.role == MessageRole::User)
                .map(render_message_for_summary)
        })
        .unwrap_or_else(|| "Continue the active thread from durable local history.".to_string());
    let fallback_coverage = previous_summary
        .as_ref()
        .map(|summary| ContextCheckpointCoverage {
            through_seq: summary.covered_through_seq,
            through_message_count: summary_message_cursor(summary),
        })
        .unwrap_or_default();
    let mut checkpoint = previous_checkpoint
        .cloned()
        .unwrap_or_else(|| ContextCheckpoint::manual(thread_id, fallback_coverage, fallback_goal));
    checkpoint.previous_checkpoint_id = previous_checkpoint.map(|checkpoint| checkpoint.id);
    checkpoint.id = Uuid::new_v4();
    checkpoint.mode = ContextCheckpointMode::NativeProvider;
    checkpoint.provider_compatibility_hash = Some(cursor.compatibility_hash.clone());
    checkpoint.created_at = Utc::now();
    let checkpoint_tokens = checkpoint_token_estimate(&checkpoint);
    let mut summary = ContextSummary::new(
        thread_id,
        checkpoint.coverage.through_seq,
        checkpoint.coverage.through_message_count,
        render_context_checkpoint(&checkpoint),
    );
    summary.token_estimate = Some(checkpoint_tokens);
    summary.metadata = json!({
        "mode": "native_provider",
        "source": "provider_native_compaction",
        "providerId": provider.id,
        "model": provider.model,
        "checkpointId": checkpoint.id,
        "checkpointTokens": checkpoint_tokens,
        "inputTokens": 0,
        "tokenReductionPercent": 0,
        "latencyMs": 0,
        "factRetentionPercent": 100,
        "activeConstraintRetentionPercent": 100,
        "nativeCompactionItemCount": cursor.compaction_item_count,
        "nativeProviderStateFingerprint": provider_state_fingerprint,
        "providerResponseIdPresent": !cursor.response_id.is_empty(),
        "observedThroughSeq": events.last().map(|event| event.seq).unwrap_or_default(),
        "coveredThroughSeq": checkpoint.coverage.through_seq,
        "coveredMessageCount": checkpoint.coverage.through_message_count,
    });
    summary.checkpoint = Some(checkpoint);
    Ok(Some(summary))
}

fn persist_suspended_continuation(
    state: &AppState,
    thread_id: Uuid,
    turn_id: Uuid,
    result: &anyhow::Result<opentopia_core::AgentTurnResult>,
) -> anyhow::Result<()> {
    let Ok(result) = result else {
        return Ok(());
    };
    match &result.outcome {
        AgentTurnOutcome::Suspended {
            approval_id,
            continuation,
        } => {
            let value = encode_turn_checkpoint(&state.store, "approval", continuation)?;
            state
                .store
                .put_turn_checkpoint(turn_id, thread_id, "approval", value.clone())?;
            state
                .store
                .put_approval_continuation(*approval_id, thread_id, value)
                .with_context(|| format!("failed to persist approval continuation {approval_id}"))
        }
        AgentTurnOutcome::AwaitingInput {
            request,
            continuation,
        } => {
            let value = encode_turn_checkpoint(&state.store, "user_input", continuation)?;
            state
                .store
                .put_turn_checkpoint(turn_id, thread_id, "user_input", value.clone())?;
            state
                .store
                .put_user_input_request(thread_id, request, value)
                .with_context(|| {
                    format!(
                        "failed to persist user input continuation {}",
                        request.request_id
                    )
                })?;
            Ok(())
        }
        AgentTurnOutcome::WaitingUserAction { continuation, .. } => {
            let value = encode_turn_checkpoint(&state.store, "external_action", continuation)?;
            state
                .store
                .put_turn_checkpoint(turn_id, thread_id, "external_action", value)
                .context("failed to persist external-action continuation")
        }
        _ => {
            state.store.delete_turn_checkpoint(turn_id, thread_id)?;
            Ok(())
        }
    }
}

const DURABLE_TURN_CHECKPOINT_VERSION: u8 = 1;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DurableTurnCheckpoint {
    schema_version: u8,
    turn_id: Uuid,
    invocation_id: u64,
    phase: String,
    round: usize,
    pending_call_ids: Vec<String>,
    runtime_snapshot_id: String,
    context_epoch_id: String,
    ledger_cursor: Uuid,
    conversation_ref: String,
    model_context_ref: String,
    tool_catalog_ref: String,
    continuation: AgentContinuation,
}

fn encode_turn_checkpoint(
    store: &Arc<SqliteSessionStore>,
    phase: &str,
    continuation: &AgentContinuation,
) -> anyhow::Result<Value> {
    let mut continuation = continuation.clone();
    let conversation = std::mem::take(&mut continuation.conversation);
    let model_context = std::mem::take(&mut continuation.model_context);
    let (round, pending_call_ids, tool_catalog) = match &mut continuation.state {
        AgentContinuationState::Provider {
            model_rounds,
            pending_tool_calls,
            tool_candidates,
            ..
        } => (
            *model_rounds,
            pending_tool_calls
                .iter()
                .map(|call| call.id.clone())
                .collect::<Vec<_>>(),
            std::mem::take(tool_candidates),
        ),
    };
    let context_epoch_id = model_context
        .prompt_cache_key
        .clone()
        .unwrap_or_else(|| model_context.content_hash());
    let conversation_ref =
        store.put_turn_checkpoint_blob("conversation", serde_json::to_value(conversation)?)?;
    let model_context_ref =
        store.put_turn_checkpoint_blob("model_context", serde_json::to_value(model_context)?)?;
    let tool_catalog_ref =
        store.put_turn_checkpoint_blob("tool_catalog", serde_json::to_value(tool_catalog)?)?;
    let runtime_snapshot_id = content_fingerprint(
        format!("{conversation_ref}\0{model_context_ref}\0{tool_catalog_ref}").as_bytes(),
    );
    serde_json::to_value(DurableTurnCheckpoint {
        schema_version: DURABLE_TURN_CHECKPOINT_VERSION,
        turn_id: continuation.turn_id,
        invocation_id: continuation.invocation_id,
        phase: phase.to_string(),
        round,
        pending_call_ids,
        runtime_snapshot_id,
        context_epoch_id,
        ledger_cursor: continuation.user_message_id,
        conversation_ref,
        model_context_ref,
        tool_catalog_ref,
        continuation,
    })
    .map_err(Into::into)
}

fn decode_turn_checkpoint(
    store: &Arc<SqliteSessionStore>,
    expected_phase: &str,
    value: Value,
) -> anyhow::Result<AgentContinuation> {
    if value.get("schemaVersion").is_none() {
        // Compatibility with pre-reference checkpoints.
        return serde_json::from_value(value).map_err(Into::into);
    }
    let checkpoint: DurableTurnCheckpoint = serde_json::from_value(value)?;
    anyhow::ensure!(
        checkpoint.schema_version == DURABLE_TURN_CHECKPOINT_VERSION,
        "unsupported turn checkpoint schema version: {}",
        checkpoint.schema_version
    );
    anyhow::ensure!(
        checkpoint.phase == expected_phase,
        "turn checkpoint phase mismatch: expected {expected_phase}, found {}",
        checkpoint.phase
    );
    let expected_runtime_snapshot_id = content_fingerprint(
        format!(
            "{}\0{}\0{}",
            checkpoint.conversation_ref, checkpoint.model_context_ref, checkpoint.tool_catalog_ref
        )
        .as_bytes(),
    );
    anyhow::ensure!(
        checkpoint.runtime_snapshot_id == expected_runtime_snapshot_id,
        "turn checkpoint runtime snapshot identity mismatch"
    );
    let mut continuation = checkpoint.continuation;
    anyhow::ensure!(
        continuation.turn_id == checkpoint.turn_id
            && continuation.invocation_id == checkpoint.invocation_id
            && continuation.user_message_id == checkpoint.ledger_cursor,
        "turn checkpoint control identity mismatch"
    );
    continuation.conversation = serde_json::from_value(
        store
            .get_turn_checkpoint_blob(&checkpoint.conversation_ref)?
            .with_context(|| {
                format!(
                    "missing checkpoint conversation blob {}",
                    checkpoint.conversation_ref
                )
            })?,
    )?;
    let model_context: CompiledModelContext = serde_json::from_value(
        store
            .get_turn_checkpoint_blob(&checkpoint.model_context_ref)?
            .with_context(|| {
                format!(
                    "missing checkpoint model-context blob {}",
                    checkpoint.model_context_ref
                )
            })?,
    )?;
    let restored_context_epoch_id = model_context
        .prompt_cache_key
        .clone()
        .unwrap_or_else(|| model_context.content_hash());
    anyhow::ensure!(
        checkpoint.context_epoch_id == restored_context_epoch_id,
        "turn checkpoint context epoch identity mismatch"
    );
    continuation.model_context = model_context;
    let tool_catalog = serde_json::from_value(
        store
            .get_turn_checkpoint_blob(&checkpoint.tool_catalog_ref)?
            .with_context(|| {
                format!(
                    "missing checkpoint tool-catalog blob {}",
                    checkpoint.tool_catalog_ref
                )
            })?,
    )?;
    match &mut continuation.state {
        AgentContinuationState::Provider {
            model_rounds,
            pending_tool_calls,
            tool_candidates,
            ..
        } => {
            anyhow::ensure!(
                *model_rounds == checkpoint.round,
                "turn checkpoint model round mismatch"
            );
            let pending_call_ids = pending_tool_calls
                .iter()
                .map(|call| call.id.clone())
                .collect::<Vec<_>>();
            anyhow::ensure!(
                pending_call_ids == checkpoint.pending_call_ids,
                "turn checkpoint pending call identity mismatch"
            );
            *tool_candidates = tool_catalog;
        }
    }
    Ok(continuation)
}

fn persist_and_publish_payloads(
    state: &AppState,
    thread_id: Uuid,
    turn_id: Uuid,
    payloads: Vec<AgentEventPayload>,
) {
    let mut projected_payloads = Vec::with_capacity(payloads.len());
    let mut conversation_messages = Vec::new();
    for payload in payloads {
        if let AgentEventPayload::AssistantMessage { message } = &payload {
            conversation_messages.push(message.clone());
        }
        let tool_message = match &payload {
            AgentEventPayload::ToolCallStarted { call } => Some(Message {
                id: Uuid::new_v4(),
                thread_id,
                role: MessageRole::Tool,
                parts: vec![MessagePart::ToolCall { call: call.clone() }],
                created_at: Utc::now(),
            }),
            AgentEventPayload::ToolCallFinished { result } => Some(Message {
                id: Uuid::new_v4(),
                thread_id,
                role: MessageRole::Tool,
                parts: vec![MessagePart::ToolResult {
                    result: result.clone(),
                }],
                created_at: Utc::now(),
            }),
            _ => None,
        };
        if let Some(message) = tool_message {
            conversation_messages.push(message);
        }
        if let AgentEventPayload::ApprovalRequested {
            approval_id,
            action,
            reason,
        } = &payload
        {
            let approval =
                Approval::pending(*approval_id, thread_id, action.clone(), reason.clone());
            if let Err(err) = state.store.insert_approval(approval) {
                error!(?err, %approval_id, "failed to persist approval request");
            }
        }
        let goal_projection = match &payload {
            AgentEventPayload::WorkFormUpdated { form } => match form.scope {
                opentopia_core::WorkScope::Goal(goal_id) => {
                    state.store.get_goal(goal_id).ok().flatten()
                }
                opentopia_core::WorkScope::Turn(_) => None,
            },
            AgentEventPayload::TokenUsage { total_tokens, .. } => state
                .store
                .get_thread_goal(thread_id)
                .ok()
                .flatten()
                .filter(|snapshot| !snapshot.status().is_terminal())
                .and_then(|snapshot| {
                    state
                        .store
                        .add_goal_usage(snapshot.goal.id, *total_tokens as u64, 0)
                        .ok()
                        .flatten()
                }),
            _ => None,
        };
        projected_payloads.push(payload);
        if let Some(snapshot) = goal_projection {
            projected_payloads.push(AgentEventPayload::GoalUpdated { snapshot });
        }
    }
    publish_payloads_with_messages(
        state,
        thread_id,
        Some(turn_id),
        conversation_messages,
        projected_payloads,
    );
}

const EVENT_PERSIST_BATCH_SIZE: usize = 256;
const STREAM_EVENT_CHUNK_BYTES: usize = 8 * 1024;

fn take_available_payload_batch(
    receiver: &mut mpsc::UnboundedReceiver<AgentEventPayload>,
    first: Option<AgentEventPayload>,
) -> Vec<AgentEventPayload> {
    let mut payloads = Vec::with_capacity(EVENT_PERSIST_BATCH_SIZE);
    if let Some(first) = first {
        payloads.push(first);
    }
    while payloads.len() < EVENT_PERSIST_BATCH_SIZE {
        let Ok(payload) = receiver.try_recv() else {
            break;
        };
        payloads.push(payload);
    }
    payloads
}

async fn persist_received_payload_batch(
    state: &AppState,
    provider: &ProviderSettings,
    thread_id: Uuid,
    turn_id: Uuid,
    payloads: Vec<AgentEventPayload>,
    deferred_wait_events: &mut Vec<AgentEventPayload>,
    discard_stream_deltas: bool,
) {
    let mut durable_payloads = Vec::with_capacity(payloads.len());
    for payload in payloads {
        if discard_stream_deltas
            && matches!(
                payload,
                AgentEventPayload::ModelDelta { .. } | AgentEventPayload::ReasoningDelta { .. }
            )
        {
            continue;
        }
        if is_wait_boundary(&payload) {
            deferred_wait_events.push(payload);
        } else {
            durable_payloads.push(payload);
        }
    }
    let mut durable_payloads = compact_stream_payload_batch(durable_payloads);
    if durable_payloads.is_empty() {
        return;
    }
    let state = state.clone();
    let provider = provider.clone();
    if let Err(error) = tokio::task::spawn_blocking(move || {
        if let Err(error) = persist_provider_request_checkpoints(
            &state.store,
            &provider,
            thread_id,
            "/root",
            &mut durable_payloads,
        ) {
            error!(?error, %thread_id, %turn_id, "failed to persist provider request checkpoint");
        }
        persist_and_publish_payloads(&state, thread_id, turn_id, durable_payloads);
    })
    .await
    {
        error!(?error, %thread_id, %turn_id, "event persistence worker failed");
    }
}

fn compact_stream_payload_batch(payloads: Vec<AgentEventPayload>) -> Vec<AgentEventPayload> {
    let mut compacted = Vec::with_capacity(payloads.len());
    for payload in payloads {
        let merged = match (compacted.last_mut(), &payload) {
            (
                Some(AgentEventPayload::ModelDelta { text: current }),
                AgentEventPayload::ModelDelta { text },
            ) if current.len().saturating_add(text.len()) <= STREAM_EVENT_CHUNK_BYTES => {
                current.push_str(text);
                true
            }
            (
                Some(AgentEventPayload::ReasoningDelta { text: current }),
                AgentEventPayload::ReasoningDelta { text },
            ) if current.len().saturating_add(text.len()) <= STREAM_EVENT_CHUNK_BYTES => {
                current.push_str(text);
                true
            }
            _ => false,
        };
        if !merged {
            compacted.push(payload);
        }
    }
    compacted
}

fn is_wait_boundary(payload: &AgentEventPayload) -> bool {
    matches!(
        payload,
        AgentEventPayload::ApprovalRequested { .. }
            | AgentEventPayload::TurnSuspended { .. }
            | AgentEventPayload::UserInputRequested { .. }
            | AgentEventPayload::TurnAwaitingInput { .. }
            | AgentEventPayload::BrowserHandoffRequired { .. }
    )
}

fn persist_deferred_approval_records(
    state: &AppState,
    thread_id: Uuid,
    payloads: &[AgentEventPayload],
) -> anyhow::Result<()> {
    for payload in payloads {
        let AgentEventPayload::ApprovalRequested {
            approval_id,
            action,
            reason,
        } = payload
        else {
            continue;
        };
        let approval = Approval::pending(*approval_id, thread_id, action.clone(), reason.clone());
        state
            .store
            .insert_approval(approval)
            .with_context(|| format!("failed to persist approval request {approval_id}"))?;
    }
    Ok(())
}

fn rollback_unpublished_wait_boundary(
    state: &AppState,
    thread_id: Uuid,
    turn_id: Uuid,
    payloads: &[AgentEventPayload],
) {
    if let Err(error) = state.store.delete_turn_checkpoint(turn_id, thread_id) {
        warn!(?error, %turn_id, "failed to remove an unpublished wait checkpoint");
    }
    for payload in payloads {
        let AgentEventPayload::ApprovalRequested { approval_id, .. } = payload else {
            continue;
        };
        if let Err(error) = state
            .store
            .update_approval_status(*approval_id, ApprovalStatus::Denied)
        {
            warn!(?error, %approval_id, "failed to close an unpublished approval boundary");
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct HealthResponse {
    ok: bool,
    service: &'static str,
    #[serde(rename = "apiVersion")]
    api_version: u32,
    shell_runtime: ShellRuntimeStatus,
    office_runtime: OfficeRuntimeStatus,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct GitWorkflowResponse {
    action: opentopia_core::GitWorkflowActionKind,
    stdout: String,
    stderr: String,
    exit_code: Option<i32>,
    success: bool,
    truncated: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkillsQuery {
    workspace_root: Option<PathBuf>,
    thread_id: Option<Uuid>,
    experience_mode: Option<ExperienceMode>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginsQuery {
    workspace_root: Option<PathBuf>,
    thread_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstallPluginRequest {
    path: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UninstallPluginRequest {
    plugin_id: String,
    workspace_root: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadPluginRequest {
    plugin_id: String,
    enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CancelAgentTurnRequest {
    turn_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
struct TurnFileDiffPreviewQuery {
    path: PathBuf,
    offset: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TurnUndoRequest {
    #[serde(default)]
    confirm: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct PluginView {
    plugin: PluginDescriptor,
    skill_ids: Vec<String>,
    mcp_servers: Vec<McpServerView>,
    thread_enabled: bool,
    compatible: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct DeleteResponse {
    deleted: bool,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: message.into(),
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }

    fn too_many_requests(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
            message: message.into(),
        }
    }

    fn bad_gateway(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: message.into(),
        }
    }

    fn gateway_timeout(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::GATEWAY_TIMEOUT,
            message: message.into(),
        }
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(value: anyhow::Error) -> Self {
        let status = value
            .downcast_ref::<StoreError>()
            .map(|error| match error {
                StoreError::DuplicateWorkspace(_) => StatusCode::CONFLICT,
                StoreError::ProjectNotFound(_) => StatusCode::NOT_FOUND,
                StoreError::EmptyProjectName
                | StoreError::EmptyThreadTitle
                | StoreError::EmptyWorkspaceRoot
                | StoreError::ProjectHasNoWorkspace(_)
                | StoreError::ProjectWorkspaceInUse(_) => StatusCode::BAD_REQUEST,
            })
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        Self {
            status,
            message: value.to_string(),
        }
    }
}

impl From<opentopia_core::collaboration::CollaborationDomainError> for ApiError {
    fn from(error: opentopia_core::collaboration::CollaborationDomainError) -> Self {
        match error {
            opentopia_core::collaboration::CollaborationDomainError::AgentThreadNotFound(_)
            | opentopia_core::collaboration::CollaborationDomainError::AgentTurnNotFound(_)
            | opentopia_core::collaboration::CollaborationDomainError::SessionNotFound(_) => {
                Self::not_found(error.to_string())
            }
            opentopia_core::collaboration::CollaborationDomainError::AgentTurnAlreadyActive(_)
            | opentopia_core::collaboration::CollaborationDomainError::InvalidTurnTransition {
                ..
            } => Self::conflict(error.to_string()),
            _ => Self::internal(error.to_string()),
        }
    }
}

impl From<opentopia_core::collaboration::AgentCollaborationRuntimeError> for ApiError {
    fn from(error: opentopia_core::collaboration::AgentCollaborationRuntimeError) -> Self {
        use opentopia_core::collaboration::{AgentCollaborationRuntimeError, AgentMailboxError};

        match error {
            AgentCollaborationRuntimeError::Domain(error) => error.into(),
            AgentCollaborationRuntimeError::Mailbox(
                error @ AgentMailboxError::MessageNotFound(_),
            ) => Self::not_found(error.to_string()),
            AgentCollaborationRuntimeError::Mailbox(
                error @ AgentMailboxError::WrongTarget { .. },
            ) => Self::forbidden(error.to_string()),
            AgentCollaborationRuntimeError::Mailbox(error) => Self::internal(error.to_string()),
            error @ AgentCollaborationRuntimeError::RunSubmission { .. } => {
                Self::internal(error.to_string())
            }
        }
    }
}

impl From<opentopia_core::mcp_host::McpHostError> for ApiError {
    fn from(value: opentopia_core::mcp_host::McpHostError) -> Self {
        let status = match &value {
            opentopia_core::mcp_host::McpHostError::ServerNotFound(_)
            | opentopia_core::mcp_host::McpHostError::ToolNotFound(_) => StatusCode::NOT_FOUND,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            status,
            message: value.to_string(),
        }
    }
}

impl From<std::io::Error> for ApiError {
    fn from(value: std::io::Error) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: value.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(serde_json::json!({
            "error": self.message,
        }));
        (self.status, body).into_response()
    }
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
