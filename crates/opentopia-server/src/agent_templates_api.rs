use super::{current_settings, ensure_experience_mode_enabled, ensure_thread, ApiError, AppState};
use axum::extract::{Path, Query, State};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use opentopia_core::{
    AgentInstanceStatusV1, AgentInstanceV1, AgentModelPolicyV1, AgentTemplateDiffV1,
    AgentTemplateError, AgentTemplateSpecV1, AgentTemplateStatusV1, AgentTemplateStoreError,
    AgentTemplateVersionV1, CapabilityProjection, ExecutionResourceGrantV1, ExperienceMode,
    ExperienceSurfaceProfile,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/agent-templates",
            get(list_agent_templates).post(create_agent_template_version),
        )
        .route(
            "/api/agent-templates/:template_id",
            delete(archive_agent_template),
        )
        .route(
            "/api/agent-templates/:template_id/versions/:version",
            get(get_agent_template_version).delete(delete_agent_template_version),
        )
        .route(
            "/api/agent-templates/:template_id/versions/:version/publish",
            post(publish_agent_template_version),
        )
        .route("/api/agent-instances", post(create_agent_instance))
        .route(
            "/api/agent-instances/:instance_id",
            get(get_agent_instance).patch(patch_agent_instance),
        )
        .route(
            "/api/threads/:thread_id/agent-instances",
            get(list_thread_agent_instances),
        )
        .route(
            "/api/threads/:thread_id/agent-instance",
            get(get_bound_thread_agent_instance),
        )
        .route(
            "/api/threads/:thread_id/agent-instance/:instance_id",
            put(bind_thread_agent_instance),
        )
}

async fn list_agent_templates(
    State(state): State<AppState>,
    Query(query): Query<AgentTemplateListQuery>,
) -> Result<Json<Vec<AgentTemplateVersionView>>, ApiError> {
    ensure_enterprise(&state)?;
    let templates = state
        .store
        .list_agent_template_versions(query.include_archived)
        .map_err(control_error)?;
    Ok(Json(template_views(templates)))
}

async fn create_agent_template_version(
    State(state): State<AppState>,
    Json(request): Json<CreateAgentTemplateVersionRequest>,
) -> Result<Json<AgentTemplateVersionView>, ApiError> {
    ensure_enterprise(&state)?;
    let template = state
        .store
        .create_agent_template_version(
            request.template_id,
            request.name,
            request.owner,
            request.spec,
        )
        .map_err(control_error)?;
    let previous = state
        .store
        .get_latest_published_agent_template(&template.template_id)
        .map_err(control_error)?;
    Ok(Json(AgentTemplateVersionView {
        diff: AgentTemplateDiffV1::between(previous.as_ref(), &template),
        template,
    }))
}

async fn get_agent_template_version(
    State(state): State<AppState>,
    Path((template_id, version)): Path<(String, u32)>,
) -> Result<Json<AgentTemplateVersionView>, ApiError> {
    ensure_enterprise(&state)?;
    let archived = state
        .store
        .agent_template_is_archived(&template_id)
        .map_err(control_error)?
        .ok_or_else(|| ApiError::not_found("Agent template not found"))?;
    if archived {
        return Err(ApiError::not_found("Agent template not found"));
    }
    let template = state
        .store
        .get_agent_template_version(&template_id, version)
        .map_err(control_error)?
        .ok_or_else(|| ApiError::not_found("Agent template version not found"))?;
    let all = state
        .store
        .list_agent_template_versions(false)
        .map_err(control_error)?;
    Ok(Json(template_view(&template, &all)))
}

async fn publish_agent_template_version(
    State(state): State<AppState>,
    Path((template_id, version)): Path<(String, u32)>,
    Json(request): Json<PublishAgentTemplateVersionRequest>,
) -> Result<Json<AgentTemplateVersionView>, ApiError> {
    ensure_enterprise(&state)?;
    let (template, diff) = state
        .store
        .publish_agent_template_version(
            &template_id,
            version,
            &request.approved_by,
            request.approve_capability_expansion,
        )
        .map_err(control_error)?;
    Ok(Json(AgentTemplateVersionView { template, diff }))
}

