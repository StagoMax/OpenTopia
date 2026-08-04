use super::{current_settings, ensure_experience_mode_enabled, ensure_thread, ApiError, AppState};
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use opentopia_core::{
    simulate_flow, validate_flow_spec, CapabilityProjection, ExperienceMode,
    ExperienceSurfaceProfile, FlowDefinitionV1, FlowDraftStatusV1, FlowDraftV1, FlowSourceV1,
    FlowSpecV1, FlowStoreError, FlowTrialV1, SessionStore, TurnStatus,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/flows", get(search_flows))
        .route("/api/flows/:flow_id", get(get_flow))
        .route(
            "/api/threads/:thread_id/flow-drafts",
            get(list_flow_drafts).post(create_flow_draft),
        )
        .route(
            "/api/threads/:thread_id/flow-draft",
            get(get_thread_flow_draft),
        )
        .route(
            "/api/flow-drafts/:draft_id",
            get(get_flow_draft).put(update_flow_draft),
        )
        .route(
            "/api/flow-drafts/:draft_id/validate",
            post(validate_flow_draft),
        )
        .route(
            "/api/flow-drafts/:draft_id/simulate",
            post(simulate_flow_draft),
        )
        .route(
            "/api/flow-drafts/:draft_id/publish",
            post(publish_flow_draft),
        )
}

async fn search_flows(
    State(state): State<AppState>,
    Query(query): Query<SearchFlowsQuery>,
) -> Result<Json<Vec<FlowDefinitionV1>>, ApiError> {
    ensure_enterprise(&state)?;
    Ok(Json(
        state
            .store
            .search_flow_definitions(query.query.as_deref().unwrap_or_default())
            .map_err(flow_error)?,
    ))
}

async fn get_flow(
    State(state): State<AppState>,
    Path(flow_id): Path<String>,
    Query(query): Query<FlowVersionQuery>,
) -> Result<Json<FlowDefinitionV1>, ApiError> {
    ensure_enterprise(&state)?;
    state
        .store
        .get_flow_definition(&flow_id, query.version)
        .map_err(flow_error)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("Flow not found"))
}

async fn list_flow_drafts(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<Vec<FlowDraftView>>, ApiError> {
    ensure_flow_thread(&state, thread_id)?;
    let drafts = state
        .store
        .list_flow_drafts(Some(thread_id))
        .map_err(flow_error)?;
    let views = drafts
        .into_iter()
        .map(|draft| draft_view(&state, draft))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(views))
}

