use super::{ApiError, AppState};
use crate::auth::constant_time_eq;
use crate::flows_api::{ensure_enterprise, ensure_flow_thread, flow_error};
use crate::workflow_automation_service::{
    evaluation_summary, start_pending_release_invocation, start_release_invocation,
    workflow_automation_error, WorkflowEvaluationSummary, WorkflowInvocationResult,
};
use crate::workflow_delivery::deliver_run_output;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{Duration, Utc};
use opentopia_core::{
    SessionStore, WorkflowDeliveryReceiptV1, WorkflowDeliveryStatusV1, WorkflowDeploymentStatusV1,
    WorkflowEvaluationV1, WorkflowIngressPolicyV1, WorkflowReleaseStatusV1, WorkflowReleaseV1,
    WorkflowTriggerInvocationV1, WorkflowTriggerSpecV1,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/workflow-releases",
            get(list_workflow_releases).post(create_workflow_release),
        )
        .route(
            "/api/workflow-releases/:release_id",
            get(get_workflow_release),
        )
        .route(
            "/api/workflow-releases/:release_id/invoke",
            post(invoke_workflow_release),
        )
        .route(
            "/api/workflow-releases/:release_id/canary",
            post(set_workflow_release_canary),
        )
        .route(
            "/api/workflow-releases/:release_id/promote",
            post(promote_workflow_release),
        )
        .route(
            "/api/workflow-releases/:release_id/rollback",
            post(rollback_workflow_release),
        )
        .route(
            "/api/workflow-releases/:release_id/disable",
            post(disable_workflow_release),
        )
        .route("/api/workflow-events", post(dispatch_workflow_event))
        .route(
            "/api/workflow-trigger-invocations",
            get(list_workflow_trigger_invocations),
        )
        .route(
            "/api/workflow-trigger-invocations/:invocation_id/start",
            post(start_pending_workflow_invocation),
        )
        .route(
            "/api/workflow-delivery-receipts",
            get(list_workflow_delivery_receipts),
        )
        .route(
            "/api/workflow-delivery-receipts/:receipt_id/retry",
            post(retry_workflow_delivery),
        )
        .route(
            "/api/workflow-evaluations",
            get(list_workflow_evaluations).post(create_workflow_evaluation),
        )
        .route(
            "/api/workflow-evaluation-summary",
            get(get_workflow_evaluation_summary),
        )
        .route(
            "/hooks/workflows/:trigger_id",
            post(invoke_workflow_webhook),
        )
}

async fn list_workflow_releases(
    State(state): State<AppState>,
    Query(query): Query<ListWorkflowReleasesQuery>,
) -> Result<Json<Vec<WorkflowReleaseV1>>, ApiError> {
    ensure_enterprise(&state)?;
    Ok(Json(
        state
            .store
            .list_workflow_releases(query.status)
            .map_err(workflow_automation_error)?,
    ))
}

async fn get_workflow_release(
    State(state): State<AppState>,
    Path(release_id): Path<Uuid>,
) -> Result<Json<WorkflowReleaseV1>, ApiError> {
    ensure_enterprise(&state)?;
    Ok(Json(release_for_request(&state, release_id)?))
}

async fn create_workflow_release(
    State(state): State<AppState>,
    Json(request): Json<CreateWorkflowReleaseRequest>,
) -> Result<Json<WorkflowReleaseV1>, ApiError> {
    ensure_enterprise(&state)?;
    ensure_flow_thread(&state, request.thread_id)?;
    let deployment = state
        .store
        .get_workflow_deployment(request.deployment_id)
        .map_err(flow_error)?
        .ok_or_else(|| ApiError::not_found("Workflow deployment not found"))?;
    let release = WorkflowReleaseV1::new_with_ingress_policy(
        request.release_key,
        request.environment,
        request.thread_id,
        &deployment,
        request.trigger,
        request.ingress_policy,
        request.created_by,
    )
    .map_err(|error| ApiError::bad_request(error.to_string()))?;
    state
        .store
        .insert_workflow_release(&release)
        .map(Json)
        .map_err(|error| ApiError::conflict(error.to_string()))
}

async fn invoke_workflow_release(
    State(state): State<AppState>,
    Path(release_id): Path<Uuid>,
    Json(request): Json<InvokeWorkflowReleaseRequest>,
) -> Result<Json<WorkflowInvocationResult>, ApiError> {
    ensure_enterprise(&state)?;
    let release = release_for_request(&state, release_id)?;
    Ok(Json(
        start_release_invocation(&state, &release, request.idempotency_key, request.input).await?,
    ))
}

