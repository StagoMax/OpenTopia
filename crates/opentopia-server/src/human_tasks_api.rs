use super::{flows_api, ApiError, AppState};
use crate::workflow_delivery::deliver_run_output;
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use opentopia_core::{
    prepare_flow_interrupt_resume, prepare_flow_resume, resolve_flow_approval, spawn_flow_run,
    FlowRunStatusV1, HumanTaskActionV1, HumanTaskSourceKindV1, HumanTaskStatusV1,
    HumanTaskStoreError, HumanTaskTypeV1, HumanTaskV1, SessionStore, WorkflowDeliveryReceiptV1,
    WorkflowDeliveryStatusV1,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/human-tasks", get(list_human_tasks))
        .route("/api/human-tasks/:task_id", get(get_human_task))
        .route(
            "/api/human-tasks/:task_id/resolve",
            post(resolve_human_task),
        )
        .route("/api/human-tasks/:task_id/claim", post(claim_human_task))
        .route("/api/human-tasks/:task_id/assign", post(assign_human_task))
}

async fn list_human_tasks(
    State(state): State<AppState>,
    Query(query): Query<ListHumanTasksQuery>,
) -> Result<Json<Vec<HumanTaskV1>>, ApiError> {
    flows_api::ensure_enterprise(&state)?;
    if let Some(thread_id) = query.thread_id {
        flows_api::ensure_flow_thread(&state, thread_id)?;
    }
    let status = query.status.or(Some(HumanTaskStatusV1::Pending));
    let tasks = state
        .store
        .list_human_tasks(query.thread_id, status)
        .map_err(human_task_error)?
        .into_iter()
        .filter(|task| query.kind.map_or(true, |kind| task.task_type == kind))
        .filter(|task| {
            query.flow_run_id.map_or(true, |flow_run_id| {
                task.source_kind == HumanTaskSourceKindV1::FlowRun && task.source_id == flow_run_id
            })
        })
        .collect();
    Ok(Json(tasks))
}

async fn get_human_task(
    State(state): State<AppState>,
    Path(task_id): Path<Uuid>,
) -> Result<Json<HumanTaskV1>, ApiError> {
    flows_api::ensure_enterprise(&state)?;
    let task = human_task_for_request(&state, task_id)?;
    Ok(Json(task))
}

