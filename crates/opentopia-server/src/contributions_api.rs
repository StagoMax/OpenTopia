use super::resource_api::{preview_api_error, resolve_preview_id_for_thread};
use super::{
    current_settings, ensure_thread, load_agent_profiles_for_thread, plugins_api, publish_payload,
    sync_plugin_mcp_configs, ApiError, AppState,
};
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderValue, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use opentopia_core::{
    discover_plugins, load_plugin_mcp_servers, mcp_operation_fingerprint, AgentEventPayload,
    AppViewDescriptor, AppViewMessage, AppViewSession, BasicPolicyEngine,
    ContributionHandlerRegistry, ContributionKind, McpServerConfig, McpToolDescriptor,
    MediaHandlerDescriptor, MediaHandlerInvocationV1, MediaHandlerOperation,
    MediaHandlerResultEnvelopeV1, MediaHandlerRuntime, MediaHandlerSelection, ModelContentPart,
    PluginContribution, PluginMcpServerDefinition, PluginPermissionKind, PluginRuntimeHealthRecord,
    PluginRuntimeHealthStatus, PolicyDecision, PolicyEngine, ToolCall, ToolPermissionDescriptor,
    ToolResult, MAX_MEDIA_HANDLER_INPUT_BYTES, MAX_MEDIA_HANDLER_OUTPUT_BYTES,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_json::Value;
use std::io::Read;
use std::path::{Path as FsPath, PathBuf};
use std::time::Duration;
use uuid::Uuid;

const MAX_APP_ENTRY_BYTES: u64 = 5 * 1024 * 1024;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/threads/:thread_id/contribution-hosts",
            get(get_contribution_hosts),
        )
        .route(
            "/api/threads/:thread_id/preview-handler",
            get(select_preview_handler),
        )
        .route(
            "/api/threads/:thread_id/context-loader",
            get(select_context_loader),
        )
        .route(
            "/api/threads/:thread_id/media-handlers/invoke",
            post(invoke_media_handler),
        )
        .route(
            "/api/threads/:thread_id/plugin-app-sessions",
            post(start_app_session),
        )
        .route(
            "/api/threads/:thread_id/plugin-app-sessions/:session_id/messages",
            post(post_app_message),
        )
        .route(
            "/api/threads/:thread_id/plugin-app-sessions/:session_id/content",
            get(read_app_content),
        )
        .route(
            "/api/threads/:thread_id/plugin-app-sessions/:session_id",
            axum::routing::delete(stop_app_session),
        )
}

async fn invoke_media_handler(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(request): Json<InvokeMediaHandlerRequest>,
) -> Result<Json<MediaHandlerInvocationResponse>, ApiError> {
    let thread = ensure_thread(&state, thread_id)?;
    let active = active_contributions(&state, &thread)?;
    let (registry, _, _) = build_hosts(&active);
    let source = if let Some(resource_id) = request.resource_id.as_deref() {
        if request.path.is_some() {
            return Err(ApiError::bad_request(
                "media handler request must use either resourceId or path",
            ));
        }
        resolve_registered_media_source(&state, thread_id, resource_id)?
    } else {
        resolve_workspace_media_source(
            &thread.workspace_root,
            request
                .path
                .as_deref()
                .ok_or_else(|| ApiError::bad_request("media handler source is required"))?,
        )?
    };
    let selection = match request.operation {
        MediaHandlerOperation::Preview => registry.select_previewer(
            Some(&source.protocol_path),
            request
                .content_type
                .as_deref()
                .or(source.content_type.as_deref()),
        ),
        MediaHandlerOperation::LoadContext => registry.select_context_loader(
            Some(&source.protocol_path),
            request
                .content_type
                .as_deref()
                .or(source.content_type.as_deref()),
        ),
    };
    let handler = selected_handler(selection)?;
    if request
        .contribution_id
        .as_ref()
        .is_some_and(|requested| requested != &handler.contribution_id)
    {
        return Err(ApiError::conflict(format!(
            "requested handler is not the active best match; selected {}",
            handler.contribution_id
        )));
    }
    let contribution = active
        .iter()
        .find(|contribution| contribution.id == handler.contribution_id)
        .cloned()
        .ok_or_else(|| ApiError::bad_request("selected handler is no longer active"))?;

    let content_type = request.content_type.or_else(|| source.content_type.clone());
    let result = invoke_active_media_handler(
        &state,
        thread_id,
        &thread.workspace_root,
        &active,
        &contribution,
        source,
        content_type,
        request.options,
    )
    .await;
    match result {
        Ok(response) => {
            record_handler_health(
                &state,
                &contribution,
                PluginRuntimeHealthStatus::Ready,
                None,
            );
            Ok(Json(response))
        }
        Err(error) => {
            record_handler_health(
                &state,
                &contribution,
                error.health_status,
                Some(error.api.message.clone()),
            );
            Err(error.api)
        }
    }
}

