use super::{
    decode_turn_checkpoint, ensure_thread, finalize_goal_after_turn, finish_turn, publish_payload,
    run_resumed_agent_turn, ApiError, AppState,
};
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use opentopia_core::collaboration::{
    AgentMailboxNotifier, AgentRunCommand, AgentRunResumeSignal, AgentRunScheduler, AgentTurnId,
    AgentTurnStatus, CollaborationRegistry,
};
use opentopia_core::{
    AgentEventPayload, AgentResumeSignal, Approval, ApprovalStatus, SessionStore, TurnStatus,
    UserInputRecord, UserInputRequest, UserInputResponse, UserInputStatus,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use tracing::warn;
use uuid::Uuid;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/threads/:thread_id/approvals", get(list_approvals))
        .route(
            "/api/threads/:thread_id/approvals/:approval_id/decision",
            post(decide_approval),
        )
        .route(
            "/api/threads/:thread_id/user-input",
            get(list_user_input_requests),
        )
        .route(
            "/api/threads/:thread_id/user-input/:request_id/response",
            post(respond_to_user_input),
        )
        .route(
            "/api/threads/:thread_id/turns/:turn_id/external-action/resume",
            post(resume_external_action),
        )
}

async fn decide_approval(
    State(state): State<AppState>,
    Path((thread_id, approval_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<ApprovalDecisionRequest>,
) -> Result<Json<ApprovalDecisionResponse>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let pending = state
        .store
        .get_approval(approval_id)?
        .ok_or_else(|| ApiError::not_found(format!("approval not found: {approval_id}")))?;
    if pending.thread_id != thread_id {
        return Err(ApiError::bad_request(
            "approval does not belong to this thread",
        ));
    }
    if pending.status != ApprovalStatus::Pending {
        return Err(ApiError::conflict(format!(
            "approval already decided: {approval_id}"
        )));
    }

    let continuation_value = state
        .store
        .get_approval_continuation(approval_id, thread_id)?;
    let continuation_value = continuation_value
        .ok_or_else(|| ApiError::conflict("approval continuation is not available"))?;
    let continuation = decode_turn_checkpoint(&state.store, "approval", continuation_value)
        .map_err(|err| ApiError::internal(format!("invalid approval continuation: {err}")))?;
    let continuation_turn_id = if continuation.turn_id.is_nil() {
        state
            .store
            .get_latest_turn(thread_id)?
            .filter(|turn| turn.user_message_id == continuation.user_message_id)
            .map(|turn| turn.turn_id)
            .ok_or_else(|| ApiError::conflict("approval continuation turn is not available"))?
    } else {
        continuation.turn_id
    };
    if let Some(collaboration_turn) = state
        .collaboration_repository
        .find_turn(AgentTurnId::from_uuid(continuation_turn_id))?
    {
        let collaboration_thread = state
            .collaboration_repository
            .get_thread(collaboration_turn.agent_thread_id)
            .await?;
        if collaboration_thread.path.as_str() != "/root" {
            let status = if request.approved {
                ApprovalStatus::Approved
            } else {
                ApprovalStatus::Denied
            };
            state
                .store
                .update_approval_status(approval_id, status)?
                .ok_or_else(|| ApiError::not_found(format!("approval not found: {approval_id}")))?;
            state
                .store
                .delete_approval_continuation(approval_id, thread_id)?;
            state
                .agent_run_scheduler
                .submit(AgentRunCommand::Resume {
                    session_id: collaboration_turn.session_id,
                    agent_thread_id: collaboration_turn.agent_thread_id,
                    agent_turn_id: collaboration_turn.id,
                    invocation_id: collaboration_turn.invocation_id.saturating_add(1),
                    signal: AgentRunResumeSignal::Approval {
                        approval_id: Some(approval_id),
                        approved: request.approved,
                    },
                })
                .await
                .map_err(|error| ApiError::internal(error.to_string()))?;
            return Ok(Json(ApprovalDecisionResponse {
                accepted: true,
                executed: request.approved,
            }));
        }
    }
    let turn = state
        .turns
        .resume(
            thread_id,
            continuation_turn_id,
            continuation.user_message_id,
        )
        .map_err(ApiError::from)?
        .map_err(|active| {
            ApiError::conflict(format!("thread already has active turn {}", active.turn_id))
        })?;
    let status = if request.approved {
        ApprovalStatus::Approved
    } else {
        ApprovalStatus::Denied
    };
    match state.store.update_approval_status(approval_id, status) {
        Ok(Some(_)) => {}
        Ok(None) => {
            let error = format!("approval not found: {approval_id}");
            finish_turn(
                &state,
                thread_id,
                turn.turn_id,
                TurnStatus::Failed,
                Some(error.clone()),
            );
            return Err(ApiError::not_found(error));
        }
        Err(error) => {
            finish_turn(
                &state,
                thread_id,
                turn.turn_id,
                TurnStatus::Failed,
                Some(error.to_string()),
            );
            return Err(error.into());
        }
    }
    if let Err(error) = state
        .store
        .delete_turn_checkpoint(continuation_turn_id, thread_id)
    {
        warn!(?error, %continuation_turn_id, "failed to delete resumed approval checkpoint");
    }
    let run_state = state.clone();
    tokio::spawn(async move {
        run_resumed_agent_turn(
            run_state,
            AgentResumeSignal::Approval {
                approval_id: Some(approval_id),
                approved: request.approved,
            },
            continuation,
            turn,
        )
        .await;
    });

    Ok(Json(ApprovalDecisionResponse {
        accepted: true,
        executed: request.approved,
    }))
}

async fn list_user_input_requests(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Query(query): Query<UserInputQuery>,
) -> Result<Json<Vec<UserInputRecord>>, ApiError> {
    ensure_thread(&state, thread_id)?;
    Ok(Json(
        state
            .store
            .list_user_input_requests(thread_id, query.status)?,
    ))
}

async fn respond_to_user_input(
    State(state): State<AppState>,
    Path((thread_id, request_id)): Path<(Uuid, Uuid)>,
    Json(response): Json<UserInputResponse>,
) -> Result<Json<UserInputResponseAccepted>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let pending = state
        .store
        .get_user_input_request(request_id)?
        .ok_or_else(|| {
            ApiError::not_found(format!("user input request not found: {request_id}"))
        })?;
    if pending.thread_id != thread_id {
        return Err(ApiError::bad_request(
            "user input request does not belong to this thread",
        ));
    }
    if pending.status != UserInputStatus::Pending {
        return Err(ApiError::conflict(format!(
            "user input request already answered: {request_id}"
        )));
    }
    let response = validate_user_input_response(&pending.request, response)?;
    let continuation_value = state
        .store
        .get_user_input_continuation(request_id, thread_id)?
        .ok_or_else(|| ApiError::conflict("user input continuation is not available"))?;
    let continuation = decode_turn_checkpoint(&state.store, "user_input", continuation_value)
        .map_err(|error| ApiError::internal(format!("invalid user input continuation: {error}")))?;
    let continuation_turn_id = if continuation.turn_id.is_nil() {
        state
            .store
            .get_latest_turn(thread_id)?
            .filter(|turn| turn.user_message_id == continuation.user_message_id)
            .map(|turn| turn.turn_id)
            .ok_or_else(|| ApiError::conflict("user-input continuation turn is not available"))?
    } else {
        continuation.turn_id
    };
    if let Some(collaboration_turn) = state
        .collaboration_repository
        .find_turn(AgentTurnId::from_uuid(continuation_turn_id))?
    {
        let collaboration_thread = state
            .collaboration_repository
            .get_thread(collaboration_turn.agent_thread_id)
            .await?;
        if collaboration_thread.path.as_str() != "/root" {
            state
                .store
                .resolve_user_input_request(request_id, thread_id, &response)?
                .ok_or_else(|| {
                    ApiError::conflict(format!(
                        "user input request is no longer pending: {request_id}"
                    ))
                })?;
            if response.cancelled {
                if let Some(message) = state.collaboration_repository.record_turn_state(
                    collaboration_turn.id,
                    AgentTurnStatus::Cancelled,
                    &json!({
                        "status": "cancelled",
                        "reason": "User dismissed the decision request.",
                    }),
                )? {
                    state.agent_run_scheduler.message_enqueued(&message);
                }
                state.agent_activity.notify(collaboration_thread.id);
                return Ok(Json(UserInputResponseAccepted {
                    accepted: true,
                    resumed: false,
                }));
            }
            state
                .agent_run_scheduler
                .submit(AgentRunCommand::Resume {
                    session_id: collaboration_turn.session_id,
                    agent_thread_id: collaboration_turn.agent_thread_id,
                    agent_turn_id: collaboration_turn.id,
                    invocation_id: collaboration_turn.invocation_id.saturating_add(1),
                    signal: AgentRunResumeSignal::UserInput {
                        request_id,
                        response,
                    },
                })
                .await
                .map_err(|error| ApiError::internal(error.to_string()))?;
            return Ok(Json(UserInputResponseAccepted {
                accepted: true,
                resumed: true,
            }));
        }
    }
    if response.cancelled {
        if state
            .store
            .resolve_user_input_request(request_id, thread_id, &response)?
            .is_none()
        {
            return Err(ApiError::conflict(format!(
                "user input request is no longer pending: {request_id}"
            )));
        }
        state
            .turns
            .finish(thread_id, continuation_turn_id, TurnStatus::Cancelled, None)?
            .ok_or_else(|| ApiError::conflict("root Turn projection is no longer available"))?;
        if let Err(error) = state
            .store
            .delete_turn_checkpoint(continuation_turn_id, thread_id)
        {
            warn!(?error, %continuation_turn_id, "failed to delete cancelled user-input checkpoint");
        }
        publish_payload(
            &state,
            thread_id,
            Some(continuation_turn_id),
            AgentEventPayload::TurnCancelled {
                reason: "User dismissed the decision request.".to_string(),
            },
        );
        finalize_goal_after_turn(
            &state,
            thread_id,
            continuation.collaboration_mode,
            continuation.goal.as_ref().map(|goal| goal.id),
            TurnStatus::Cancelled,
        );
        let _ = state.turn_inbox.drain(continuation_turn_id);
        let _ = state.turn_queue.send(thread_id);
        return Ok(Json(UserInputResponseAccepted {
            accepted: true,
            resumed: false,
        }));
    }
    let turn = state
        .turns
        .resume(
            thread_id,
            continuation_turn_id,
            continuation.user_message_id,
        )
        .map_err(ApiError::from)?
        .map_err(|active| {
            ApiError::conflict(format!("thread already has active turn {}", active.turn_id))
        })?;
    if state
        .store
        .resolve_user_input_request(request_id, thread_id, &response)?
        .is_none()
    {
        let message = format!("user input request is no longer pending: {request_id}");
        finish_turn(
            &state,
            thread_id,
            turn.turn_id,
            TurnStatus::Failed,
            Some(message.clone()),
        );
        return Err(ApiError::conflict(message));
    }
    if let Err(error) = state
        .store
        .delete_turn_checkpoint(continuation_turn_id, thread_id)
    {
        warn!(?error, %continuation_turn_id, "failed to delete resumed user-input checkpoint");
    }

    let run_state = state.clone();
    tokio::spawn(async move {
        run_resumed_agent_turn(
            run_state,
            AgentResumeSignal::UserInput {
                request_id,
                response,
            },
            continuation,
            turn,
        )
        .await;
    });

    Ok(Json(UserInputResponseAccepted {
        accepted: true,
        resumed: true,
    }))
}