async fn resolve_human_task(
    State(state): State<AppState>,
    Path(task_id): Path<Uuid>,
    Json(request): Json<ResolveHumanTaskRequest>,
) -> Result<Json<ResolveHumanTaskResponse>, ApiError> {
    flows_api::ensure_enterprise(&state)?;
    let mut task = human_task_for_request(&state, task_id)?;
    let idempotency_key = request
        .idempotency_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            format!(
                "legacy:{}:{}:{:?}",
                task.id, request.expected_revision, request.action
            )
        });
    if task.status != HumanTaskStatusV1::Pending {
        if resolution_matches(&task, request.action, &idempotency_key) {
            let (run, delivery_receipt) = match task.source_kind {
                HumanTaskSourceKindV1::FlowRun => (
                    Some(
                        state
                            .store
                            .get_flow_run(task.source_id)
                            .map_err(human_task_error)?
                            .ok_or_else(|| ApiError::not_found("Flow run not found"))?,
                    ),
                    None,
                ),
                HumanTaskSourceKindV1::DeliveryReceipt => (
                    None,
                    state
                        .store
                        .get_workflow_delivery_receipt(task.source_id)
                        .map_err(human_task_error)?,
                ),
            };
            return Ok(Json(ResolveHumanTaskResponse {
                task,
                run,
                delivery_receipt,
            }));
        }
        return Err(ApiError::conflict("Human task is no longer pending"));
    }
    if task.revision != request.expected_revision {
        return Err(ApiError::conflict(format!(
            "Human task revision conflict; current revision is {}",
            task.revision
        )));
    }
    if task.source_kind == HumanTaskSourceKindV1::DeliveryReceipt {
        return resolve_delivery_human_task(state, task, request, idempotency_key).await;
    }
    let mut run = state
        .store
        .get_flow_run(task.source_id)
        .map_err(human_task_error)?
        .ok_or_else(|| ApiError::not_found("Flow run not found"))?;
    let thread = flows_api::ensure_flow_thread(&state, run.thread_id)?;
    if run.active_human_task_id != Some(task.id) {
        return Err(ApiError::conflict(
            "Flow run is no longer waiting on this Human task",
        ));
    }
    let expected_run_revision = run.revision;
    let actor = "local_operator";
    if task
        .claimed_by
        .as_deref()
        .is_some_and(|claimed_by| claimed_by != actor)
    {
        return Err(ApiError::conflict(format!(
            "Human task is claimed by {}",
            task.claimed_by.as_deref().unwrap_or_default()
        )));
    }
    let mut resume_command_id = None;
    match task.task_type {
        HumanTaskTypeV1::Approval if task.continuation_id.is_none() => match request.action {
            HumanTaskActionV1::Approve | HumanTaskActionV1::Reject => {
                resolve_flow_approval(
                    &mut run,
                    request.action == HumanTaskActionV1::Approve,
                    request.note.as_deref(),
                )
                .map_err(ApiError::from)?;
            }
            _ => {
                return Err(ApiError::bad_request(
                    "approval tasks only accept approve or reject",
                ))
            }
        },
        HumanTaskTypeV1::Approval
        | HumanTaskTypeV1::InputRequest
        | HumanTaskTypeV1::Reconnect
        | HumanTaskTypeV1::Reconciliation => {
            if request.action == HumanTaskActionV1::Cancel {
                run.status = FlowRunStatusV1::Cancelled;
                run.error = request
                    .note
                    .clone()
                    .or_else(|| Some("Human task cancelled by operator".to_string()));
                run.active_human_task_id = None;
                run.completed_at = Some(Utc::now());
                run.touch();
            } else {
                let command = prepare_flow_interrupt_resume(
                    &mut run,
                    &task,
                    request.action,
                    request.response.clone(),
                    request.note.as_deref(),
                    actor,
                    &idempotency_key,
                )
                .map_err(ApiError::from)?;
                resume_command_id = Some(command.id);
            }
        }
        HumanTaskTypeV1::Recovery if task.continuation_id.is_some() => {
            if request.action == HumanTaskActionV1::Cancel {
                run.status = FlowRunStatusV1::Cancelled;
                run.error = request
                    .note
                    .clone()
                    .or_else(|| Some("Agent continuation retry cancelled".to_string()));
                run.active_human_task_id = None;
                run.completed_at = Some(Utc::now());
                run.touch();
            } else {
                let command = prepare_flow_interrupt_resume(
                    &mut run,
                    &task,
                    request.action,
                    request.response.clone(),
                    request.note.as_deref(),
                    actor,
                    &idempotency_key,
                )
                .map_err(ApiError::from)?;
                resume_command_id = Some(command.id);
            }
        }
        HumanTaskTypeV1::Recovery => match request.action {
            HumanTaskActionV1::Retry => {
                if run.pending_resume_command_id().is_some() {
                    run.status = FlowRunStatusV1::Resuming;
                    for node in &mut run.node_runs {
                        if node.status == opentopia_core::FlowNodeRunStatusV1::Resuming {
                            node.error = None;
                        }
                    }
                } else {
                    prepare_flow_resume(&mut run, true).map_err(ApiError::from)?;
                    run.status = FlowRunStatusV1::Running;
                }
                run.error = None;
                run.active_human_task_id = None;
                run.touch();
            }
            HumanTaskActionV1::Cancel => {
                run.status = FlowRunStatusV1::Cancelled;
                run.error = request
                    .note
                    .clone()
                    .or_else(|| Some("recovery cancelled by operator".to_string()));
                run.active_human_task_id = None;
                run.completed_at = Some(Utc::now());
                run.touch();
            }
            _ => {
                return Err(ApiError::bad_request(
                    "recovery tasks only accept retry or cancel",
                ))
            }
        },
        HumanTaskTypeV1::OutputReview => match request.action {
            HumanTaskActionV1::Approve => {
                run.output_reviewed = true;
                run.status = FlowRunStatusV1::Succeeded;
                run.active_human_task_id = None;
                run.error = None;
                run.completed_at = Some(Utc::now());
                run.touch();
            }
            HumanTaskActionV1::Reject => {
                run.status = FlowRunStatusV1::Cancelled;
                run.active_human_task_id = None;
                run.error = request
                    .note
                    .clone()
                    .or_else(|| Some("Flow output rejected by operator".to_string()));
                run.completed_at = Some(Utc::now());
                run.touch();
            }
            _ => {
                return Err(ApiError::bad_request(
                    "output review only accepts approve or reject",
                ))
            }
        },
        HumanTaskTypeV1::DataCorrection | HumanTaskTypeV1::Manual => {
            return Err(ApiError::bad_request(
                "Human task kind is not supported yet",
            ))
        }
    }
    let audit_command_id =
        resume_command_id.or_else(|| Some(Uuid::new_v5(&task.id, idempotency_key.as_bytes())));
    task.resolve_with_command(
        request.action,
        request.note.as_deref(),
        actor,
        audit_command_id,
        Some(&idempotency_key),
        request.response.clone(),
    )
    .map_err(ApiError::from)?;
    // Keep the Human task pending when its Flow runtime cannot be rebuilt.
    // Operators can restore the Connection and retry the same idempotent
    // action instead of losing the approval into a stuck Resuming run.
    let context = if matches!(
        run.status,
        FlowRunStatusV1::Running | FlowRunStatusV1::Resuming
    ) {
        Some(flows_api::flow_runtime_context(&state, &thread, &run).await?)
    } else {
        None
    };
    let expected_task_revision = request.expected_revision;
    let updated = state.store.update_flow_run_and_human_task(
        &run,
        expected_run_revision,
        &task,
        Some(expected_task_revision),
    );
    let (run, task) = match updated {
        Ok(updated) => updated,
        Err(error)
            if matches!(
                error.downcast_ref::<HumanTaskStoreError>(),
                Some(HumanTaskStoreError::RevisionConflict(_))
            ) =>
        {
            let current = human_task_for_request(&state, task_id)?;
            if !resolution_matches(&current, request.action, &idempotency_key) {
                return Err(human_task_error(error));
            }
            let current_run = state
                .store
                .get_flow_run(current.source_id)
                .map_err(human_task_error)?
                .ok_or_else(|| ApiError::not_found("Flow run not found"))?;
            return Ok(Json(ResolveHumanTaskResponse {
                task: current,
                run: Some(current_run),
                delivery_receipt: None,
            }));
        }
        Err(error) => return Err(human_task_error(error)),
    };
    if let Some(context) = context {
        spawn_flow_run(run.id, context).map_err(ApiError::from)?;
    }
    let delivery_receipt = if run.status == FlowRunStatusV1::Succeeded {
        Some(
            deliver_run_output(&state, &run, false)
                .await
                .map_err(|error| ApiError::bad_gateway(error.to_string()))?,
        )
    } else {
        None
    };
    Ok(Json(ResolveHumanTaskResponse {
        task,
        run: Some(run),
        delivery_receipt,
    }))
}

