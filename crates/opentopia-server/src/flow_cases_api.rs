use super::{ApiError, AppState};
use crate::auth::constant_time_eq;
use crate::flow_cases_service::{
    accept_flow_case, evaluation_summary, start_pending_flow_case, supersede_pending_flow_case,
    FlowCaseResult, FlowEvaluationSummary,
};
use crate::flows_api::{ensure_enterprise, flow_error};
use crate::workflow_delivery::deliver_run_output;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{Duration, Utc};
use opentopia_core::{
    FlowCaseV1, FlowStatusV1, SessionStore, WorkflowDeliveryReceiptV1, WorkflowDeliveryStatusV1,
    WorkflowEvaluationV1, WorkflowTriggerSpecV1,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/flows/:flow_id/invoke", post(invoke_flow))
        .route("/api/flow-events", post(dispatch_flow_event))
        .route("/api/flow-cases", get(list_flow_cases))
        .route("/api/flow-cases/:case_id/start", post(start_pending_case))
        .route(
            "/api/flow-cases/:case_id/supersede",
            post(supersede_pending_case),
        )
        .route(
            "/api/flow-delivery-receipts",
            get(list_flow_delivery_receipts),
        )
        .route(
            "/api/flow-delivery-receipts/:receipt_id/retry",
            post(retry_flow_delivery),
        )
        .route(
            "/api/flow-evaluations",
            get(list_flow_evaluations).post(create_flow_evaluation),
        )
        .route(
            "/api/flow-evaluation-summary",
            get(get_flow_evaluation_summary),
        )
        .route("/hooks/flows/:trigger_id", post(invoke_flow_webhook))
}

async fn invoke_flow(
    State(state): State<AppState>,
    Path(flow_id): Path<String>,
    Json(request): Json<InvokeFlowRequest>,
) -> Result<Json<FlowCaseResult>, ApiError> {
    ensure_enterprise(&state)?;
    let flow = active_flow_for_request(&state, &flow_id)?;
    Ok(Json(
        accept_flow_case(&state, &flow, request.idempotency_key, request.input).await?,
    ))
}