async fn resume_external_action(
    State(state): State<AppState>,
    Path((thread_id, turn_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<ExternalActionResumeRequest>,
) -> Result<Json<ExternalActionResumeResponse>, ApiError> {
    ensure_thread(&state, thread_id)?;
    if let Some(collaboration_turn) = state
        .collaboration_repository
        .find_turn(AgentTurnId::from_uuid(turn_id))?
    {
        let collaboration_thread = state
            .collaboration_repository
            .get_thread(collaboration_turn.agent_thread_id)
            .await?;
        if collaboration_thread.path.as_str() != "/root" {
            if collaboration_turn.status != AgentTurnStatus::WaitingAction {
                return Err(ApiError::conflict(format!(
                    "Agent Turn {turn_id} is not waiting for an external action"
                )));
            }
            let observation = request
                .observation
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| {
                    "The user reports that the requested external action is complete.".to_string()
                });
            let invocation_id = collaboration_turn.invocation_id.saturating_add(1);
            state
                .agent_run_scheduler
                .submit(AgentRunCommand::Resume {
                    session_id: collaboration_turn.session_id,
                    agent_thread_id: collaboration_turn.agent_thread_id,
                    agent_turn_id: collaboration_turn.id,
                    invocation_id,
                    signal: AgentRunResumeSignal::ExternalAction { observation },
                })
                .await
                .map_err(|error| ApiError::internal(error.to_string()))?;
            return Ok(Json(ExternalActionResumeResponse {
                accepted: true,
                resumed: true,
                turn_id,
                invocation_id,
            }));
        }
    }
    let turn_record = state
        .turns
        .status(thread_id)?
        .filter(|turn| turn.turn_id == turn_id)
        .ok_or_else(|| ApiError::not_found(format!("turn not found: {turn_id}")))?;
    if turn_record.status != TurnStatus::WaitingUserAction {
        return Err(ApiError::conflict(format!(
            "turn {turn_id} is not waiting for an external action"
        )));
    }
    let (wait_kind, checkpoint) = state
        .store
        .get_turn_checkpoint(turn_id, thread_id)?
        .ok_or_else(|| ApiError::conflict("external-action checkpoint is not available"))?;
    if wait_kind != "external_action" {
        return Err(ApiError::conflict(format!(
            "turn checkpoint is for {wait_kind}, not an external action"
        )));
    }
    let continuation = decode_turn_checkpoint(&state.store, "external_action", checkpoint)
        .map_err(|error| {
            ApiError::internal(format!("invalid external-action checkpoint: {error}"))
        })?;
    if continuation.turn_id != turn_id || continuation.thread_id != thread_id {
        return Err(ApiError::conflict(
            "external-action checkpoint does not belong to this turn",
        ));
    }
    let turn = state
        .turns
        .resume(thread_id, turn_id, continuation.user_message_id)
        .map_err(ApiError::from)?
        .map_err(|active| {
            ApiError::conflict(format!("thread already has active turn {}", active.turn_id))
        })?;
    if let Err(error) = state.store.delete_turn_checkpoint(turn_id, thread_id) {
        warn!(?error, %turn_id, "failed to delete resumed external-action checkpoint");
    }
    publish_payload(
        &state,
        thread_id,
        Some(turn_id),
        AgentEventPayload::BrowserHandoffCompleted {
            prior_turn_id: turn_id,
        },
    );
    let observation = request
        .observation
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            "The user reports that the requested external action is complete.".to_string()
        });
    let invocation_id = turn.invocation_id;
    let run_state = state.clone();
    tokio::spawn(async move {
        run_resumed_agent_turn(
            run_state,
            AgentResumeSignal::ExternalAction { observation },
            continuation,
            turn,
        )
        .await;
    });
    Ok(Json(ExternalActionResumeResponse {
        accepted: true,
        resumed: true,
        turn_id,
        invocation_id,
    }))
}