async fn resolve_delivery_human_task(
    state: AppState,
    mut task: HumanTaskV1,
    request: ResolveHumanTaskRequest,
    idempotency_key: String,
) -> Result<Json<ResolveHumanTaskResponse>, ApiError> {
    let actor = "local_operator";
    if task
        .claimed_by
        .as_deref()
        .is_some_and(|claimed_by| claimed_by != actor)
    {
        return Err(ApiError::conflict(format!(
            "Human task is claimed by {}",
            task.claimed_by.as_deref().unwrap_or_default()
        )));
    }
    let mut receipt = state
        .store
        .get_workflow_delivery_receipt(task.source_id)
        .map_err(human_task_error)?
        .ok_or_else(|| ApiError::not_found("DeliveryReceipt not found"))?;
    match (task.task_type, request.action) {
        (HumanTaskTypeV1::Recovery, HumanTaskActionV1::Retry) => {
            let run = state
                .store
                .get_flow_run(receipt.run_id)
                .map_err(human_task_error)?
                .ok_or_else(|| ApiError::not_found("Flow run not found"))?;
            receipt = deliver_run_output(&state, &run, true)
                .await
                .map_err(|error| ApiError::bad_gateway(error.to_string()))?;
            if receipt.status == WorkflowDeliveryStatusV1::Failed {
                return Err(ApiError::bad_gateway(
                    receipt
                        .error
                        .clone()
                        .unwrap_or_else(|| "Workflow output delivery failed".to_string()),
                ));
            }
        }
        (HumanTaskTypeV1::Recovery, HumanTaskActionV1::Cancel)
        | (HumanTaskTypeV1::Manual, HumanTaskActionV1::Cancel) => {
            let expected = receipt.revision;
            receipt.mark_cancelled();
            receipt = state
                .store
                .update_workflow_delivery_receipt(&receipt, expected)
                .map_err(human_task_error)?;
        }
        (HumanTaskTypeV1::Manual, HumanTaskActionV1::Acknowledge) => {
            let expected = receipt.revision;
            receipt.mark_delivered(
                None,
                Some(serde_json::json!({
                    "acknowledgedBy": actor,
                    "humanTaskId": task.id,
                })),
            );
            receipt = state
                .store
                .update_workflow_delivery_receipt(&receipt, expected)
                .map_err(human_task_error)?;
        }
        (HumanTaskTypeV1::Recovery, _) => {
            return Err(ApiError::bad_request(
                "delivery recovery tasks only accept retry or cancel",
            ))
        }
        (HumanTaskTypeV1::Manual, _) => {
            return Err(ApiError::bad_request(
                "delivery handoff tasks only accept acknowledge or cancel",
            ))
        }
        _ => {
            return Err(ApiError::bad_request(
                "unsupported DeliveryReceipt task kind",
            ))
        }
    }

    task.resolve_with_command(
        request.action,
        request.note.as_deref(),
        actor,
        Some(Uuid::new_v5(&task.id, idempotency_key.as_bytes())),
        Some(&idempotency_key),
        request.response,
    )
    .map_err(ApiError::from)?;
    task = state
        .store
        .update_human_task(&task, request.expected_revision)
        .map_err(human_task_error)?;
    Ok(Json(ResolveHumanTaskResponse {
        task,
        run: None,
        delivery_receipt: Some(receipt),
    }))
}

