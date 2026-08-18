use super::{
    current_settings, ensure_thread, load_bound_agent_context, publish_payload, truncate_chars,
    ApiError, AppState, DeleteResponse,
};
use axum::extract::{Path, Query, State};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use opentopia_core::{
    configured_provider_from_settings, AgentEventPayload, AppSettings, CanonicalModelRequest,
    CompiledModelContext, ContextAssembler, ContextAssemblyInput, ContextCacheScope,
    ContextItemKind, ContextRole, ContextSensitivity, DefaultContextAssembler, ExperienceMode,
    ExperienceSurfaceProfile, GoalSnapshot, GoalStatus, Message, ModelContextItem, ModelGateway,
    PromptCacheBreakpointPolicy, ProviderModelGateway, ProviderSettings, ProviderTransportKind,
    SessionStore, ThreadModelSelection,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path as FsPath, PathBuf};
use std::time::Duration;
use tokio::time::timeout;
use uuid::Uuid;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/threads", get(list_threads).post(create_thread))
        .route("/api/threads/:thread_id/title", post(generate_thread_title))
        .route(
            "/api/threads/:thread_id",
            patch(update_thread).delete(delete_thread),
        )
        .route("/api/projects", get(list_projects).post(create_project))
        .route(
            "/api/projects/:project_id",
            patch(update_project).delete(delete_project),
        )
        .route("/api/threads/:thread_id/messages", get(list_messages))
        .route("/api/threads/:thread_id/goal", get(get_thread_goal))
        .route("/api/threads/:thread_id/goal/:goal_id", patch(update_goal))
}

async fn list_threads(
    State(state): State<AppState>,
    Query(query): Query<ThreadListQuery>,
) -> Result<Json<Vec<opentopia_core::Thread>>, ApiError> {
    let enterprise_enabled = current_settings(&state).enterprise.enabled;
    if query.experience_mode == Some(ExperienceMode::Flow) && !enterprise_enabled {
        return Err(ApiError::forbidden(
            "Flow mode is disabled by the enterprise deployment boundary",
        ));
    }
    let mut threads = match query.experience_mode {
        Some(mode) => state
            .store
            .list_threads_for_mode(query.include_archived, mode)?,
        None => state
            .store
            .list_threads_including_archived(query.include_archived)?,
    };
    if !enterprise_enabled {
        threads.retain(|thread| thread.experience_mode != ExperienceMode::Flow);
    }
    Ok(Json(threads))
}

async fn create_thread(
    State(state): State<AppState>,
    Json(request): Json<CreateThreadRequest>,
) -> Result<Json<opentopia_core::Thread>, ApiError> {
    ensure_experience_mode_enabled(&current_settings(&state), request.experience_mode)?;
    let thread = if let Some(project_id) = request.project_id {
        state.store.create_thread_in_project_with_mode(
            request.title,
            project_id,
            request.experience_mode,
        )?
    } else if let Some(workspace_root) = request.workspace_root {
        let workspace_root = canonicalize_workspace_root(workspace_root);
        let project = state
            .store
            .find_or_create_project(project_name_for_workspace(&workspace_root), workspace_root)?;
        state.store.create_thread_in_project_with_mode(
            request.title,
            project.id,
            request.experience_mode,
        )?
    } else {
        let workspace_root = std::env::current_dir().map_err(anyhow::Error::from)?;
        state.store.create_thread_with_mode(
            request.title,
            workspace_root,
            request.experience_mode,
        )?
    };
    Ok(Json(thread))
}

pub(super) fn ensure_experience_mode_enabled(
    settings: &AppSettings,
    mode: ExperienceMode,
) -> Result<(), ApiError> {
    let profile = ExperienceSurfaceProfile::for_mode(mode);
    if profile.enterprise_only && !settings.enterprise.enabled {
        return Err(ApiError::forbidden(
            "Flow mode is disabled by the enterprise deployment boundary",
        ));
    }
    Ok(())
}