pub(super) fn validate_user_input_response(
    request: &UserInputRequest,
    response: UserInputResponse,
) -> Result<UserInputResponse, ApiError> {
    if response.cancelled {
        if response.skipped || !response.answers.is_empty() {
            return Err(ApiError::bad_request(
                "a cancelled user decision cannot also be skipped or contain answers",
            ));
        }
        return Ok(response);
    }
    if response.skipped {
        if !response.answers.is_empty() {
            return Err(ApiError::bad_request(
                "a skipped planning question response cannot contain answers",
            ));
        }
        return Ok(response);
    }
    if response.answers.len() != request.questions.len() {
        return Err(ApiError::bad_request(
            "every planning question requires exactly one answer",
        ));
    }
    let mut answers_by_question = HashMap::new();
    for mut answer in response.answers {
        answer.question_id = answer.question_id.trim().to_string();
        if answer.question_id.is_empty()
            || answers_by_question
                .insert(answer.question_id.clone(), answer)
                .is_some()
        {
            return Err(ApiError::bad_request(
                "user input response contains a missing or duplicate question id",
            ));
        }
    }

    let mut answers = Vec::with_capacity(request.questions.len());
    for question in &request.questions {
        let mut answer = answers_by_question
            .remove(&question.id)
            .ok_or_else(|| ApiError::bad_request(format!("missing answer for {}", question.id)))?;
        answer.option_id = answer
            .option_id
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        answer.custom_text = answer
            .custom_text
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        match (&answer.option_id, &answer.custom_text) {
            (Some(option_id), None) => {
                if !question
                    .options
                    .iter()
                    .any(|option| option.id == *option_id)
                {
                    return Err(ApiError::bad_request(format!(
                        "unknown option {option_id} for question {}",
                        question.id
                    )));
                }
            }
            (None, Some(custom_text)) => {
                if !question.allow_custom {
                    return Err(ApiError::bad_request(format!(
                        "question {} does not allow a custom answer",
                        question.id
                    )));
                }
                if custom_text.chars().count() > 1_000 {
                    return Err(ApiError::bad_request(format!(
                        "custom answer for {} exceeds 1000 characters",
                        question.id
                    )));
                }
            }
            _ => {
                return Err(ApiError::bad_request(format!(
                    "question {} requires either one option or one custom answer",
                    question.id
                )));
            }
        }
        answers.push(answer);
    }
    if !answers_by_question.is_empty() {
        return Err(ApiError::bad_request(
            "user input response contains an unknown question id",
        ));
    }
    Ok(UserInputResponse {
        answers,
        skipped: false,
        cancelled: false,
    })
}

async fn list_approvals(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Query(query): Query<ApprovalQuery>,
) -> Result<Json<Vec<Approval>>, ApiError> {
    ensure_thread(&state, thread_id)?;
    Ok(Json(state.store.list_approvals(thread_id, query.status)?))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApprovalDecisionRequest {
    approved: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct ApprovalDecisionResponse {
    accepted: bool,
    executed: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct UserInputResponseAccepted {
    accepted: bool,
    resumed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExternalActionResumeRequest {
    #[serde(default)]
    observation: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct ExternalActionResumeResponse {
    accepted: bool,
    resumed: bool,
    turn_id: Uuid,
    invocation_id: u64,
}

#[derive(Debug, Deserialize)]
struct ApprovalQuery {
    status: Option<ApprovalStatus>,
}

#[derive(Debug, Deserialize)]
struct UserInputQuery {
    status: Option<UserInputStatus>,
}