async fn claim_human_task(
    State(state): State<AppState>,
    Path(task_id): Path<Uuid>,
    Json(request): Json<ClaimHumanTaskRequest>,
) -> Result<Json<HumanTaskV1>, ApiError> {
    flows_api::ensure_enterprise(&state)?;
    let mut task = human_task_for_request(&state, task_id)?;
    if task.revision != request.expected_revision {
        return Err(ApiError::conflict(format!(
            "Human task revision conflict; current revision is {}",
            task.revision
        )));
    }
    task.claim("local_operator").map_err(ApiError::from)?;
    Ok(Json(
        state
            .store
            .update_human_task(&task, request.expected_revision)
            .map_err(human_task_error)?,
    ))
}

async fn assign_human_task(
    State(state): State<AppState>,
    Path(task_id): Path<Uuid>,
    Json(request): Json<AssignHumanTaskRequest>,
) -> Result<Json<HumanTaskV1>, ApiError> {
    flows_api::ensure_enterprise(&state)?;
    let mut task = human_task_for_request(&state, task_id)?;
    if task.revision != request.expected_revision {
        return Err(ApiError::conflict(format!(
            "Human task revision conflict; current revision is {}",
            task.revision
        )));
    }
    task.assign(request.assignee.as_deref())
        .map_err(ApiError::from)?;
    Ok(Json(
        state
            .store
            .update_human_task(&task, request.expected_revision)
            .map_err(human_task_error)?,
    ))
}

fn resolution_matches(task: &HumanTaskV1, action: HumanTaskActionV1, key: &str) -> bool {
    task.resolution.as_ref().is_some_and(|resolution| {
        resolution.action == action && resolution.idempotency_key.as_deref() == Some(key)
    })
}

fn human_task_for_request(state: &AppState, task_id: Uuid) -> Result<HumanTaskV1, ApiError> {
    let task = state
        .store
        .get_human_task(task_id)
        .map_err(human_task_error)?
        .ok_or_else(|| ApiError::not_found("Human task not found"))?;
    flows_api::ensure_flow_thread(state, task.thread_id)?;
    Ok(task)
}

fn human_task_error(error: anyhow::Error) -> ApiError {
    if let Some(error) = error.downcast_ref::<HumanTaskStoreError>() {
        return match error {
            HumanTaskStoreError::NotFound(_) => ApiError::not_found(error.to_string()),
            HumanTaskStoreError::RevisionConflict(_) => ApiError::conflict(error.to_string()),
        };
    }
    flows_api::flow_error(error)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListHumanTasksQuery {
    #[serde(default)]
    status: Option<HumanTaskStatusV1>,
    #[serde(default)]
    kind: Option<HumanTaskTypeV1>,
    #[serde(default)]
    thread_id: Option<Uuid>,
    #[serde(default)]
    flow_run_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResolveHumanTaskRequest {
    expected_revision: u32,
    action: HumanTaskActionV1,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    idempotency_key: Option<String>,
    #[serde(default)]
    response: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaimHumanTaskRequest {
    expected_revision: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssignHumanTaskRequest {
    expected_revision: u32,
    #[serde(default)]
    assignee: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct ResolveHumanTaskResponse {
    task: HumanTaskV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    run: Option<opentopia_core::FlowRunV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    delivery_receipt: Option<WorkflowDeliveryReceiptV1>,
}