async fn invoke_active_media_handler(
    state: &AppState,
    thread_id: Uuid,
    workspace_root: &FsPath,
    active: &[PluginContribution],
    contribution: &PluginContribution,
    source: MediaSource,
    content_type: Option<String>,
    options: Value,
) -> Result<MediaHandlerInvocationResponse, HandlerInvocationError> {
    ensure_workspace_read_permission(contribution)?;
    let settings = current_settings(state);
    let sandbox = settings.sandbox.to_local_sandbox_config();
    let policy = BasicPolicyEngine::new_with_sandbox_config(
        workspace_root.to_path_buf(),
        settings.permission_mode,
        &sandbox,
    );
    if source.brokered_content.is_none() {
        enforce_handler_policy(
            policy.inspect_read(&source.canonical_path),
            "media handler source read",
        )?;
    }

    let runtime = MediaHandlerRuntime::parse(
        &MediaHandlerDescriptor::from_contribution(contribution)
            .map_err(HandlerInvocationError::bad_request)?
            .runtime,
    )
    .map_err(HandlerInvocationError::bad_request)?;
    let MediaHandlerRuntime::McpV1 { server, tool } = &runtime else {
        return Err(HandlerInvocationError::bad_request(
            "builtin media handlers are invoked by their host-owned adapter, not the plugin MCP endpoint",
        ));
    };
    if !active.iter().any(|candidate| {
        candidate.plugin_id == contribution.plugin_id
            && candidate.kind == ContributionKind::McpServer
    }) {
        return Err(HandlerInvocationError::bad_request(
            "the handler's plugin MCP server contribution is not active for this thread",
        ));
    }

    let plugin = discover_plugins(Some(workspace_root))
        .into_iter()
        .find(|plugin| plugin.id == contribution.plugin_id)
        .ok_or_else(|| {
            HandlerInvocationError::not_found("handler plugin is no longer available")
        })?;
    let server_config = resolve_plugin_mcp_server(state, &plugin, server).await?;
    if server_config.plugin_id.as_deref() != Some(contribution.plugin_id.as_str())
        || server_config.plugin_server_name.as_deref() != Some(server.as_str())
    {
        return Err(HandlerInvocationError::bad_request(
            "MCP runtime server does not belong to the handler plugin",
        ));
    }

    state
        .mcp_host
        .ensure_server(server_config.clone())
        .await
        .map_err(HandlerInvocationError::bad_gateway)?;
    let descriptor = state
        .mcp_host
        .cached_tools(server_config.server_id)
        .await
        .into_iter()
        .find(|descriptor| descriptor.tool_name == *tool)
        .ok_or_else(|| {
            HandlerInvocationError::not_found(format!(
                "handler MCP tool `{tool}` was not exposed by server `{server}`"
            ))
        })?;
    if descriptor.server_id != server_config.server_id {
        return Err(HandlerInvocationError::bad_request(
            "handler MCP tool route does not belong to the declared server",
        ));
    }
    enforce_handler_policy(
        policy.inspect_mcp_tool_call(&ToolPermissionDescriptor::from(&descriptor)),
        "media handler MCP tool call",
    )?;

    let content = match source.brokered_content {
        Some(content) => content,
        None => read_bounded_media_source(source.canonical_path.clone()).await?,
    };
    let invocation = MediaHandlerInvocationV1::new(
        match contribution.kind {
            ContributionKind::Previewer => MediaHandlerOperation::Preview,
            ContributionKind::ContextLoader => MediaHandlerOperation::LoadContext,
            _ => {
                return Err(HandlerInvocationError::bad_request(
                    "media handler contribution has the wrong kind",
                ))
            }
        },
        contribution.id.clone(),
        path_for_protocol(&source.protocol_path),
        content_type.unwrap_or_else(|| "application/octet-stream".to_string()),
        &content,
        options,
    )
    .map_err(HandlerInvocationError::bad_request)?;
    let operation = invocation.operation;
    let arguments = invocation.into_mcp_arguments();
    let call = ToolCall::new(&descriptor.public_name, arguments.clone());
    publish_payload(
        state,
        thread_id,
        None,
        AgentEventPayload::ToolCallStarted { call: call.clone() },
    );

    let timeout_ms = server_config.timeout_ms.min(60_000).max(1_000);
    let operation_fingerprint = mcp_operation_fingerprint(&descriptor);
    let call_result = match tokio::time::timeout(
        Duration::from_millis(timeout_ms),
        state.mcp_host.call_server_tool(
            server_config.server_id,
            &descriptor.tool_name,
            &operation_fingerprint,
            arguments,
        ),
    )
    .await
    {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => {
            publish_handler_failure(state, thread_id, &call, &descriptor, error.to_string());
            return Err(HandlerInvocationError::bad_gateway(error));
        }
        Err(_) => {
            let message = format!("media handler MCP call timed out after {timeout_ms} ms");
            publish_handler_failure(state, thread_id, &call, &descriptor, message.clone());
            return Err(HandlerInvocationError::gateway_timeout(message));
        }
    };
    if call_result.is_error {
        publish_handler_failure(
            state,
            thread_id,
            &call,
            &descriptor,
            call_result.output.clone(),
        );
        return Err(HandlerInvocationError::bad_gateway(format!(
            "media handler MCP tool returned an error: {}",
            call_result.output
        )));
    }
    let raw_output_bytes = serde_json::to_vec(&call_result.raw)
        .map_err(HandlerInvocationError::bad_gateway)?
        .len();
    if raw_output_bytes > MAX_MEDIA_HANDLER_OUTPUT_BYTES {
        let message = format!(
            "media handler MCP response is {raw_output_bytes} bytes; maximum is {}",
            MAX_MEDIA_HANDLER_OUTPUT_BYTES
        );
        publish_handler_failure(state, thread_id, &call, &descriptor, message.clone());
        return Err(HandlerInvocationError::bad_gateway(message));
    }
    let structured_content = match call_result.structured_content {
        Some(content) => content,
        None => {
            let message = "media handler MCP tool must return structuredContent".to_string();
            publish_handler_failure(state, thread_id, &call, &descriptor, message.clone());
            return Err(HandlerInvocationError::bad_gateway(message));
        }
    };
    let output = match MediaHandlerResultEnvelopeV1::from_structured_content(
        structured_content,
        operation,
    ) {
        Ok(output) => output,
        Err(error) => {
            publish_handler_failure(state, thread_id, &call, &descriptor, error.to_string());
            return Err(HandlerInvocationError::bad_gateway(error));
        }
    };
    publish_payload(
        state,
        thread_id,
        None,
        AgentEventPayload::ToolCallFinished {
            result: ToolResult {
                call_id: call.id,
                output: "media handler completed".to_string(),
                content: vec![ModelContentPart::json(
                    serde_json::to_value(&output).unwrap_or(Value::Null),
                )],
                metadata: json!({
                    "success": true,
                    "contributionId": contribution.id,
                    "serverId": descriptor.server_id,
                    "toolName": descriptor.tool_name,
                }),
            },
        },
    );
    Ok(MediaHandlerInvocationResponse {
        contribution_id: contribution.id.clone(),
        plugin_id: contribution.plugin_id.clone(),
        runtime,
        bytes_read: content.len(),
        output,
    })
}