async fn delete_agent_template_version(
    State(state): State<AppState>,
    Path((template_id, version)): Path<(String, u32)>,
) -> Result<Json<AgentControlMutationResponse>, ApiError> {
    ensure_enterprise(&state)?;
    let deleted = state
        .store
        .delete_agent_template_version(&template_id, version)
        .map_err(control_error)?;
    if !deleted {
        return Err(ApiError::not_found("Agent template version not found"));
    }
    Ok(Json(AgentControlMutationResponse { ok: true }))
}

async fn archive_agent_template(
    State(state): State<AppState>,
    Path(template_id): Path<String>,
) -> Result<Json<AgentControlMutationResponse>, ApiError> {
    ensure_enterprise(&state)?;
    let archived = state
        .store
        .archive_agent_template(&template_id)
        .map_err(control_error)?;
    if !archived {
        return Err(ApiError::not_found("Agent template not found"));
    }
    Ok(Json(AgentControlMutationResponse { ok: true }))
}

async fn create_agent_instance(
    State(state): State<AppState>,
    Json(request): Json<CreateAgentInstanceRequest>,
) -> Result<Json<CreateAgentInstanceResponse>, ApiError> {
    ensure_enterprise(&state)?;
    let thread = ensure_thread(&state, request.thread_id)?;
    if thread.experience_mode != ExperienceMode::Flow {
        return Err(ApiError::bad_request(
            "enterprise Agent instances must be created from a Flow session",
        ));
    }
    if state
        .store
        .agent_template_is_archived(&request.template_id)
        .map_err(control_error)?
        != Some(false)
    {
        return Err(ApiError::not_found("Agent template not found"));
    }
    let template = match request.template_version {
        Some(version) => state
            .store
            .get_agent_template_version(&request.template_id, version)
            .map_err(control_error)?,
        None => state
            .store
            .get_latest_published_agent_template(&request.template_id)
            .map_err(control_error)?,
    }
    .ok_or_else(|| ApiError::not_found("published Agent template version not found"))?;

    let parent = request
        .parent_instance_id
        .map(|id| {
            state
                .store
                .get_agent_instance(id)
                .map_err(control_error)?
                .ok_or_else(|| ApiError::not_found("parent Agent instance not found"))
        })
        .transpose()?;
    let parent_template = parent
        .as_ref()
        .map(|parent| {
            state
                .store
                .get_agent_template_version(&parent.template_id, parent.template_version)
                .map_err(control_error)?
                .ok_or_else(|| ApiError::internal("parent Agent template version is missing"))
        })
        .transpose()?;
    let profile = ExperienceSurfaceProfile::for_mode(thread.experience_mode);
    let instance = AgentInstanceV1::instantiate(
        &template,
        thread.id,
        thread.experience_mode,
        &profile.capabilities,
        parent.as_ref(),
        parent_template.as_ref(),
        request.requested_capabilities.as_ref(),
        request.requested_resource_grants.as_deref(),
        request.requested_model_policy.as_ref(),
        request.initial_state,
    )
    .map_err(|error| control_error(error.into()))?;
    state
        .store
        .insert_agent_instance(&instance)
        .map_err(control_error)?;
    let bound = request.bind_to_thread && instance.parent_instance_id.is_none();
    if bound {
        state
            .store
            .bind_thread_agent_instance(thread.id, instance.id)
            .map_err(control_error)?;
    }
    Ok(Json(CreateAgentInstanceResponse { instance, bound }))
}

async fn get_agent_instance(
    State(state): State<AppState>,
    Path(instance_id): Path<Uuid>,
) -> Result<Json<AgentInstanceV1>, ApiError> {
    ensure_enterprise(&state)?;
    let instance = state
        .store
        .get_agent_instance(instance_id)
        .map_err(control_error)?
        .ok_or_else(|| ApiError::not_found("Agent instance not found"))?;
    ensure_thread(&state, instance.thread_id)?;
    Ok(Json(instance))
}