async fn set_workflow_release_canary(
    State(state): State<AppState>,
    Path(release_id): Path<Uuid>,
    Json(request): Json<SetWorkflowReleaseCanaryRequest>,
) -> Result<Json<WorkflowReleaseV1>, ApiError> {
    ensure_enterprise(&state)?;
    let mut release = release_for_request(&state, release_id)?;
    ensure_release_revision(&release, request.expected_revision)?;
    let deployment = state
        .store
        .get_workflow_deployment(request.deployment_id)
        .map_err(flow_error)?
        .ok_or_else(|| ApiError::not_found("Canary deployment not found"))?;
    release
        .set_canary(&deployment, request.percent)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    Ok(Json(
        state
            .store
            .update_workflow_release(&release, request.expected_revision)
            .map_err(workflow_automation_error)?,
    ))
}

async fn promote_workflow_release(
    State(state): State<AppState>,
    Path(release_id): Path<Uuid>,
    Json(request): Json<ReleaseRevisionRequest>,
) -> Result<Json<WorkflowReleaseV1>, ApiError> {
    ensure_enterprise(&state)?;
    let mut release = release_for_request(&state, release_id)?;
    ensure_release_revision(&release, request.expected_revision)?;
    ensure_release_target_active(
        &state,
        release
            .canary_deployment_id
            .ok_or_else(|| ApiError::conflict("release has no canary to promote"))?,
        &release.environment,
    )?;
    release
        .promote_canary()
        .map_err(|error| ApiError::conflict(error.to_string()))?;
    Ok(Json(
        state
            .store
            .update_workflow_release(&release, request.expected_revision)
            .map_err(workflow_automation_error)?,
    ))
}

async fn rollback_workflow_release(
    State(state): State<AppState>,
    Path(release_id): Path<Uuid>,
    Json(request): Json<ReleaseRevisionRequest>,
) -> Result<Json<WorkflowReleaseV1>, ApiError> {
    ensure_enterprise(&state)?;
    let mut release = release_for_request(&state, release_id)?;
    ensure_release_revision(&release, request.expected_revision)?;
    ensure_release_target_active(
        &state,
        release
            .previous_primary_deployment_id
            .ok_or_else(|| ApiError::conflict("release has no previous primary to restore"))?,
        &release.environment,
    )?;
    release
        .rollback()
        .map_err(|error| ApiError::conflict(error.to_string()))?;
    Ok(Json(
        state
            .store
            .update_workflow_release(&release, request.expected_revision)
            .map_err(workflow_automation_error)?,
    ))
}

async fn disable_workflow_release(
    State(state): State<AppState>,
    Path(release_id): Path<Uuid>,
    Json(request): Json<ReleaseRevisionRequest>,
) -> Result<Json<WorkflowReleaseV1>, ApiError> {
    ensure_enterprise(&state)?;
    let mut release = release_for_request(&state, release_id)?;
    ensure_release_revision(&release, request.expected_revision)?;
    release.disable();
    Ok(Json(
        state
            .store
            .update_workflow_release(&release, request.expected_revision)
            .map_err(workflow_automation_error)?,
    ))
}