async fn invoke_flow_webhook(
    State(state): State<AppState>,
    Path(trigger_id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Result<Json<FlowCaseResult>, ApiError> {
    ensure_enterprise(&state)?;
    let flow = state
        .store
        .list_active_flows(Some(FlowStatusV1::Active))
        .map_err(flow_error)?
        .into_iter()
        .find(|item| item.active_revision.trigger.trigger_id() == Some(trigger_id))
        .ok_or_else(|| ApiError::not_found("Flow trigger not found"))?;
    let WorkflowTriggerSpecV1::Webhook { token_ref, .. } = &flow.active_revision.trigger else {
        return Err(ApiError::not_found("Flow webhook trigger not found"));
    };
    let env_name = token_ref
        .strip_prefix("env:")
        .ok_or_else(|| ApiError::forbidden("Flow trigger is not configured"))?;
    let expected = std::env::var(env_name)
        .map_err(|_| ApiError::forbidden("Flow trigger is not configured"))?;
    let provided = headers
        .get("x-opentopia-trigger-token")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if expected.is_empty() || !constant_time_eq(provided.as_bytes(), expected.as_bytes()) {
        return Err(ApiError::forbidden("invalid Flow trigger token"));
    }
    let idempotency_key = required_header(&headers, "idempotency-key")?;
    if state
        .store
        .get_flow_case(&flow.flow_id, &idempotency_key)
        .map_err(flow_error)?
        .is_none()
    {
        let limit = std::env::var("OPENTOPIA_FLOW_WEBHOOK_RATE_LIMIT_PER_MINUTE")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(120)
            .clamp(1, 10_000);
        let recent = state
            .store
            .count_recent_flow_cases(trigger_id, Utc::now() - Duration::minutes(1))
            .map_err(flow_error)?;
        if recent >= limit {
            return Err(ApiError::too_many_requests(
                "Flow trigger rate limit exceeded",
            ));
        }
    }
    Ok(Json(
        accept_flow_case(&state, &flow, idempotency_key, input).await?,
    ))
}

async fn dispatch_flow_event(
    State(state): State<AppState>,
    Json(request): Json<DispatchFlowEventRequest>,
) -> Result<Json<Vec<FlowCaseResult>>, ApiError> {
    ensure_enterprise(&state)?;
    let source = request.source.trim();
    let event_type = request.event_type.trim();
    if source.is_empty() || event_type.is_empty() || request.idempotency_key.trim().is_empty() {
        return Err(ApiError::bad_request(
            "source, eventType, and idempotencyKey are required",
        ));
    }
    let flows = state
        .store
        .list_active_flows(Some(FlowStatusV1::Active))
        .map_err(flow_error)?;
    let mut results = Vec::new();
    for flow in flows {
        let WorkflowTriggerSpecV1::EventSubscription {
            source: configured_source,
            event_type: configured_type,
            ..
        } = &flow.active_revision.trigger
        else {
            continue;
        };
        if configured_source != source || configured_type != event_type {
            continue;
        }
        let key = format!("event:{source}:{event_type}:{}", request.idempotency_key);
        results.push(accept_flow_case(&state, &flow, key, request.payload.clone()).await?);
    }
    Ok(Json(results))
}

async fn list_flow_cases(
    State(state): State<AppState>,
    Query(query): Query<ListFlowCasesQuery>,
) -> Result<Json<Vec<FlowCaseV1>>, ApiError> {
    ensure_enterprise(&state)?;
    Ok(Json(
        state
            .store
            .list_flow_cases(query.flow_id.as_deref())
            .map_err(flow_error)?,
    ))
}

async fn start_pending_case(
    State(state): State<AppState>,
    Path(case_id): Path<Uuid>,
) -> Result<Json<FlowCaseResult>, ApiError> {
    ensure_enterprise(&state)?;
    Ok(Json(start_pending_flow_case(&state, case_id).await?))
}

async fn supersede_pending_case(
    State(state): State<AppState>,
    Path(case_id): Path<Uuid>,
    Json(request): Json<SupersedeFlowCaseRequest>,
) -> Result<Json<FlowCaseV1>, ApiError> {
    ensure_enterprise(&state)?;
    Ok(Json(supersede_pending_flow_case(
        &state,
        case_id,
        request.replacement_case_id,
        request.note,
    )?))
}

async fn list_flow_delivery_receipts(
    State(state): State<AppState>,
    Query(query): Query<ListFlowDeliveryReceiptsQuery>,
) -> Result<Json<Vec<WorkflowDeliveryReceiptV1>>, ApiError> {
    ensure_enterprise(&state)?;
    Ok(Json(
        state
            .store
            .list_workflow_delivery_receipts(query.flow_revision_id, query.status)
            .map_err(flow_error)?,
    ))
}

async fn retry_flow_delivery(
    State(state): State<AppState>,
    Path(receipt_id): Path<Uuid>,
    Json(request): Json<RetryFlowDeliveryRequest>,
) -> Result<Json<WorkflowDeliveryReceiptV1>, ApiError> {
    ensure_enterprise(&state)?;
    let receipt = state
        .store
        .get_workflow_delivery_receipt(receipt_id)
        .map_err(flow_error)?
        .ok_or_else(|| ApiError::not_found("Flow delivery receipt not found"))?;
    if receipt.revision != request.expected_revision {
        return Err(ApiError::conflict(format!(
            "Flow delivery receipt revision conflict; current revision is {}",
            receipt.revision
        )));
    }
    if !matches!(
        receipt.status,
        WorkflowDeliveryStatusV1::Failed | WorkflowDeliveryStatusV1::Pending
    ) {
        return Err(ApiError::conflict("Flow delivery receipt is not retryable"));
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

async fn list_flow_evaluations(
    State(state): State<AppState>,
    Query(query): Query<ListFlowEvaluationsQuery>,
) -> Result<Json<Vec<WorkflowEvaluationV1>>, ApiError> {
    ensure_enterprise(&state)?;
    Ok(Json(
        state
            .store
            .list_workflow_evaluations(query.flow_revision_id)
            .map_err(flow_error)?,
    ))
}

async fn create_flow_evaluation(
    State(state): State<AppState>,
    Json(request): Json<CreateFlowEvaluationRequest>,
) -> Result<Json<WorkflowEvaluationV1>, ApiError> {
    ensure_enterprise(&state)?;
    let run = state
        .store
        .get_flow_run(request.run_id)
        .map_err(flow_error)?
        .ok_or_else(|| ApiError::not_found("Flow run not found"))?;
    let flow_revision_id = run
        .flow_revision_id
        .ok_or_else(|| ApiError::bad_request("Flow run has no active revision"))?;
    let evaluation = WorkflowEvaluationV1::new(
        run.id,
        flow_revision_id,
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
        .map_err(flow_error)?
    {
        if existing == evaluation {
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
            .map_err(flow_error)?,
    ))
}

async fn get_flow_evaluation_summary(
    State(state): State<AppState>,
    Query(query): Query<GetFlowEvaluationSummaryQuery>,
) -> Result<Json<FlowEvaluationSummary>, ApiError> {
    ensure_enterprise(&state)?;
    Ok(Json(evaluation_summary(&state, query.flow_revision_id)?))
}

fn active_flow_for_request(
    state: &AppState,
    flow_id: &str,
) -> Result<opentopia_core::ActiveFlowV1, ApiError> {
    state
        .store
        .get_active_flow(flow_id)
        .map_err(flow_error)?
        .ok_or_else(|| ApiError::not_found("Flow not found"))
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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InvokeFlowRequest {
    idempotency_key: String,
    #[serde(default)]
    input: Value,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DispatchFlowEventRequest {
    source: String,
    event_type: String,
    idempotency_key: String,
    #[serde(default)]
    payload: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListFlowCasesQuery {
    #[serde(default)]
    flow_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SupersedeFlowCaseRequest {
    #[serde(default)]
    replacement_case_id: Option<Uuid>,
    note: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListFlowDeliveryReceiptsQuery {
    #[serde(default)]
    flow_revision_id: Option<Uuid>,
    #[serde(default)]
    status: Option<WorkflowDeliveryStatusV1>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RetryFlowDeliveryRequest {
    expected_revision: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListFlowEvaluationsQuery {
    #[serde(default)]
    flow_revision_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateFlowEvaluationRequest {
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
struct GetFlowEvaluationSummaryQuery {
    flow_revision_id: Uuid,
}