async fn generate_thread_title(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(request): Json<GenerateThreadTitleRequest>,
) -> Result<Json<GenerateThreadTitleResponse>, ApiError> {
    let current = state
        .store
        .get_thread(thread_id)?
        .ok_or_else(|| ApiError::not_found(format!("thread not found: {thread_id}")))?;
    if current.title != request.expected_title {
        return Ok(Json(GenerateThreadTitleResponse {
            thread: current,
            updated: false,
        }));
    }

    let settings = current_settings(&state);
    ensure_experience_mode_enabled(&settings, current.experience_mode)?;
    if let (Some(instance), _) = load_bound_agent_context(&state, &current)? {
        let provider = provider_settings_for_thread(&settings, current.model_selection.as_ref());
        if !instance
            .execution_context
            .model_policy
            .allows(&provider.id, &provider.model)
        {
            return Err(ApiError::forbidden(format!(
                "Agent template {}@{} does not allow model {}:{}",
                instance.template_id, instance.template_version, provider.id, provider.model
            )));
        }
    }

    let title =
        summarize_thread_title(&state, &request.prompt, current.model_selection.as_ref()).await?;
    let latest = state
        .store
        .get_thread(thread_id)?
        .ok_or_else(|| ApiError::not_found(format!("thread not found: {thread_id}")))?;
    if latest.title != request.expected_title {
        return Ok(Json(GenerateThreadTitleResponse {
            thread: latest,
            updated: false,
        }));
    }

    let thread = state
        .store
        .update_thread(thread_id, Some(title), None, None)?
        .ok_or_else(|| ApiError::not_found(format!("thread not found: {thread_id}")))?;
    Ok(Json(GenerateThreadTitleResponse {
        thread,
        updated: true,
    }))
}

async fn summarize_thread_title(
    state: &AppState,
    prompt: &str,
    model_selection: Option<&ThreadModelSelection>,
) -> Result<String, ApiError> {
    const TITLE_PROMPT_LIMIT: usize = 12_000;

    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err(ApiError::bad_request("thread title prompt cannot be empty"));
    }

    let settings = current_settings(state);
    let mut provider_settings = provider_settings_for_thread(&settings, model_selection);
    if provider_settings.effective_transport() == ProviderTransportKind::Mock {
        return Err(ApiError::bad_request(
            "thread title generation requires a configured model provider",
        ));
    }
    provider_settings.temperature = provider_settings
        .temperature
        .map(|temperature| temperature.min(0.2));
    provider_settings.max_output_tokens =
        Some(provider_settings.max_output_tokens.unwrap_or(64).min(64));
    // The title prompt is too short to justify an explicit cache write. Keep
    // this one-shot path out of the append-only user-anchor policy.
    provider_settings.prompt_cache_policy = None;
    let provider = configured_provider_from_settings(&provider_settings).ok_or_else(|| {
        ApiError::bad_request(format!(
            "provider '{}' has no configured API key",
            provider_settings.id
        ))
    })?;
    let system_prompt = format!(
            "Create a concise sidebar title for the user's first message. Use the same language as the user and preserve specific product, file, and error names. Return only the title: no quotes, Markdown, label, or trailing punctuation. The title must contain at most {MAX_THREAD_TITLE_CHARS} Unicode characters."
        );
    let request = assemble_one_shot_model_request(
        "opentopia:thread_title",
        &system_prompt,
        truncate_chars(prompt, TITLE_PROMPT_LIMIT),
        None,
    )
    .map_err(|error| ApiError::internal(format!("title request assembly failed: {error}")))?;
    let gateway = ProviderModelGateway::from_provider(provider);
    let prepared = gateway.prepare(Uuid::new_v4(), request).map_err(|error| {
        ApiError::bad_gateway(format!("title request encoding failed: {error}"))
    })?;
    let response = timeout(
        Duration::from_secs(45),
        gateway.stream_prepared(prepared, &mut |_| Ok(()), &mut |_| Ok(())),
    )
    .await
    .map_err(|_| ApiError::gateway_timeout("thread title generation timed out"))?
    .map_err(|error| ApiError::bad_gateway(format!("thread title generation failed: {error}")))?;
    normalize_generated_thread_title(&response.text)
        .ok_or_else(|| ApiError::bad_gateway("thread title provider returned an empty title"))
}

pub(super) fn assemble_one_shot_model_request(
    source: &str,
    system_prompt: &str,
    user_message: String,
    final_output_json_schema: Option<Value>,
) -> anyhow::Result<CanonicalModelRequest> {
    let context = CompiledModelContext {
        items: vec![ModelContextItem::text(
            ContextItemKind::BaseInstructions,
            ContextRole::System,
            source,
            system_prompt,
            ContextCacheScope::Stable,
            ContextSensitivity::Public,
        )],
        prompt_cache_key: None,
    };
    DefaultContextAssembler.compile(ContextAssemblyInput {
        model_context: &context,
        context_summary: None,
        conversation: Vec::new(),
        user_message,
        user_content: Vec::new(),
        tool_candidates: Vec::new(),
        previous_tool_calls: Vec::new(),
        tool_results: Vec::new(),
        previous_response_items: Vec::new(),
        previous_response_id: None,
        branch_developer_instructions: None,
        prompt_cache_breakpoint_policy: PromptCacheBreakpointPolicy::StableOnly,
        final_output_json_schema,
    })
}