async fn invoke_workflow_webhook(
    State(state): State<AppState>,
    Path(trigger_id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Result<Json<WorkflowInvocationResult>, ApiError> {
    ensure_enterprise(&state)?;
    let release = state
        .store
        .get_workflow_release_by_trigger(trigger_id)
        .map_err(workflow_automation_error)?
        .ok_or_else(|| ApiError::not_found("Workflow trigger not found"))?;
    let WorkflowTriggerSpecV1::Webhook { token_ref, .. } = &release.trigger else {
        return Err(ApiError::not_found("Workflow webhook trigger not found"));
    };
    let env_name = token_ref
        .strip_prefix("env:")
        .ok_or_else(|| ApiError::forbidden("Workflow trigger is not configured"))?;
    let expected = std::env::var(env_name)
        .map_err(|_| ApiError::forbidden("Workflow trigger is not configured"))?;
    let provided = headers
        .get("x-opentopia-trigger-token")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if expected.is_empty() || !constant_time_eq(provided.as_bytes(), expected.as_bytes()) {
        return Err(ApiError::forbidden("invalid Workflow trigger token"));
    }
    let idempotency_key = required_header(&headers, "idempotency-key")?;
    if state
        .store
        .get_workflow_trigger_invocation(release.id, &idempotency_key)
        .map_err(workflow_automation_error)?
        .is_none()
    {
        let limit = std::env::var("OPENTOPIA_WORKFLOW_WEBHOOK_RATE_LIMIT_PER_MINUTE")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(120)
            .clamp(1, 10_000);
        let recent = state
            .store
            .count_recent_workflow_trigger_invocations(
                trigger_id,
                Utc::now() - Duration::minutes(1),
            )
            .map_err(workflow_automation_error)?;
        if recent >= limit {
            return Err(ApiError::too_many_requests(
                "Workflow trigger rate limit exceeded",
            ));
        }
    }
    Ok(Json(
        start_release_invocation(&state, &release, idempotency_key, input).await?,
    ))
}

async fn dispatch_workflow_event(
    State(state): State<AppState>,
    Json(request): Json<DispatchWorkflowEventRequest>,
) -> Result<Json<Vec<WorkflowInvocationResult>>, ApiError> {
    ensure_enterprise(&state)?;
    let source = request.source.trim();
    let event_type = request.event_type.trim();
    if source.is_empty() || event_type.is_empty() || request.idempotency_key.trim().is_empty() {
        return Err(ApiError::bad_request(
            "source, eventType, and idempotencyKey are required",
        ));
    }
    let releases = state
        .store
        .list_workflow_releases(Some(WorkflowReleaseStatusV1::Active))
        .map_err(workflow_automation_error)?;
    let mut results = Vec::new();
    for release in releases {
        let WorkflowTriggerSpecV1::EventSubscription {
            source: configured_source,
            event_type: configured_type,
            ..
        } = &release.trigger
        else {
            continue;
        };
        if configured_source != source || configured_type != event_type {
            continue;
        }
        let key = format!("event:{source}:{event_type}:{}", request.idempotency_key);
        results
            .push(start_release_invocation(&state, &release, key, request.payload.clone()).await?);
    }
    Ok(Json(results))
}

async fn list_workflow_trigger_invocations(
    State(state): State<AppState>,
    Query(query): Query<ListWorkflowInvocationsQuery>,
) -> Result<Json<Vec<WorkflowTriggerInvocationV1>>, ApiError> {
    ensure_enterprise(&state)?;
    Ok(Json(
        state
            .store
            .list_workflow_trigger_invocations(query.release_id)
            .map_err(workflow_automation_error)?,
    ))
}

async fn start_pending_workflow_invocation(
    State(state): State<AppState>,
    Path(invocation_id): Path<Uuid>,
) -> Result<Json<WorkflowInvocationResult>, ApiError> {
    ensure_enterprise(&state)?;
    Ok(Json(
        start_pending_release_invocation(&state, invocation_id).await?,
    ))
}

async fn list_workflow_delivery_receipts(
    State(state): State<AppState>,
    Query(query): Query<ListWorkflowDeliveryReceiptsQuery>,
) -> Result<Json<Vec<WorkflowDeliveryReceiptV1>>, ApiError> {
    ensure_enterprise(&state)?;
    Ok(Json(
        state
            .store
            .list_workflow_delivery_receipts(query.deployment_id, query.status)
            .map_err(workflow_automation_error)?,
    ))
}

async fn retry_workflow_delivery(
    State(state): State<AppState>,
    Path(receipt_id): Path<Uuid>,
    Json(request): Json<RetryWorkflowDeliveryRequest>,
) -> Result<Json<WorkflowDeliveryReceiptV1>, ApiError> {
    ensure_enterprise(&state)?;
    let receipt = state
        .store
        .get_workflow_delivery_receipt(receipt_id)
        .map_err(workflow_automation_error)?
        .ok_or_else(|| ApiError::not_found("DeliveryReceipt not found"))?;
    if receipt.revision != request.expected_revision {
        return Err(ApiError::conflict(format!(
            "DeliveryReceipt revision conflict; current revision is {}",
            receipt.revision
        )));
    }
    if !matches!(
        receipt.status,
        WorkflowDeliveryStatusV1::Failed | WorkflowDeliveryStatusV1::Pending
    ) {
        return Err(ApiError::conflict("DeliveryReceipt is not retryable"));
    }
    let run = state
        .store
        .get_flow_run(receipt.run_id)
        .map_err(flow_error)?
        .ok_or_else(|| ApiError::not_found("Flow run not found"))?;
    Ok(Json(deliver_run_output(&state, &run, true).await.map_err(
        |error| ApiError::bad_gateway(error.to_string()),
    )?))
}

async fn list_workflow_evaluations(
    State(state): State<AppState>,
    Query(query): Query<ListWorkflowEvaluationsQuery>,
) -> Result<Json<Vec<WorkflowEvaluationV1>>, ApiError> {
    ensure_enterprise(&state)?;
    Ok(Json(
        state
            .store
            .list_workflow_evaluations(query.deployment_id)
            .map_err(workflow_automation_error)?,
    ))
}

async fn create_workflow_evaluation(
    State(state): State<AppState>,
    Json(request): Json<CreateWorkflowEvaluationRequest>,
) -> Result<Json<WorkflowEvaluationV1>, ApiError> {
    ensure_enterprise(&state)?;
    let run = state
        .store
        .get_flow_run(request.run_id)
        .map_err(flow_error)?
        .ok_or_else(|| ApiError::not_found("Flow run not found"))?;
    let deployment_id = run
        .deployment_id
        .ok_or_else(|| ApiError::bad_request("Flow run is not deployed"))?;
    let evaluation = WorkflowEvaluationV1::new(
        run.id,
        deployment_id,
        request.evaluator,
        request.score,
        request.passed,
        request.labels,
        request.note,
    )
    .map_err(|error| ApiError::bad_request(error.to_string()))?;
    if let Some(existing) = state
        .store
        .get_workflow_evaluation(run.id, &evaluation.evaluator)
        .map_err(workflow_automation_error)?
    {
        if existing.run_id == evaluation.run_id
            && existing.deployment_id == evaluation.deployment_id
            && existing.evaluator == evaluation.evaluator
            && existing.score == evaluation.score
            && existing.passed == evaluation.passed
            && existing.labels == evaluation.labels
            && existing.note == evaluation.note
        {
            return Ok(Json(existing));
        }
        return Err(ApiError::conflict(
            "evaluation already exists for this run and evaluator",
        ));
    }
    Ok(Json(
        state
            .store
            .insert_workflow_evaluation(&evaluation)
            .map_err(workflow_automation_error)?,
    ))
}

async fn get_workflow_evaluation_summary(
    State(state): State<AppState>,
    Query(query): Query<GetWorkflowEvaluationSummaryQuery>,
) -> Result<Json<WorkflowEvaluationSummary>, ApiError> {
    ensure_enterprise(&state)?;
    Ok(Json(evaluation_summary(&state, query.deployment_id)?))
}

fn release_for_request(state: &AppState, release_id: Uuid) -> Result<WorkflowReleaseV1, ApiError> {
    state
        .store
        .get_workflow_release(release_id)
        .map_err(workflow_automation_error)?
        .ok_or_else(|| ApiError::not_found("Workflow release not found"))
}

fn ensure_release_revision(
    release: &WorkflowReleaseV1,
    expected_revision: u32,
) -> Result<(), ApiError> {
    if release.revision != expected_revision {
        return Err(ApiError::conflict(format!(
            "Workflow release revision conflict; current revision is {}",
            release.revision
        )));
    }
    Ok(())
}

fn ensure_release_target_active(
    state: &AppState,
    deployment_id: Uuid,
    environment: &str,
) -> Result<(), ApiError> {
    let deployment = state
        .store
        .get_workflow_deployment(deployment_id)
        .map_err(flow_error)?
        .ok_or_else(|| ApiError::not_found("Workflow deployment not found"))?;
    if deployment.status != WorkflowDeploymentStatusV1::Active
        || deployment.environment != environment
    {
        return Err(ApiError::conflict(
            "release target must be active in the same environment",
        ));
    }
    Ok(())
}

fn required_header(headers: &HeaderMap, name: &str) -> Result<String, ApiError> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| ApiError::bad_request(format!("{name} header is required")))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListWorkflowReleasesQuery {
    #[serde(default)]
    status: Option<WorkflowReleaseStatusV1>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateWorkflowReleaseRequest {
    release_key: String,
    environment: String,
    thread_id: Uuid,
    deployment_id: Uuid,
    trigger: WorkflowTriggerSpecV1,
    #[serde(default)]
    ingress_policy: WorkflowIngressPolicyV1,
    created_by: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InvokeWorkflowReleaseRequest {
    idempotency_key: String,
    #[serde(default)]
    input: Value,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetWorkflowReleaseCanaryRequest {
    expected_revision: u32,
    deployment_id: Uuid,
    percent: u8,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReleaseRevisionRequest {
    expected_revision: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DispatchWorkflowEventRequest {
    source: String,
    event_type: String,
    idempotency_key: String,
    #[serde(default)]
    payload: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListWorkflowInvocationsQuery {
    #[serde(default)]
    release_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListWorkflowDeliveryReceiptsQuery {
    #[serde(default)]
    deployment_id: Option<Uuid>,
    #[serde(default)]
    status: Option<WorkflowDeliveryStatusV1>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RetryWorkflowDeliveryRequest {
    expected_revision: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListWorkflowEvaluationsQuery {
    #[serde(default)]
    deployment_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateWorkflowEvaluationRequest {
    run_id: Uuid,
    evaluator: String,
    score: f64,
    passed: bool,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default)]
    note: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetWorkflowEvaluationSummaryQuery {
    deployment_id: Uuid,
}