async fn list_thread_agent_instances(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<Vec<AgentInstanceV1>>, ApiError> {
    ensure_enterprise(&state)?;
    let thread = ensure_thread(&state, thread_id)?;
    if thread.experience_mode != ExperienceMode::Flow {
        return Err(ApiError::not_found("Flow session not found"));
    }
    Ok(Json(
        state
            .store
            .list_thread_agent_instances(thread_id)
            .map_err(control_error)?,
    ))
}

async fn get_bound_thread_agent_instance(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<Option<AgentInstanceV1>>, ApiError> {
    ensure_enterprise(&state)?;
    let thread = ensure_thread(&state, thread_id)?;
    if thread.experience_mode != ExperienceMode::Flow {
        return Err(ApiError::not_found("Flow session not found"));
    }
    Ok(Json(
        state
            .store
            .get_bound_thread_agent_instance(thread_id)
            .map_err(control_error)?,
    ))
}

async fn bind_thread_agent_instance(
    State(state): State<AppState>,
    Path((thread_id, instance_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<AgentInstanceV1>, ApiError> {
    ensure_enterprise(&state)?;
    let thread = ensure_thread(&state, thread_id)?;
    if thread.experience_mode != ExperienceMode::Flow {
        return Err(ApiError::not_found("Flow session not found"));
    }
    Ok(Json(
        state
            .store
            .bind_thread_agent_instance(thread_id, instance_id)
            .map_err(control_error)?,
    ))
}

async fn patch_agent_instance(
    State(state): State<AppState>,
    Path(instance_id): Path<Uuid>,
    Json(request): Json<PatchAgentInstanceRequest>,
) -> Result<Json<AgentInstanceV1>, ApiError> {
    ensure_enterprise(&state)?;
    let mut instance = state
        .store
        .get_agent_instance(instance_id)
        .map_err(control_error)?
        .ok_or_else(|| ApiError::not_found("Agent instance not found"))?;
    ensure_thread(&state, instance.thread_id)?;
    if request.state.is_some() && request.status.is_some() {
        return Err(ApiError::bad_request(
            "state and status updates must be submitted separately",
        ));
    }
    if request.state.is_none() && request.status.is_none() {
        return Err(ApiError::bad_request("Agent instance patch is empty"));
    }
    if let Some(next_state) = request.state {
        let expected_revision = request.expected_state_revision.ok_or_else(|| {
            ApiError::bad_request("expectedStateRevision is required when updating state")
        })?;
        instance = state
            .store
            .update_agent_instance_state(instance_id, expected_revision, next_state)
            .map_err(control_error)?;
    }
    if let Some(status) = request.status {
        if !valid_instance_status_transition(instance.status, status) {
            return Err(ApiError::conflict(
                "invalid Agent instance status transition",
            ));
        }
        instance = state
            .store
            .update_agent_instance_status(instance_id, status)
            .map_err(control_error)?;
    }
    Ok(Json(instance))
}

fn ensure_enterprise(state: &AppState) -> Result<(), ApiError> {
    ensure_experience_mode_enabled(&current_settings(state), ExperienceMode::Flow)
}

fn template_views(templates: Vec<AgentTemplateVersionV1>) -> Vec<AgentTemplateVersionView> {
    templates
        .iter()
        .map(|template| template_view(template, &templates))
        .collect()
}

fn template_view(
    template: &AgentTemplateVersionV1,
    templates: &[AgentTemplateVersionV1],
) -> AgentTemplateVersionView {
    let previous = templates
        .iter()
        .filter(|candidate| {
            candidate.template_id == template.template_id
                && candidate.status == AgentTemplateStatusV1::Published
                && candidate.version < template.version
        })
        .max_by_key(|candidate| candidate.version);
    AgentTemplateVersionView {
        template: template.clone(),
        diff: AgentTemplateDiffV1::between(previous, template),
    }
}

fn valid_instance_status_transition(
    current: AgentInstanceStatusV1,
    next: AgentInstanceStatusV1,
) -> bool {
    current == next
        || matches!(
            (current, next),
            (
                AgentInstanceStatusV1::Active,
                AgentInstanceStatusV1::Suspended
                    | AgentInstanceStatusV1::Completed
                    | AgentInstanceStatusV1::Revoked
            ) | (
                AgentInstanceStatusV1::Suspended,
                AgentInstanceStatusV1::Active
                    | AgentInstanceStatusV1::Completed
                    | AgentInstanceStatusV1::Revoked
            )
        )
}

fn control_error(error: anyhow::Error) -> ApiError {
    if let Some(error) = error.downcast_ref::<AgentTemplateStoreError>() {
        return match error {
            AgentTemplateStoreError::TemplateNotFound(_)
            | AgentTemplateStoreError::VersionNotFound { .. }
            | AgentTemplateStoreError::InstanceNotFound(_) => {
                ApiError::not_found(error.to_string())
            }
            AgentTemplateStoreError::TemplateArchived(_)
            | AgentTemplateStoreError::OwnerMismatch
            | AgentTemplateStoreError::PublishedVersionIsImmutable
            | AgentTemplateStoreError::StaleVersion
            | AgentTemplateStoreError::VersionInUse
            | AgentTemplateStoreError::StateRevisionConflict(_)
            | AgentTemplateStoreError::InvalidThreadBinding
            | AgentTemplateStoreError::InstanceThreadMismatch => {
                ApiError::conflict(error.to_string())
            }
        };
    }
    if let Some(error) = error.downcast_ref::<AgentTemplateError>() {
        return match error {
            AgentTemplateError::OwnerApprovalRequired
            | AgentTemplateError::DelegateTemplateDenied(_) => {
                ApiError::forbidden(error.to_string())
            }
            AgentTemplateError::VersionIsImmutable
            | AgentTemplateError::ContentHashMismatch
            | AgentTemplateError::CapabilityExpansionApprovalRequired
            | AgentTemplateError::TemplateNotPublished
            | AgentTemplateError::ParentInstanceNotActive
            | AgentTemplateError::ParentTemplateMismatch
            | AgentTemplateError::DelegationContextMismatch
            | AgentTemplateError::DelegationDepthExceeded
            | AgentTemplateError::InvalidInstanceContext
            | AgentTemplateError::InstanceCapabilityViolation => {
                ApiError::conflict(error.to_string())
            }
            _ => ApiError::bad_request(error.to_string()),
        };
    }
    ApiError::from(error)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentTemplateListQuery {
    #[serde(default)]
    include_archived: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateAgentTemplateVersionRequest {
    template_id: String,
    name: String,
    owner: String,
    spec: AgentTemplateSpecV1,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublishAgentTemplateVersionRequest {
    approved_by: String,
    #[serde(default)]
    approve_capability_expansion: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct AgentTemplateVersionView {
    template: AgentTemplateVersionV1,
    diff: AgentTemplateDiffV1,
}

#[derive(Debug, Serialize)]
struct AgentControlMutationResponse {
    ok: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateAgentInstanceRequest {
    template_id: String,
    template_version: Option<u32>,
    thread_id: Uuid,
    parent_instance_id: Option<Uuid>,
    requested_capabilities: Option<CapabilityProjection>,
    requested_resource_grants: Option<Vec<ExecutionResourceGrantV1>>,
    requested_model_policy: Option<AgentModelPolicyV1>,
    #[serde(default = "empty_object")]
    initial_state: Value,
    #[serde(default = "default_true")]
    bind_to_thread: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreateAgentInstanceResponse {
    instance: AgentInstanceV1,
    bound: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PatchAgentInstanceRequest {
    state: Option<Value>,
    expected_state_revision: Option<u64>,
    status: Option<AgentInstanceStatusV1>,
}

fn empty_object() -> Value {
    serde_json::json!({})
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_and_revoked_instances_are_terminal() {
        assert!(valid_instance_status_transition(
            AgentInstanceStatusV1::Active,
            AgentInstanceStatusV1::Suspended
        ));
        assert!(valid_instance_status_transition(
            AgentInstanceStatusV1::Suspended,
            AgentInstanceStatusV1::Active
        ));
        assert!(!valid_instance_status_transition(
            AgentInstanceStatusV1::Completed,
            AgentInstanceStatusV1::Active
        ));
        assert!(!valid_instance_status_transition(
            AgentInstanceStatusV1::Revoked,
            AgentInstanceStatusV1::Active
        ));
    }
}