pub(super) const MAX_THREAD_TITLE_CHARS: usize = 50;

pub(super) fn provider_settings_for_thread(
    settings: &AppSettings,
    model_selection: Option<&ThreadModelSelection>,
) -> ProviderSettings {
    let connection = settings.provider_by_id_or_active(
        model_selection.map(|selection| selection.connection_id.as_str()),
    );
    match model_selection {
        Some(selection) => connection.with_model_route_override(
            Some(selection.model_id.as_str()),
            Some(selection.reasoning_effort.as_deref()),
            None,
        ),
        None => connection.clone(),
    }
}

pub(super) fn normalize_generated_thread_title(response: &str) -> Option<String> {
    response.lines().find_map(|line| {
        let mut title = line.trim();
        if title.is_empty() || title == "```" {
            return None;
        }
        title = title.trim_start_matches(['#', '-', '*', ' ']);
        for prefix in ["Title:", "Title：", "标题:", "标题："] {
            if let Some(value) = title.strip_prefix(prefix) {
                title = value.trim();
                break;
            }
        }
        title = title
            .trim_matches('`')
            .trim_matches('*')
            .trim_matches('"')
            .trim_matches('\'')
            .trim_matches('“')
            .trim_matches('”')
            .trim_matches('「')
            .trim_matches('」')
            .trim();
        if title.is_empty() {
            return None;
        }
        let chars = title.chars().collect::<Vec<_>>();
        if chars.len() <= MAX_THREAD_TITLE_CHARS {
            return Some(title.to_string());
        }
        let mut shortened = chars
            .into_iter()
            .take(MAX_THREAD_TITLE_CHARS - 1)
            .collect::<String>();
        shortened.push('…');
        Some(shortened)
    })
}

async fn update_thread(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(request): Json<UpdateThreadRequest>,
) -> Result<Json<opentopia_core::Thread>, ApiError> {
    let archived = request.archived.or_else(|| match request.archived_at {
        PatchValue::Missing => None,
        PatchValue::Null => Some(false),
        PatchValue::Value(_) => Some(true),
    });
    let project_id = match request.project_id {
        PatchValue::Missing => None,
        PatchValue::Null => Some(None),
        PatchValue::Value(project_id) => Some(Some(project_id)),
    };
    let thread = state
        .store
        .update_thread(thread_id, request.title, project_id, archived)?
        .ok_or_else(|| ApiError::not_found(format!("thread not found: {thread_id}")))?;
    Ok(Json(thread))
}