async fn get_thread_flow_draft(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<Option<FlowDraftView>>, ApiError> {
    ensure_flow_thread(&state, thread_id)?;
    let draft = state
        .store
        .get_thread_flow_draft(thread_id)
        .map_err(flow_error)?;
    Ok(Json(
        draft.map(|draft| draft_view(&state, draft)).transpose()?,
    ))
}

async fn create_flow_draft(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(request): Json<CreateFlowDraftRequest>,
) -> Result<Json<FlowDraftView>, ApiError> {
    ensure_flow_thread(&state, thread_id)?;
    if let FlowSourceV1::RunTrace { run_id, .. } = &request.spec.source {
        let run = state
            .store
            .get_turn(*run_id)
            .map_err(flow_error)?
            .ok_or_else(|| ApiError::not_found("source Run/Trace not found"))?;
        if run.status != TurnStatus::Succeeded {
            return Err(ApiError::conflict(
                "only a successful Run/Trace can be converted into a FlowDraft",
            ));
        }
    }
    let capabilities = flow_capabilities(&state, thread_id)?;
    let draft = FlowDraftV1::new(thread_id, request.spec, &capabilities);
    let draft = state.store.create_flow_draft(&draft).map_err(flow_error)?;
    Ok(Json(draft_view(&state, draft)?))
}

async fn get_flow_draft(
    State(state): State<AppState>,
    Path(draft_id): Path<Uuid>,
) -> Result<Json<FlowDraftView>, ApiError> {
    ensure_enterprise(&state)?;
    let draft = state
        .store
        .get_flow_draft(draft_id)
        .map_err(flow_error)?
        .ok_or_else(|| ApiError::not_found("Flow draft not found"))?;
    ensure_flow_thread(&state, draft.thread_id)?;
    Ok(Json(draft_view(&state, draft)?))
}

async fn update_flow_draft(
    State(state): State<AppState>,
    Path(draft_id): Path<Uuid>,
    Json(request): Json<UpdateFlowDraftRequest>,
) -> Result<Json<FlowDraftView>, ApiError> {
    ensure_enterprise(&state)?;
    let mut draft = state
        .store
        .get_flow_draft(draft_id)
        .map_err(flow_error)?
        .ok_or_else(|| ApiError::not_found("Flow draft not found"))?;
    ensure_flow_thread(&state, draft.thread_id)?;
    if draft.revision != request.expected_revision {
        return Err(ApiError::conflict(format!(
            "Flow draft revision conflict; current revision is {}",
            draft.revision
        )));
    }
    let capabilities = flow_capabilities(&state, draft.thread_id)?;
    draft.replace_spec(request.spec, &capabilities);
    let draft = state
        .store
        .update_flow_draft(&draft, request.expected_revision)
        .map_err(flow_error)?;
    Ok(Json(draft_view(&state, draft)?))
}

async fn validate_flow_draft(
    State(state): State<AppState>,
    Path(draft_id): Path<Uuid>,
) -> Result<Json<FlowDraftView>, ApiError> {
    ensure_enterprise(&state)?;
    let mut draft = state
        .store
        .get_flow_draft(draft_id)
        .map_err(flow_error)?
        .ok_or_else(|| ApiError::not_found("Flow draft not found"))?;
    ensure_flow_thread(&state, draft.thread_id)?;
    let expected_revision = draft.revision;
    let report = validate_flow_spec(&draft.spec, &flow_capabilities(&state, draft.thread_id)?);
    draft.status = if report.valid {
        FlowDraftStatusV1::ReadyToPublish
    } else {
        FlowDraftStatusV1::Reviewing
    };
    draft.last_validation = Some(report);
    draft.updated_at = Utc::now();
    let draft = state
        .store
        .update_flow_draft(&draft, expected_revision)
        .map_err(flow_error)?;
    Ok(Json(draft_view(&state, draft)?))
}

async fn simulate_flow_draft(
    State(state): State<AppState>,
    Path(draft_id): Path<Uuid>,
    Json(request): Json<SimulateFlowDraftRequest>,
) -> Result<Json<FlowTrialV1>, ApiError> {
    ensure_enterprise(&state)?;
    let mut draft = state
        .store
        .get_flow_draft(draft_id)
        .map_err(flow_error)?
        .ok_or_else(|| ApiError::not_found("Flow draft not found"))?;
    ensure_flow_thread(&state, draft.thread_id)?;
    let expected_revision = draft.revision;
    let trial = simulate_flow(
        &draft,
        request.input,
        &flow_capabilities(&state, draft.thread_id)?,
    );
    draft.last_validation = Some(trial.report.clone());
    draft.status = if trial.report.valid {
        FlowDraftStatusV1::ReadyToPublish
    } else {
        FlowDraftStatusV1::Reviewing
    };
    draft.updated_at = Utc::now();
    state
        .store
        .update_flow_draft(&draft, expected_revision)
        .map_err(flow_error)?;
    Ok(Json(
        state.store.insert_flow_trial(&trial).map_err(flow_error)?,
    ))
}

async fn publish_flow_draft(
    State(state): State<AppState>,
    Path(draft_id): Path<Uuid>,
    Json(request): Json<PublishFlowDraftRequest>,
) -> Result<Json<FlowDefinitionV1>, ApiError> {
    ensure_enterprise(&state)?;
    let draft = state
        .store
        .get_flow_draft(draft_id)
        .map_err(flow_error)?
        .ok_or_else(|| ApiError::not_found("Flow draft not found"))?;
    ensure_flow_thread(&state, draft.thread_id)?;
    if request.published_by.trim().is_empty() {
        return Err(ApiError::bad_request("publishedBy is required"));
    }
    Ok(Json(
        state
            .store
            .publish_flow_draft(draft_id, request.published_by.trim())
            .map_err(flow_error)?,
    ))
}

fn draft_view(state: &AppState, draft: FlowDraftV1) -> Result<FlowDraftView, ApiError> {
    let trials = state.store.list_flow_trials(draft.id).map_err(flow_error)?;
    Ok(FlowDraftView { draft, trials })
}

fn ensure_enterprise(state: &AppState) -> Result<(), ApiError> {
    ensure_experience_mode_enabled(&current_settings(state), ExperienceMode::Flow)
}

fn ensure_flow_thread(state: &AppState, thread_id: Uuid) -> Result<(), ApiError> {
    ensure_enterprise(state)?;
    let thread = ensure_thread(state, thread_id)?;
    if thread.experience_mode != ExperienceMode::Flow {
        return Err(ApiError::not_found("Flow session not found"));
    }
    Ok(())
}

fn flow_capabilities(state: &AppState, thread_id: Uuid) -> Result<CapabilityProjection, ApiError> {
    if let Some(instance) = state
        .store
        .get_bound_thread_agent_instance(thread_id)
        .map_err(flow_error)?
    {
        return Ok(instance.execution_context.capabilities);
    }
    Ok(ExperienceSurfaceProfile::for_mode(ExperienceMode::Flow).capabilities)
}

fn flow_error(error: anyhow::Error) -> ApiError {
    if let Some(error) = error.downcast_ref::<FlowStoreError>() {
        return match error {
            FlowStoreError::DraftNotFound(_) => ApiError::not_found(error.to_string()),
            FlowStoreError::RevisionConflict(_)
            | FlowStoreError::ValidationRequired
            | FlowStoreError::PassedTrialRequired
            | FlowStoreError::IndependentApproverRequired => ApiError::conflict(error.to_string()),
        };
    }
    ApiError::from(error)
}

#[derive(Debug, Deserialize)]
struct SearchFlowsQuery {
    query: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FlowVersionQuery {
    version: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct CreateFlowDraftRequest {
    spec: FlowSpecV1,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateFlowDraftRequest {
    expected_revision: u32,
    spec: FlowSpecV1,
}

#[derive(Debug, Deserialize)]
struct SimulateFlowDraftRequest {
    #[serde(default)]
    input: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublishFlowDraftRequest {
    published_by: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FlowDraftView {
    draft: FlowDraftV1,
    trials: Vec<FlowTrialV1>,
}
