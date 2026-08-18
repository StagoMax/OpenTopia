use super::{flows_api, ApiError, AppState};
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use opentopia_core::{
    prepare_flow_resume, resolve_flow_approval, spawn_flow_run, FlowRunStatusV1, HumanTaskActionV1,
    HumanTaskStatusV1, HumanTaskStoreError, HumanTaskTypeV1, HumanTaskV1, SessionStore,
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
            query
                .flow_run_id
                .map_or(true, |flow_run_id| task.source_id == flow_run_id)
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
    if task.revision != request.expected_revision {
        return Err(ApiError::conflict(format!(
            "Human task revision conflict; current revision is {}",
            task.revision
        )));
    }
    if task.status != HumanTaskStatusV1::Pending {
        return Err(ApiError::conflict("Human task is no longer pending"));
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
    match task.task_type {
        HumanTaskTypeV1::Approval => match request.action {
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
        HumanTaskTypeV1::Recovery => match request.action {
            HumanTaskActionV1::Retry => {
                prepare_flow_resume(&mut run, true).map_err(ApiError::from)?;
                run.status = FlowRunStatusV1::Running;
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
        _ => {
            return Err(ApiError::bad_request(
                "Human task kind is not supported yet",
            ))
        }
    }
    task.resolve(request.action, request.note.as_deref(), "local_operator")
        .map_err(ApiError::from)?;
    let expected_task_revision = request.expected_revision;
    let (run, task) = state
        .store
        .update_flow_run_and_human_task(
            &run,
            expected_run_revision,
            &task,
            Some(expected_task_revision),
        )
        .map_err(human_task_error)?;
    if run.status == FlowRunStatusV1::Running {
        let capabilities = run.effective_capabilities.clone();
        let context =
            flows_api::flow_runtime_context(&state, &thread, run.id, capabilities).await?;
        spawn_flow_run(run.id, context).map_err(ApiError::from)?;
    }
    Ok(Json(ResolveHumanTaskResponse { task, run }))
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
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct ResolveHumanTaskResponse {
    task: HumanTaskV1,
    run: opentopia_core::FlowRunV1,
}