async fn delete_thread(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<DeleteResponse>, ApiError> {
    let deleted = state.store.delete_thread(thread_id)?;
    if !deleted {
        return Err(ApiError::not_found(format!(
            "thread not found: {thread_id}"
        )));
    }
    state.resources.release_thread(thread_id);
    Ok(Json(DeleteResponse { deleted }))
}

async fn list_projects(
    State(state): State<AppState>,
) -> Result<Json<Vec<opentopia_core::Project>>, ApiError> {
    Ok(Json(state.store.list_projects()?))
}

async fn create_project(
    State(state): State<AppState>,
    Json(request): Json<CreateProjectRequest>,
) -> Result<Json<opentopia_core::Project>, ApiError> {
    let workspace_root = request.workspace_root.map(canonicalize_workspace_root);
    let project = state.store.create_project(
        request.name,
        workspace_root,
        request.pinned.unwrap_or(false),
        request.sort_order.unwrap_or(0),
    )?;
    Ok(Json(project))
}

async fn update_project(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Json(request): Json<UpdateProjectRequest>,
) -> Result<Json<opentopia_core::Project>, ApiError> {
    let workspace_root = match request.workspace_root {
        PatchValue::Missing => None,
        PatchValue::Null => Some(None),
        PatchValue::Value(path) => Some(Some(canonicalize_workspace_root(path))),
    };
    let project = state
        .store
        .update_project(
            project_id,
            request.name,
            workspace_root,
            request.pinned,
            request.sort_order,
        )?
        .ok_or_else(|| ApiError::not_found(format!("project not found: {project_id}")))?;
    Ok(Json(project))
}

async fn delete_project(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
) -> Result<Json<DeleteResponse>, ApiError> {
    let deleted = state.store.delete_project(project_id)?;
    if !deleted {
        return Err(ApiError::not_found(format!(
            "project not found: {project_id}"
        )));
    }
    Ok(Json(DeleteResponse { deleted }))
}

fn canonicalize_workspace_root(workspace_root: PathBuf) -> PathBuf {
    workspace_root.canonicalize().unwrap_or(workspace_root)
}

fn project_name_for_workspace(workspace_root: &FsPath) -> String {
    workspace_root
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .rsplit('/')
        .find(|part| !part.is_empty())
        .filter(|part| *part != ".")
        .unwrap_or("Workspace")
        .to_string()
}

async fn list_messages(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<Vec<Message>>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let store = state.store.clone();
    let messages = tokio::task::spawn_blocking(move || store.list_messages(thread_id))
        .await
        .map_err(|error| ApiError::internal(format!("message history task failed: {error}")))??;
    Ok(Json(messages))
}

async fn get_thread_goal(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<Option<GoalSnapshot>>, ApiError> {
    ensure_thread(&state, thread_id)?;
    Ok(Json(state.store.get_thread_goal(thread_id)?))
}

async fn update_goal(
    State(state): State<AppState>,
    Path((thread_id, goal_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<UpdateGoalRequest>,
) -> Result<Json<GoalSnapshot>, ApiError> {
    ensure_thread(&state, thread_id)?;
    if request.status.is_none()
        && request.objective.is_none()
        && request.constraints.is_none()
        && request.acceptance.is_none()
    {
        return Err(ApiError::bad_request("goal update contains no changes"));
    }
    let mut snapshot = if request.objective.is_some()
        || request.constraints.is_some()
        || request.acceptance.is_some()
    {
        state
            .store
            .update_goal_definition(
                thread_id,
                goal_id,
                request.objective,
                request.constraints,
                request.acceptance,
            )?
            .ok_or_else(|| ApiError::not_found(format!("goal not found: {goal_id}")))?
    } else {
        state
            .store
            .get_goal(goal_id)?
            .filter(|snapshot| snapshot.goal.thread_id == thread_id)
            .ok_or_else(|| ApiError::not_found(format!("goal not found: {goal_id}")))?
    };
    if let Some(status) = request.status {
        if !matches!(
            status,
            GoalStatus::Active | GoalStatus::Paused | GoalStatus::Cancelled
        ) {
            return Err(ApiError::bad_request(
                "clients may only start, pause, resume, or cancel a goal",
            ));
        }
        snapshot = state
            .store
            .update_goal_status(thread_id, goal_id, status)?
            .ok_or_else(|| ApiError::not_found(format!("goal not found: {goal_id}")))?;
    }
    publish_payload(
        &state,
        thread_id,
        None,
        AgentEventPayload::GoalUpdated {
            snapshot: snapshot.clone(),
        },
    );
    Ok(Json(snapshot))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreateThreadRequest {
    title: Option<String>,
    workspace_root: Option<PathBuf>,
    pub(super) project_id: Option<Uuid>,
    #[serde(default)]
    experience_mode: ExperienceMode,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GenerateThreadTitleRequest {
    prompt: String,
    expected_title: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct GenerateThreadTitleResponse {
    thread: opentopia_core::Thread,
    updated: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadListQuery {
    #[serde(default)]
    include_archived: bool,
    experience_mode: Option<ExperienceMode>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UpdateThreadRequest {
    title: Option<String>,
    #[serde(default)]
    pub(super) project_id: PatchValue<Uuid>,
    archived: Option<bool>,
    #[serde(default)]
    pub(super) archived_at: PatchValue<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateProjectRequest {
    name: String,
    workspace_root: Option<PathBuf>,
    pinned: Option<bool>,
    sort_order: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UpdateProjectRequest {
    name: Option<String>,
    #[serde(default)]
    pub(super) workspace_root: PatchValue<PathBuf>,
    pinned: Option<bool>,
    pub(super) sort_order: Option<i64>,
}

#[derive(Debug)]
pub(super) enum PatchValue<T> {
    Missing,
    Null,
    Value(T),
}

impl<T> Default for PatchValue<T> {
    fn default() -> Self {
        Self::Missing
    }
}

impl<'de, T> Deserialize<'de> for PatchValue<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(match Option::<T>::deserialize(deserializer)? {
            Some(value) => Self::Value(value),
            None => Self::Null,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateGoalRequest {
    #[serde(default)]
    status: Option<GoalStatus>,
    #[serde(default)]
    objective: Option<String>,
    #[serde(default)]
    constraints: Option<Vec<String>>,
    #[serde(default)]
    acceptance: Option<Vec<String>>,
}
