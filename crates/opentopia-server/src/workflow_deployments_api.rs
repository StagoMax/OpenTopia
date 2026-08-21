use super::{ApiError, AppState};
use crate::flows_api::{ensure_enterprise, ensure_flow_thread, flow_error, flow_runtime_context};
use crate::workflow_compiler::compile_published_workflow;
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use opentopia_core::{
    spawn_flow_run, FlowRunV1, SessionStore, WorkflowCompileError, WorkflowDeploymentStatusV1,
    WorkflowDeploymentStoreError, WorkflowDeploymentV1, WorkflowOutputSpecV1,
    WorkflowTriggerSpecV1,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/workflow-deployments",
            get(list_workflow_deployments).post(create_workflow_deployment),
        )
        .route(
            "/api/workflow-deployments/:deployment_id",
            get(get_workflow_deployment),
        )
        .route(
            "/api/workflow-deployments/:deployment_id/disable",
            post(disable_workflow_deployment),
        )
        .route(
            "/api/threads/:thread_id/workflow-deployments/:deployment_id/runs",
            post(start_deployed_workflow_run),
        )
}

async fn list_workflow_deployments(
    State(state): State<AppState>,
    Query(query): Query<ListWorkflowDeploymentsQuery>,
) -> Result<Json<Vec<WorkflowDeploymentV1>>, ApiError> {
    ensure_enterprise(&state)?;
    let deployments = state
        .store
        .list_workflow_deployments(query.flow_id.as_deref(), query.status)
        .map_err(deployment_error)?;
    Ok(Json(deployments))
}

async fn get_workflow_deployment(
    State(state): State<AppState>,
    Path(deployment_id): Path<Uuid>,
) -> Result<Json<WorkflowDeploymentV1>, ApiError> {
    ensure_enterprise(&state)?;
    Ok(Json(deployment_for_request(&state, deployment_id)?))
}

async fn create_workflow_deployment(
    State(state): State<AppState>,
    Json(request): Json<CreateWorkflowDeploymentRequest>,
) -> Result<Json<WorkflowDeploymentV1>, ApiError> {
    ensure_enterprise(&state)?;
    let flow_id = request.flow_id.trim();
    if flow_id.is_empty() {
        return Err(ApiError::bad_request("flowId is required"));
    }
    let definition = state
        .store
        .get_flow_definition(flow_id, Some(request.flow_version))
        .map_err(flow_error)?
        .ok_or_else(|| ApiError::not_found("published Flow definition not found"))?;
    let compiled = compile_published_workflow(&state.store, &definition)
        .map_err(|error| ApiError::conflict(error.to_string()))?;
    let trigger = request.trigger.unwrap_or(WorkflowTriggerSpecV1::Manual);
    let output = request.output.unwrap_or(WorkflowOutputSpecV1::Inbox);
    trigger
        .validate()
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    output
        .validate()
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let deployment = WorkflowDeploymentV1::new_with_io(
        request.name,
        request.environment,
        compiled,
        trigger,
        output,
        request.created_by,
    )
    .map_err(workflow_compile_error)?;
    Ok(Json(
        state
            .store
            .insert_workflow_deployment(&deployment)
            .map_err(deployment_error)?,
    ))
}

async fn disable_workflow_deployment(
    State(state): State<AppState>,
    Path(deployment_id): Path<Uuid>,
    Json(request): Json<DisableWorkflowDeploymentRequest>,
) -> Result<Json<WorkflowDeploymentV1>, ApiError> {
    ensure_enterprise(&state)?;
    let mut deployment = deployment_for_request(&state, deployment_id)?;
    if deployment.revision != request.expected_revision {
        return Err(ApiError::conflict(format!(
            "Workflow deployment revision conflict; current revision is {}",
            deployment.revision
        )));
    }
    if deployment.status == WorkflowDeploymentStatusV1::Disabled {
        return Err(ApiError::conflict(
            "Workflow deployment is already disabled",
        ));
    }
    deployment.disable();
    Ok(Json(
        state
            .store
            .update_workflow_deployment(&deployment, request.expected_revision)
            .map_err(deployment_error)?,
    ))
}

async fn start_deployed_workflow_run(
    State(state): State<AppState>,
    Path((thread_id, deployment_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<StartDeployedWorkflowRunRequest>,
) -> Result<Json<FlowRunV1>, ApiError> {
    let thread = ensure_flow_thread(&state, thread_id)?;
    let deployment = deployment_for_request(&state, deployment_id)?;
    if deployment.status != WorkflowDeploymentStatusV1::Active {
        return Err(ApiError::conflict("Workflow deployment is not active"));
    }
    let run = FlowRunV1::new_from_deployment(thread_id, &deployment, request.input)
        .map_err(ApiError::from)?;
    let run = state.store.insert_flow_run(&run).map_err(flow_error)?;
    let context = flow_runtime_context(
        &state,
        &thread,
        run.id,
        run.harness_capabilities(),
        run.harness_connection_authority(),
        run.workflow_agent_specs(),
    )
    .await?;
    spawn_flow_run(run.id, context).map_err(ApiError::from)?;
    Ok(Json(run))
}

fn deployment_for_request(
    state: &AppState,
    deployment_id: Uuid,
) -> Result<WorkflowDeploymentV1, ApiError> {
    state
        .store
        .get_workflow_deployment(deployment_id)
        .map_err(deployment_error)?
        .ok_or_else(|| ApiError::not_found("Workflow deployment not found"))
}

fn workflow_compile_error(error: WorkflowCompileError) -> ApiError {
    ApiError::bad_request(error.to_string())
}

fn deployment_error(error: anyhow::Error) -> ApiError {
    if let Some(error) = error.downcast_ref::<WorkflowDeploymentStoreError>() {
        return match error {
            WorkflowDeploymentStoreError::NotFound(_) => ApiError::not_found(error.to_string()),
            WorkflowDeploymentStoreError::RevisionConflict(_) => {
                ApiError::conflict(error.to_string())
            }
        };
    }
    ApiError::internal(error.to_string())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListWorkflowDeploymentsQuery {
    #[serde(default)]
    flow_id: Option<String>,
    #[serde(default)]
    status: Option<WorkflowDeploymentStatusV1>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateWorkflowDeploymentRequest {
    flow_id: String,
    flow_version: u32,
    name: String,
    environment: String,
    created_by: String,
    #[serde(default)]
    trigger: Option<WorkflowTriggerSpecV1>,
    #[serde(default)]
    output: Option<WorkflowOutputSpecV1>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DisableWorkflowDeploymentRequest {
    expected_revision: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StartDeployedWorkflowRunRequest {
    input: Value,
}