async fn get_contribution_hosts(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<ContributionHostSnapshot>, ApiError> {
    let thread = ensure_thread(&state, thread_id)?;
    let contributions = active_contributions(&state, &thread)?;
    let (handlers, apps, mut issues) = build_hosts(&contributions);
    let profiles = load_agent_profiles_for_thread(&state.store, &thread)?;
    issues.extend(profiles.warnings().iter().cloned());
    Ok(Json(ContributionHostSnapshot {
        previewers: handlers.previewers(),
        context_loaders: handlers.context_loaders(),
        apps,
        agent_profiles: profiles
            .list()
            .into_iter()
            .filter(|profile| profile.source_plugin_id.is_some())
            .collect(),
        issues,
    }))
}

async fn select_preview_handler(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Query(query): Query<HandlerQuery>,
) -> Result<Json<MediaHandlerSelection>, ApiError> {
    let thread = ensure_thread(&state, thread_id)?;
    let contributions = active_contributions(&state, &thread)?;
    let (handlers, _, _) = build_hosts(&contributions);
    Ok(Json(handlers.select_previewer(
        query.path.as_deref().map(FsPath::new),
        query.content_type.as_deref(),
    )))
}

async fn select_context_loader(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Query(query): Query<HandlerQuery>,
) -> Result<Json<MediaHandlerSelection>, ApiError> {
    let thread = ensure_thread(&state, thread_id)?;
    let contributions = active_contributions(&state, &thread)?;
    let (handlers, _, _) = build_hosts(&contributions);
    Ok(Json(handlers.select_context_loader(
        query.path.as_deref().map(FsPath::new),
        query.content_type.as_deref(),
    )))
}

async fn start_app_session(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(request): Json<StartAppSessionRequest>,
) -> Result<Json<AppViewSessionResponse>, ApiError> {
    let thread = ensure_thread(&state, thread_id)?;
    let contribution = active_contributions(&state, &thread)?
        .into_iter()
        .find(|contribution| {
            contribution.kind == ContributionKind::App && contribution.id == request.contribution_id
        })
        .ok_or_else(|| ApiError::not_found("app contribution is not active for this thread"))?;
    let session = state
        .app_views
        .lock()
        .map_err(|_| ApiError::internal("app view host lock poisoned"))?
        .start_contribution(thread_id, &contribution)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    Ok(Json(AppViewSessionResponse::from_session(session)))
}

async fn post_app_message(
    State(state): State<AppState>,
    Path((thread_id, session_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<AppMessageRequest>,
) -> Result<Json<AppViewMessage>, ApiError> {
    ensure_active_session(&state, thread_id, session_id)?;
    let message = state
        .app_views
        .lock()
        .map_err(|_| ApiError::internal("app view host lock poisoned"))?
        .post_message(session_id, &request.channel, request.payload)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    Ok(Json(message))
}

async fn stop_app_session(
    State(state): State<AppState>,
    Path((thread_id, session_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<AppViewSession>, ApiError> {
    ensure_session_thread(&state, thread_id, session_id)?;
    let session = state
        .app_views
        .lock()
        .map_err(|_| ApiError::internal("app view host lock poisoned"))?
        .stop(session_id)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    Ok(Json(session))
}

async fn read_app_content(
    State(state): State<AppState>,
    Path((thread_id, session_id)): Path<(Uuid, Uuid)>,
) -> Result<Response<Body>, ApiError> {
    let session = ensure_active_session(&state, thread_id, session_id)?;
    if PathBuf::from(&session.descriptor.entry)
        .extension()
        .and_then(|value| value.to_str())
        != Some("html")
    {
        return Err(ApiError::bad_request(
            "app view entry must be a standalone HTML document",
        ));
    }
    let thread = ensure_thread(&state, thread_id)?;
    let plugin = discover_plugins(Some(&thread.workspace_root))
        .into_iter()
        .find(|plugin| plugin.id == session.descriptor.plugin_id)
        .ok_or_else(|| ApiError::not_found("app plugin is no longer available"))?;
    let root = plugin
        .path
        .canonicalize()
        .map_err(|error| ApiError::internal(error.to_string()))?;
    let entry = root
        .join(&session.descriptor.entry)
        .canonicalize()
        .map_err(|_| ApiError::not_found("app view entry was not found"))?;
    if !entry.starts_with(&root) || !entry.is_file() {
        return Err(ApiError::bad_request(
            "app view entry escapes the plugin package",
        ));
    }
    let metadata =
        std::fs::metadata(&entry).map_err(|error| ApiError::internal(error.to_string()))?;
    if metadata.len() > MAX_APP_ENTRY_BYTES {
        return Err(ApiError::bad_request("app view entry exceeds 5 MiB"));
    }
    let bytes = std::fs::read(&entry).map_err(|error| ApiError::internal(error.to_string()))?;
    let mut response = Response::new(Body::from(bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "sandbox allow-scripts; default-src 'none'; img-src data: blob:; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'none'; frame-ancestors 'self'",
        ),
    );
    Ok(response)
}

fn active_contributions(
    state: &AppState,
    thread: &opentopia_core::Thread,
) -> Result<Vec<PluginContribution>, ApiError> {
    plugins_api::active_contributions_for_thread(&state.store, thread)
        .map_err(|error| ApiError::bad_request(error.to_string()))
}

pub(crate) fn handler_registry_for_thread(
    state: &AppState,
    thread: &opentopia_core::Thread,
) -> Result<ContributionHandlerRegistry, ApiError> {
    let contributions = active_contributions(state, thread)?;
    Ok(build_hosts(&contributions).0)
}

fn build_hosts(
    contributions: &[PluginContribution],
) -> (
    ContributionHandlerRegistry,
    Vec<AppViewDescriptor>,
    Vec<String>,
) {
    let mut handlers = ContributionHandlerRegistry::default();
    let mut apps = Vec::new();
    let mut issues = Vec::new();
    for contribution in contributions {
        match contribution.kind {
            ContributionKind::Previewer | ContributionKind::ContextLoader => {
                if let Err(error) = handlers.register(contribution) {
                    issues.push(error.to_string());
                }
            }
            ContributionKind::App => match AppViewDescriptor::from_contribution(contribution) {
                Ok(app) => apps.push(app),
                Err(error) => issues.push(error.to_string()),
            },
            _ => {}
        }
    }
    apps.sort_by(|left, right| left.contribution_id.cmp(&right.contribution_id));
    (handlers, apps, issues)
}

fn ensure_session_thread(
    state: &AppState,
    thread_id: Uuid,
    session_id: Uuid,
) -> Result<AppViewSession, ApiError> {
    ensure_thread(state, thread_id)?;
    let session = state
        .app_views
        .lock()
        .map_err(|_| ApiError::internal("app view host lock poisoned"))?
        .session(session_id)
        .ok_or_else(|| ApiError::not_found("app view session was not found"))?;
    if session.thread_id != thread_id {
        return Err(ApiError::not_found("app view session was not found"));
    }
    Ok(session)
}

fn ensure_active_session(
    state: &AppState,
    thread_id: Uuid,
    session_id: Uuid,
) -> Result<AppViewSession, ApiError> {
    let session = ensure_session_thread(state, thread_id, session_id)?;
    let thread = ensure_thread(state, thread_id)?;
    let active = active_contributions(state, &thread)?
        .into_iter()
        .any(|contribution| contribution.id == session.descriptor.contribution_id);
    if !active {
        let _ = state
            .app_views
            .lock()
            .map_err(|_| ApiError::internal("app view host lock poisoned"))?
            .stop(session_id);
        return Err(ApiError::bad_request(
            "app contribution is no longer active for this thread",
        ));
    }
    Ok(session)
}

fn selected_handler(selection: MediaHandlerSelection) -> Result<MediaHandlerDescriptor, ApiError> {
    match selection {
        MediaHandlerSelection::Selected { handler } => Ok(handler),
        MediaHandlerSelection::None => Err(ApiError::not_found(
            "no active media handler matches the requested source",
        )),
        MediaHandlerSelection::Conflict { contribution_ids } => Err(ApiError::conflict(format!(
            "multiple active media handlers have equal priority: {}",
            contribution_ids.join(", ")
        ))),
    }
}

fn resolve_workspace_media_source(
    workspace_root: &FsPath,
    requested_path: &FsPath,
) -> Result<MediaSource, ApiError> {
    if requested_path.as_os_str().is_empty()
        || requested_path.is_absolute()
        || requested_path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(ApiError::bad_request(
            "media handler path must be relative to the thread workspace",
        ));
    }
    let workspace_root = workspace_root
        .canonicalize()
        .map_err(|error| ApiError::internal(error.to_string()))?;
    let canonical_path = workspace_root
        .join(requested_path)
        .canonicalize()
        .map_err(|_| ApiError::not_found("media handler source was not found"))?;
    if !canonical_path.starts_with(&workspace_root) || !canonical_path.is_file() {
        return Err(ApiError::bad_request(
            "media handler source escapes the thread workspace or is not a file",
        ));
    }
    let metadata = std::fs::metadata(&canonical_path)
        .map_err(|error| ApiError::internal(error.to_string()))?;
    if metadata.len() > MAX_MEDIA_HANDLER_INPUT_BYTES as u64 {
        return Err(ApiError::bad_request(format!(
            "media handler source exceeds {} bytes",
            MAX_MEDIA_HANDLER_INPUT_BYTES
        )));
    }
    let relative_path = canonical_path
        .strip_prefix(&workspace_root)
        .map(PathBuf::from)
        .map_err(|_| ApiError::bad_request("media handler source escapes the thread workspace"))?;
    Ok(MediaSource {
        canonical_path,
        protocol_path: relative_path,
        content_type: None,
        brokered_content: None,
    })
}

fn resolve_registered_media_source(
    state: &AppState,
    thread_id: Uuid,
    resource_id: &str,
) -> Result<MediaSource, ApiError> {
    let preview = resolve_preview_id_for_thread(state, thread_id, resource_id)?;
    let descriptor = preview.descriptor.clone();
    let content =
        opentopia_core::read_preview_content(&preview, MAX_MEDIA_HANDLER_INPUT_BYTES as u64)
            .map_err(preview_api_error)?;
    let protocol_name = FsPath::new(&descriptor.name)
        .file_name()
        .filter(|name| !name.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("resource.bin"));
    let canonical_path = match preview.content {
        opentopia_core::PreviewContentSource::Path(path) => path,
        opentopia_core::PreviewContentSource::Inline(_) => protocol_name.clone(),
    };
    Ok(MediaSource {
        canonical_path,
        protocol_path: protocol_name,
        content_type: Some(descriptor.content_type),
        brokered_content: Some(content),
    })
}

fn ensure_workspace_read_permission(
    contribution: &PluginContribution,
) -> Result<(), HandlerInvocationError> {
    if contribution.permissions.iter().any(|permission| {
        permission.kind == PluginPermissionKind::Filesystem && permission.value == "workspace:read"
    }) {
        Ok(())
    } else {
        Err(HandlerInvocationError::bad_request(
            "MCP media handlers must declare and receive the filesystem permission `workspace:read`",
        ))
    }
}

fn enforce_handler_policy(
    decision: PolicyDecision,
    action: &str,
) -> Result<(), HandlerInvocationError> {
    match decision {
        PolicyDecision::Allow => Ok(()),
        PolicyDecision::Deny { reason } => Err(HandlerInvocationError::degraded(format!(
            "{action} denied by policy: {reason}"
        ))),
        PolicyDecision::Ask { reason } => Err(HandlerInvocationError::degraded(format!(
            "{action} requires approval and was not executed: {reason}"
        ))),
    }
}

async fn resolve_plugin_mcp_server(
    state: &AppState,
    plugin: &opentopia_core::PluginDescriptor,
    server_name: &str,
) -> Result<McpServerConfig, HandlerInvocationError> {
    let definitions =
        load_plugin_mcp_servers(plugin).map_err(HandlerInvocationError::bad_request)?;
    let definition = definitions
        .iter()
        .find(|definition| definition.name == server_name)
        .ok_or_else(|| {
            HandlerInvocationError::bad_request(format!(
                "handler runtime references undeclared MCP server `{server_name}`"
            ))
        })?;
    let existing = state
        .store
        .list_plugin_mcp_servers(&plugin.id)
        .map_err(HandlerInvocationError::internal)?;
    if let Some(server) = existing.into_iter().find(|server| {
        server.plugin_server_name.as_deref() == Some(server_name)
            && server_matches_definition(server, definition)
    }) {
        return Ok(server);
    }
    sync_plugin_mcp_configs(state, plugin)
        .await
        .map_err(HandlerInvocationError::from_api)?
        .into_iter()
        .find(|server| server.plugin_server_name.as_deref() == Some(server_name))
        .ok_or_else(|| {
            HandlerInvocationError::bad_request(format!(
                "handler MCP server `{server_name}` could not be synchronized"
            ))
        })
}

fn server_matches_definition(
    server: &McpServerConfig,
    definition: &PluginMcpServerDefinition,
) -> bool {
    server.enabled
        && server.command == definition.command
        && server.args == definition.args
        && server.cwd.as_deref() == Some(definition.cwd.as_path())
        && server.env_keys == definition.env_keys
        && server.timeout_ms == definition.timeout_ms
}

async fn read_bounded_media_source(path: PathBuf) -> Result<Vec<u8>, HandlerInvocationError> {
    tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(path)?;
        let mut bytes = Vec::new();
        file.take((MAX_MEDIA_HANDLER_INPUT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_MEDIA_HANDLER_INPUT_BYTES {
            anyhow::bail!(
                "media handler source exceeds {} bytes",
                MAX_MEDIA_HANDLER_INPUT_BYTES
            );
        }
        Ok::<_, anyhow::Error>(bytes)
    })
    .await
    .map_err(|error| HandlerInvocationError::internal(error.to_string()))?
    .map_err(HandlerInvocationError::bad_request)
}

fn path_for_protocol(path: &FsPath) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn publish_handler_failure(
    state: &AppState,
    thread_id: Uuid,
    call: &ToolCall,
    descriptor: &McpToolDescriptor,
    message: String,
) {
    publish_payload(
        state,
        thread_id,
        None,
        AgentEventPayload::ToolCallFinished {
            result: ToolResult {
                call_id: call.id,
                output: message.clone(),
                content: vec![ModelContentPart::text(message.clone())],
                metadata: json!({
                    "success": false,
                    "error": message,
                    "serverId": descriptor.server_id,
                    "toolName": descriptor.tool_name,
                }),
            },
        },
    );
}

fn record_handler_health(
    state: &AppState,
    contribution: &PluginContribution,
    status: PluginRuntimeHealthStatus,
    last_error: Option<String>,
) {
    let restart_count = state
        .store
        .list_plugin_runtime_health(&contribution.plugin_id)
        .ok()
        .and_then(|records| {
            records
                .into_iter()
                .find(|record| record.contribution_id == contribution.id)
        })
        .map(|record| record.restart_count)
        .unwrap_or_default();
    let last_error = last_error.map(|message| message.chars().take(2048).collect());
    let record = PluginRuntimeHealthRecord {
        plugin_id: contribution.plugin_id.clone(),
        contribution_id: contribution.id.clone(),
        status,
        last_error,
        last_checked_at: Utc::now(),
        restart_count,
    };
    if let Err(error) = state.store.put_plugin_runtime_health(&record) {
        tracing::warn!(
            contribution_id = %contribution.id,
            %error,
            "failed to persist media handler health"
        );
    }
}

struct HandlerInvocationError {
    api: ApiError,
    health_status: PluginRuntimeHealthStatus,
}

impl HandlerInvocationError {
    fn from_api(api: ApiError) -> Self {
        Self {
            api,
            health_status: PluginRuntimeHealthStatus::Error,
        }
    }

    fn bad_request(error: impl ToString) -> Self {
        Self::from_api(ApiError::bad_request(error.to_string()))
    }

    fn not_found(error: impl ToString) -> Self {
        Self::from_api(ApiError::not_found(error.to_string()))
    }

    fn bad_gateway(error: impl ToString) -> Self {
        Self::from_api(ApiError::bad_gateway(error.to_string()))
    }

    fn gateway_timeout(error: impl ToString) -> Self {
        Self::from_api(ApiError::gateway_timeout(error.to_string()))
    }

    fn internal(error: impl ToString) -> Self {
        Self::from_api(ApiError::internal(error.to_string()))
    }

    fn degraded(error: impl ToString) -> Self {
        Self {
            api: ApiError::bad_request(error.to_string()),
            health_status: PluginRuntimeHealthStatus::Degraded,
        }
    }
}

struct MediaSource {
    canonical_path: PathBuf,
    protocol_path: PathBuf,
    content_type: Option<String>,
    brokered_content: Option<Vec<u8>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InvokeMediaHandlerRequest {
    operation: MediaHandlerOperation,
    #[serde(default)]
    contribution_id: Option<String>,
    #[serde(default)]
    path: Option<PathBuf>,
    #[serde(default)]
    resource_id: Option<String>,
    #[serde(default)]
    content_type: Option<String>,
    #[serde(default = "empty_json_object")]
    options: Value,
}

fn empty_json_object() -> Value {
    json!({})
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct MediaHandlerInvocationResponse {
    contribution_id: String,
    plugin_id: String,
    runtime: MediaHandlerRuntime,
    bytes_read: usize,
    output: MediaHandlerResultEnvelopeV1,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct ContributionHostSnapshot {
    previewers: Vec<MediaHandlerDescriptor>,
    context_loaders: Vec<MediaHandlerDescriptor>,
    apps: Vec<AppViewDescriptor>,
    agent_profiles: Vec<opentopia_core::AgentProfile>,
    issues: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HandlerQuery {
    path: Option<String>,
    content_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartAppSessionRequest {
    contribution_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct AppViewSessionResponse {
    #[serde(flatten)]
    session: AppViewSession,
    content_path: String,
}

impl AppViewSessionResponse {
    fn from_session(session: AppViewSession) -> Self {
        Self {
            content_path: format!(
                "/api/threads/{}/plugin-app-sessions/{}/content",
                session.thread_id, session.session_id
            ),
            session,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppMessageRequest {
    channel: String,
    #[serde(default)]
    payload: Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        auth::ApiAuth, turn_changes::TurnChangeManager, turns::RootTurnLifecycle, EventBus,
        PtyManager, TerminalBus,
    };
    use async_trait::async_trait;
    use axum::body::to_bytes;
    use axum::http::StatusCode;
    use chrono::Utc;
    use opentopia_core::mcp_host::{
        McpChildProcess, McpExtensionHost, McpHostError, McpProcessSpawner, McpSpawnedProcess,
    };
    use opentopia_core::{
        AppSettings, AppViewSessionStatus, BackgroundProcessRegistry, BrowserRuntimeConfig,
        CodexAccountManager, ComputerRuntime, ComputerRuntimeConfig, ContributionOrigin,
        LocalBrowserRuntime, LocalComputerRuntime, PermissionMode, PluginControlScope,
        PluginDescriptor, PluginPermission, PluginPermissionGrantStatus, SessionStore,
        SqliteSessionStore, MEDIA_HANDLER_RESULT_API_VERSION,
    };
    use serde_json::json;
    use std::fs;
    use std::sync::{Arc, Mutex, RwLock};
    use tokio::io::{duplex, AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};
    use tokio::sync::mpsc;

    struct AppPluginFixture {
        workspace: PathBuf,
    }

    impl AppPluginFixture {
        fn new() -> Self {
            let workspace =
                std::env::temp_dir().join(format!("opentopia-app-view-api-{}", Uuid::new_v4()));
            let root = workspace.join(".opentopia/plugins/app-view-pack");
            fs::create_dir_all(root.join(".codex-plugin")).expect("create manifest directory");
            fs::create_dir_all(root.join("apps")).expect("create app directory");
            fs::write(
                root.join(".codex-plugin/plugin.json"),
                serde_json::to_vec_pretty(&json!({
                    "name": "app-view-pack",
                    "version": "1.0.0",
                    "opentopia": {
                        "apiVersion": "1",
                        "requires": {"hostCapabilities": ["appView.v1"]},
                        "permissions": {
                            "filesystem": ["workspace:read"],
                            "network": [],
                            "secrets": [],
                            "desktop": []
                        },
                        "contributes": {
                            "apps": [{
                                "id": "dashboard",
                                "entry": "apps/dashboard.html",
                                "title": "Dashboard",
                                "allowedChannels": ["refresh"]
                            }]
                        }
                    }
                }))
                .expect("serialize app manifest"),
            )
            .expect("write app manifest");
            fs::write(
                root.join("apps/dashboard.html"),
                "<!doctype html><title>Sandboxed dashboard</title>",
            )
            .expect("write app entry");
            Self { workspace }
        }

        fn plugin(&self) -> PluginDescriptor {
            discover_plugins(Some(&self.workspace))
                .into_iter()
                .find(|plugin| plugin.name == "app-view-pack")
                .expect("discover app plugin")
        }
    }

    impl Drop for AppPluginFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.workspace);
        }
    }

    struct MediaPluginFixture {
        workspace: PathBuf,
    }

    impl MediaPluginFixture {
        fn new() -> Self {
            let workspace = std::env::temp_dir()
                .join(format!("opentopia-media-handler-api-{}", Uuid::new_v4()));
            let root = workspace.join(".opentopia/plugins/media-pack");
            fs::create_dir_all(root.join(".codex-plugin")).expect("create manifest directory");
            fs::write(
                root.join(".codex-plugin/plugin.json"),
                serde_json::to_vec_pretty(&json!({
                    "name": "media-pack",
                    "version": "1.0.0",
                    "mcpServers": "./.mcp.json",
                    "opentopia": {
                        "apiVersion": "1",
                        "requires": {"hostCapabilities": ["previewer.v1"]},
                        "permissions": {
                            "filesystem": ["workspace:read"],
                            "network": [],
                            "secrets": [],
                            "desktop": []
                        },
                        "contributes": {
                            "previewers": [{
                                "id": "text",
                                "extensions": ["txt"],
                                "mediaTypes": ["text/plain"],
                                "runtime": "mcp.v1:media/render"
                            }]
                        }
                    }
                }))
                .expect("serialize media manifest"),
            )
            .expect("write media manifest");
            fs::write(
                root.join(".mcp.json"),
                serde_json::to_vec_pretty(&json!({
                    "mcpServers": {
                        "media": {
                            "type": "stdio",
                            "command": "mock-media-handler",
                            "timeoutMs": 5_000
                        }
                    }
                }))
                .expect("serialize MCP configuration"),
            )
            .expect("write MCP configuration");
            fs::write(workspace.join("sample.txt"), b"hello").expect("write media source");
            Self { workspace }
        }

        fn plugin(&self) -> PluginDescriptor {
            discover_plugins(Some(&self.workspace))
                .into_iter()
                .find(|plugin| plugin.name == "media-pack")
                .expect("discover media plugin")
        }
    }

    impl Drop for MediaPluginFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.workspace);
        }
    }

    fn test_state(store: Arc<SqliteSessionStore>) -> AppState {
        test_state_with_mcp(store, McpExtensionHost::new())
    }

    fn test_state_with_mcp(store: Arc<SqliteSessionStore>, mcp_host: McpExtensionHost) -> AppState {
        let loaded_settings = AppSettings::from_env(PermissionMode::Auto);
        let settings = Arc::new(RwLock::new(loaded_settings.clone()));
        let turn_inbox: Arc<dyn opentopia_core::TurnInbox> =
            Arc::new(opentopia_core::BufferedTurnInbox::default());
        let background = BackgroundProcessRegistry::default();
        let collaboration_repository = Arc::new(
            opentopia_core::collaboration::SqliteCollaborationRepository::new(store.clone())
                .expect("collaboration repository"),
        );
        let agent_activity = Arc::new(
            opentopia_core::collaboration::SqliteAgentActivitySource::new(
                collaboration_repository.clone(),
            ),
        );
        let snapshot_deriver: Arc<dyn opentopia_core::collaboration::RuntimeSnapshotDeriver> =
            Arc::new(opentopia_core::collaboration::AttenuatingRuntimeSnapshotDeriver);
        let agent_run_scheduler = crate::agent_runs::ServerAgentRunScheduler::new(
            collaboration_repository.clone(),
            turn_inbox.clone(),
            4,
        );
        let collaboration_runtime = opentopia_core::collaboration::AgentCollaborationRuntime::new(
            collaboration_repository.clone(),
            agent_run_scheduler.clone(),
            collaboration_repository.clone(),
        )
        .with_mailbox_notifier(agent_run_scheduler.clone());
        let turns = RootTurnLifecycle::new(
            store.clone(),
            collaboration_repository.clone(),
            agent_activity.clone(),
            agent_run_scheduler.clone(),
        );
        let (turn_queue, _turn_queue_rx) = mpsc::unbounded_channel();
        let browser = Arc::new(LocalBrowserRuntime::new(BrowserRuntimeConfig::default()));
        let browser_router = Arc::new(opentopia_core::BrowserRuntimeRouter::new(
            browser.clone(),
            None,
        ));
        let computer: Arc<dyn ComputerRuntime> =
            Arc::new(LocalComputerRuntime::new(ComputerRuntimeConfig::default()));
        let turn_changes = TurnChangeManager::new(store.clone());
        let agent_factory = crate::agent_factory::AgentFactory::new(
            turn_inbox.clone(),
            browser.clone(),
            computer.clone(),
            background.clone(),
            turn_changes.clone(),
        );
        let agent = Arc::new(RwLock::new(agent_factory.build(&loaded_settings)));
        AppState {
            store: store.clone(),
            agent,
            agent_factory,
            settings,
            codex_account: Arc::new(CodexAccountManager::default()),
            events: EventBus::default(),
            terminals: TerminalBus::default(),
            ptys: PtyManager::default(),
            browser,
            browser_router,
            computer,
            mcp_host,
            auth: ApiAuth::for_tests(),
            turns,
            turn_changes,
            turn_queue,
            turn_inbox,
            collaboration_repository,
            collaboration_runtime,
            agent_activity,
            snapshot_deriver,
            agent_run_scheduler,
            background,
            app_views: Arc::new(Mutex::new(opentopia_core::AppViewHost::default())),
            library_providers: Arc::new(crate::library_api::LibraryProviderRegistry::for_tests()),
            resources: crate::resource_registry::ResourceRegistry::default(),
            provider_runtime_health: crate::provider_runtime_health::ProviderRuntimeHealth::default(
            ),
            shutdown: crate::runtime_shutdown::RuntimeShutdown::default(),
        }
    }

    #[derive(Debug, Default)]
    struct MediaHandlerMcpSpawner;

    #[async_trait]
    impl McpProcessSpawner for MediaHandlerMcpSpawner {
        async fn spawn(
            &self,
            _config: &McpServerConfig,
        ) -> Result<McpSpawnedProcess, McpHostError> {
            let (client_stdin, server_stdin) = duplex(64 * 1024);
            let (server_stdout, client_stdout) = duplex(64 * 1024);
            tokio::spawn(run_media_handler_mcp_server(server_stdin, server_stdout));
            Ok(McpSpawnedProcess::new(
                client_stdin,
                client_stdout,
                None::<DuplexStream>,
                MockMcpChild,
            ))
        }
    }

    struct MockMcpChild;

    #[async_trait]
    impl McpChildProcess for MockMcpChild {
        async fn kill(&mut self) -> Result<(), McpHostError> {
            Ok(())
        }

        async fn wait(&mut self) -> Result<(), McpHostError> {
            Ok(())
        }

        fn start_kill(&mut self) {}
    }

    async fn run_media_handler_mcp_server(stdin: DuplexStream, mut stdout: DuplexStream) {
        let mut lines = BufReader::new(stdin).lines();
        while let Some(line) = lines.next_line().await.expect("read MCP request") {
            let request: Value = serde_json::from_str(&line).expect("parse MCP request");
            let method = request
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let id = request.get("id").cloned().unwrap_or(Value::Null);
            let response = match method {
                "initialize" => Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "protocolVersion": "2024-11-05",
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": "media-test", "version": "1.0.0"}
                    }
                })),
                "notifications/initialized" => None,
                "tools/list" => Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {"tools": [{
                        "name": "render",
                        "description": "Render bounded media",
                        "inputSchema": {"type": "object"},
                        "annotations": {"readOnlyHint": true}
                    }]}
                })),
                "tools/call" => {
                    let content_base64 = request
                        .pointer("/params/arguments/request/source/contentBase64")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    Some(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [{"type": "text", "text": "rendered"}],
                            "structuredContent": {
                                "apiVersion": MEDIA_HANDLER_RESULT_API_VERSION,
                                "kind": "preview",
                                "payload": {"receivedBase64": content_base64}
                            },
                            "isError": false
                        }
                    }))
                }
                _ => Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32601, "message": "unknown method"}
                })),
            };
            if let Some(response) = response {
                let mut bytes = serde_json::to_vec(&response).expect("serialize MCP response");
                bytes.push(b'\n');
                stdout.write_all(&bytes).await.expect("write MCP response");
                stdout.flush().await.expect("flush MCP response");
            }
        }
    }

    fn set_app_permissions(
        store: &SqliteSessionStore,
        plugin: &PluginDescriptor,
        status: PluginPermissionGrantStatus,
    ) {
        let manifest =
            opentopia_core::inspect_plugin_control_manifest(plugin).expect("inspect app manifest");
        for request in &manifest.permission_requests {
            store
                .set_manifest_plugin_permission_grant(
                    &plugin.id,
                    &manifest,
                    &PluginControlScope::global(),
                    &request.permission,
                    &Value::Null,
                    status,
                )
                .expect("update app permission");
        }
    }

    fn contribution(
        kind: ContributionKind,
        local_id: &str,
        declaration: Value,
    ) -> PluginContribution {
        PluginContribution {
            id: format!("plugin/{local_id}"),
            plugin_id: "plugin".to_string(),
            local_id: local_id.to_string(),
            kind,
            origin: ContributionOrigin::OpenTopia,
            api_version: "1".to_string(),
            required_host_capabilities: Vec::new(),
            permissions: Vec::<PluginPermission>::new(),
            configuration_schema: None,
            declaration,
        }
    }

    #[test]
    fn host_snapshot_keeps_invalid_contributions_as_issues() {
        let contributions = vec![
            contribution(
                ContributionKind::Previewer,
                "valid",
                json!({"extensions": ["pdf"], "runtime": "mcp.v1:documents/preview"}),
            ),
            contribution(
                ContributionKind::App,
                "invalid",
                json!({"entry": "../app.html"}),
            ),
        ];
        let (handlers, apps, issues) = build_hosts(&contributions);
        assert_eq!(handlers.previewers().len(), 1);
        assert!(apps.is_empty());
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn workspace_media_source_rejects_escape_and_keeps_protocol_path_relative() {
        let root =
            std::env::temp_dir().join(format!("opentopia-media-handler-source-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::write(root.join("docs/readme.md"), b"hello").unwrap();

        let source = resolve_workspace_media_source(&root, FsPath::new("docs/readme.md")).unwrap();
        assert_eq!(path_for_protocol(&source.protocol_path), "docs/readme.md");
        assert!(resolve_workspace_media_source(&root, FsPath::new("../secret.txt")).is_err());
        assert!(resolve_workspace_media_source(&root, &root.join("docs/readme.md")).is_err());

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn registered_media_source_brokers_local_bytes_without_exposing_the_path() {
        let store = Arc::new(SqliteSessionStore::open(":memory:").expect("open store"));
        let workspace = std::env::temp_dir().join(format!(
            "opentopia-media-handler-resource-workspace-{}",
            Uuid::new_v4()
        ));
        let local_root = std::env::temp_dir().join(format!(
            "opentopia-media-handler-resource-local-{}",
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&workspace).expect("create workspace");
        std::fs::create_dir_all(&local_root).expect("create local directory");
        let local_path = local_root.join("outside.md");
        std::fs::write(&local_path, b"local resource").expect("write local resource");
        let thread = store
            .create_thread(Some("resource".to_string()), workspace.clone())
            .expect("create thread");
        let state = test_state(store);
        let lease = state.resources.register(
            thread.id,
            crate::resource_registry::ResourceLocator::Local {
                path: local_path.clone(),
            },
        );
        let resource_id = crate::resource_registry::resource_preview_id(lease.id);

        let source = resolve_registered_media_source(&state, thread.id, &resource_id)
            .expect("resolve registered source");
        assert_eq!(
            source.brokered_content.as_deref(),
            Some(b"local resource".as_slice())
        );
        assert_eq!(path_for_protocol(&source.protocol_path), "outside.md");
        assert!(!path_for_protocol(&source.protocol_path).contains(&path_for_protocol(&local_root)));

        std::fs::remove_dir_all(workspace).unwrap();
        std::fs::remove_dir_all(local_root).unwrap();
    }

    #[test]
    fn mcp_media_handler_requires_explicit_workspace_read_permission() {
        let mut handler = contribution(
            ContributionKind::ContextLoader,
            "markdown",
            json!({
                "extensions": ["md"],
                "runtime": "mcp.v1:documents/load_context"
            }),
        );
        assert!(ensure_workspace_read_permission(&handler).is_err());
        handler.permissions.push(PluginPermission::new(
            PluginPermissionKind::Filesystem,
            "workspace:read",
        ));
        assert!(ensure_workspace_read_permission(&handler).is_ok());
    }

    #[test]
    fn runtime_server_must_match_the_plugin_owned_definition() {
        let root = PathBuf::from("plugin-root");
        let definition = PluginMcpServerDefinition {
            name: "documents".to_string(),
            command: "document-server".to_string(),
            args: vec!["--stdio".to_string()],
            cwd: root.clone(),
            env_keys: vec!["DOCUMENT_TOKEN".to_string()],
            timeout_ms: 30_000,
        };
        let now = Utc::now();
        let server = McpServerConfig {
            server_id: Uuid::new_v4(),
            name: "plugin/documents".to_string(),
            command: definition.command.clone(),
            args: definition.args.clone(),
            cwd: Some(root),
            env_keys: definition.env_keys.clone(),
            timeout_ms: definition.timeout_ms,
            enabled: true,
            plugin_id: Some("plugin".to_string()),
            plugin_server_name: Some("documents".to_string()),
            created_at: now,
            updated_at: now,
        };
        assert!(server_matches_definition(&server, &definition));
        let mut foreign = server;
        foreign.command = "other-server".to_string();
        assert!(!server_matches_definition(&foreign, &definition));
    }

    #[tokio::test]
    async fn active_mcp_media_handler_invokes_the_owned_tool_and_records_health() {
        let fixture = MediaPluginFixture::new();
        let store = Arc::new(SqliteSessionStore::open(":memory:").expect("open store"));
        let thread = store
            .create_thread(None, fixture.workspace.clone())
            .expect("create thread");
        let plugin = fixture.plugin();
        store
            .set_plugin_activation(&plugin.id, &PluginControlScope::global(), true)
            .expect("activate media plugin");
        set_app_permissions(&store, &plugin, PluginPermissionGrantStatus::Granted);
        let host = McpExtensionHost::with_spawner(Arc::new(MediaHandlerMcpSpawner));
        let state = test_state_with_mcp(store.clone(), host);

        let Json(response) = invoke_media_handler(
            State(state),
            Path(thread.id),
            Json(InvokeMediaHandlerRequest {
                operation: MediaHandlerOperation::Preview,
                contribution_id: None,
                path: Some(PathBuf::from("sample.txt")),
                resource_id: None,
                content_type: Some("text/plain".to_string()),
                options: json!({}),
            }),
        )
        .await
        .expect("invoke active MCP media handler");

        assert_eq!(response.bytes_read, 5);
        assert_eq!(response.output.payload["receivedBase64"], "aGVsbG8=");
        assert!(matches!(
            response.runtime,
            MediaHandlerRuntime::McpV1 { .. }
        ));
        let health = store
            .list_plugin_runtime_health(&plugin.id)
            .expect("read handler health")
            .into_iter()
            .find(|record| record.contribution_id.ends_with("/text"))
            .expect("previewer health");
        assert_eq!(health.status, PluginRuntimeHealthStatus::Ready);
        assert!(health.last_error.is_none());
    }

    #[tokio::test]
    async fn app_content_and_messages_require_an_active_sandboxed_session() {
        let fixture = AppPluginFixture::new();
        let store = Arc::new(SqliteSessionStore::open(":memory:").expect("open store"));
        let thread = store
            .create_thread(None, fixture.workspace.clone())
            .expect("create thread");
        let plugin = fixture.plugin();
        store
            .set_plugin_activation(&plugin.id, &PluginControlScope::global(), true)
            .expect("activate app plugin");
        let state = test_state(store.clone());
        let contribution_id = format!("{}/dashboard", plugin.id);

        let error = start_app_session(
            State(state.clone()),
            Path(thread.id),
            Json(StartAppSessionRequest {
                contribution_id: contribution_id.clone(),
            }),
        )
        .await
        .expect_err("ungranted app must not start");
        assert_eq!(error.status, StatusCode::NOT_FOUND);

        set_app_permissions(&store, &plugin, PluginPermissionGrantStatus::Granted);
        let Json(started) = start_app_session(
            State(state.clone()),
            Path(thread.id),
            Json(StartAppSessionRequest { contribution_id }),
        )
        .await
        .expect("start active app");
        let session = started.session;
        assert_eq!(session.status, AppViewSessionStatus::Ready);
        assert!(!session.descriptor.sandbox.node_integration);
        assert!(!session.descriptor.sandbox.allow_popups);
        assert!(!session.descriptor.sandbox.allow_top_navigation);

        let response =
            read_app_content(State(state.clone()), Path((thread.id, session.session_id)))
                .await
                .expect("read active app content");
        assert_eq!(response.status(), StatusCode::OK);
        let csp = response
            .headers()
            .get(header::CONTENT_SECURITY_POLICY)
            .and_then(|value| value.to_str().ok())
            .expect("content security policy");
        assert!(csp.starts_with("sandbox allow-scripts;"));
        assert!(csp.contains("default-src 'none'"));
        assert!(csp.contains("connect-src 'none'"));
        assert!(!csp.contains("allow-same-origin"));
        let content = to_bytes(response.into_body(), MAX_APP_ENTRY_BYTES as usize)
            .await
            .expect("read response body");
        assert!(String::from_utf8_lossy(&content).contains("Sandboxed dashboard"));

        let _ = post_app_message(
            State(state.clone()),
            Path((thread.id, session.session_id)),
            Json(AppMessageRequest {
                channel: "refresh".to_string(),
                payload: json!({"page": 1}),
            }),
        )
        .await
        .expect("post message to active app");

        set_app_permissions(&store, &plugin, PluginPermissionGrantStatus::Revoked);
        let error = read_app_content(State(state.clone()), Path((thread.id, session.session_id)))
            .await
            .expect_err("revoked app content must be rejected");
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert_eq!(
            state
                .app_views
                .lock()
                .expect("app host lock")
                .session(session.session_id)
                .expect("session remains inspectable")
                .status,
            AppViewSessionStatus::Stopped
        );
        let error = post_app_message(
            State(state.clone()),
            Path((thread.id, session.session_id)),
            Json(AppMessageRequest {
                channel: "refresh".to_string(),
                payload: Value::Null,
            }),
        )
        .await
        .expect_err("revoked app message must be rejected");
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
    }
}
