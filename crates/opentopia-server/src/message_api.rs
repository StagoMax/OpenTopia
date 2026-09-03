use super::auth::TURN_ID_HEADER;
use super::context_api::model_user_message_with_attachment_manifest;
use super::library_api;
use super::send_trace::ConversationSendTrace;
use super::{
    ensure_bound_agent_skills_visible, ensure_mode_skills_visible, ensure_plugin_skills_enabled,
    ensure_thread, finish_turn, publish_payload, run_new_agent_turn, ApiError, AppState,
};
use anyhow::Context;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, HeaderValue};
use axum::routing::post;
use axum::{Json, Router};
use opentopia_core::{
    load_context_source_metadata, load_selected_skills, AgentEventPayload, ApprovalStatus,
    CollaborationMode, ContextSourcePolicy, ContextSourceRef, ExperienceMode, GoalSnapshot,
    GoalStatus, Message, MessagePart, MessageRole, SessionStore, SkillRef, TurnInboxItem,
    TurnStatus, UserInputStatus,
};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::PathBuf;
use tracing::error;
use uuid::Uuid;

pub(super) fn router() -> Router<AppState> {
    Router::new().route(
        "/api/threads/:thread_id/messages",
        post(send_message).layer(DefaultBodyLimit::max(MAX_INLINE_IMAGE_BYTES * 5)),
    )
}

async fn send_message(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<SendMessageRequest>,
) -> Result<(HeaderMap, Json<Message>), ApiError> {
    let send_trace = ConversationSendTrace::from_headers(&headers);
    send_trace.phase("request_received", thread_id, None);
    if state.shutdown.is_preparing() {
        return Err(ApiError::conflict(
            "OpenTopia is shutting down and cannot start another Turn",
        ));
    }
    let thread = ensure_thread(&state, thread_id)?;
    let replacing_message_id = request.replace_message_id;
    let replaced_message_created_at = if let Some(message_id) = replacing_message_id {
        if request.delivery != MessageDelivery::QueueNext {
            return Err(ApiError::bad_request(
                "replaceMessageId cannot be used while steering a running Turn",
            ));
        }
        if state.turns.status(thread_id)?.is_some() {
            return Err(ApiError::conflict(
                "wait for the current Turn to finish before editing a message",
            ));
        }
        let existing = state
            .store
            .list_messages(thread_id)?
            .into_iter()
            .find(|message| message.id == message_id && message.role == MessageRole::User)
            .ok_or_else(|| ApiError::not_found("user message to replace was not found"))?;
        if existing.parts.iter().any(|part| {
            !matches!(
                part,
                MessagePart::Text { .. } | MessagePart::TurnContext { .. }
            )
        }) {
            return Err(ApiError::bad_request(
                "only plain text user messages can be replaced",
            ));
        }
        Some(existing.created_at)
    } else {
        None
    };
    let library_provider = request.library_provider;
    if library_provider.is_some() && thread.experience_mode != ExperienceMode::Flow {
        return Err(ApiError::bad_request(
            "libraryProvider is currently available only in Flow conversations",
        ));
    }
    if state
        .turns
        .status(thread_id)?
        .is_some_and(|turn| turn.status == TurnStatus::WaitingUserAction)
    {
        return Err(ApiError::conflict(
            "complete or cancel the pending external action before starting another turn",
        ));
    }
    let image_attachments = request.image_attachments;
    let content_parts = request.content_parts;
    validate_inline_image_attachments(&image_attachments, &content_parts)?;
    if let Some(command) = legacy_direct_tool_command(&request.content) {
        return Err(ApiError::bad_request(format!(
            "{command} is a direct tool command. Use the terminal or file workspace API instead of sending it to the agent."
        )));
    }
    if request.content.trim().is_empty()
        && request.source_paths.is_empty()
        && request.skill_ids.is_empty()
        && image_attachments.is_empty()
    {
        return Err(ApiError::bad_request("message content cannot be empty"));
    }

    let sources =
        load_context_source_metadata(&request.source_paths, &ContextSourcePolicy::default())
            .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let source_refs = sources
        .iter()
        .map(ContextSourceRef::from)
        .collect::<Vec<_>>();
    let (ordered_content_parts, inline_source_ids) =
        resolve_inline_message_parts(content_parts, &source_refs)?;
    // Explicit Skill selection is structured user input. Load its bounded main prompt once,
    // persist only the reference, and inject the instructions into this Turn's user context.
    let loaded_skills = load_selected_skills(Some(&thread.workspace_root), &request.skill_ids)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    ensure_mode_skills_visible(thread.experience_mode, &loaded_skills)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    ensure_plugin_skills_enabled(&state.store, &thread, &loaded_skills)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    ensure_bound_agent_skills_visible(&state, &thread, &loaded_skills)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let pinned_skills = loaded_skills.iter().map(SkillRef::from).collect::<Vec<_>>();
    let prompt = if request.content.trim().is_empty() {
        match (sources.is_empty(), loaded_skills.is_empty()) {
            (false, false) => "Use the selected Skill instructions to review the attached sources.",
            (true, false) => "Follow the selected Skill instructions for this task.",
            _ => "Review the attached sources.",
        }
        .to_string()
    } else {
        request.content.clone()
    };
    send_trace.phase("context_inputs_loaded", thread_id, None);

    let steer_turn = if request.delivery == MessageDelivery::SteerCurrent {
        Some(
            state
                .turns
                .status(thread_id)?
                .filter(|turn| turn.status == TurnStatus::Running)
                .ok_or_else(|| ApiError::conflict("there is no running Turn to steer"))?,
        )
    } else {
        None
    };

    if steer_turn.is_none()
        && !state
            .store
            .list_approvals(thread_id, Some(ApprovalStatus::Pending))?
            .is_empty()
    {
        return Err(ApiError::conflict(
            "resolve the pending approval before starting another turn",
        ));
    }
    if steer_turn.is_none()
        && !state
            .store
            .list_user_input_requests(thread_id, Some(UserInputStatus::Pending))?
            .is_empty()
    {
        return Err(ApiError::conflict(
            "answer or dismiss the pending user decision before starting another turn",
        ));
    }

    let collaboration_mode = request.collaboration_mode;
    let goal_snapshot = if steer_turn.is_some() {
        None
    } else {
        resolve_message_goal(
            &state,
            thread_id,
            collaboration_mode,
            request.goal_id,
            &prompt,
        )?
    };
    if let Some(snapshot) = goal_snapshot.as_ref() {
        publish_payload(
            &state,
            thread_id,
            None,
            AgentEventPayload::GoalUpdated {
                snapshot: snapshot.clone(),
            },
        );
    }
    send_trace.phase("admission_checks_completed", thread_id, None);

    let mut pending_message = if !ordered_content_parts.is_empty() {
        let mut message = Message::text(thread_id, MessageRole::User, "");
        message.parts = ordered_content_parts;
        message
    } else {
        Message::text(thread_id, MessageRole::User, prompt.clone())
    };
    pending_message.parts.push(MessagePart::TurnContext {
        collaboration_mode,
        goal_id: goal_snapshot.as_ref().map(|snapshot| snapshot.goal.id),
        library_provider: library_provider.map(|provider| provider.as_str().to_string()),
    });
    pending_message.parts.extend(
        source_refs
            .into_iter()
            .filter(|source| !inline_source_ids.contains(&source.id))
            .map(|source| MessagePart::SourceRef {
                source,
                inline: Some(false),
            }),
    );
    pending_message.parts.extend(
        image_attachments
            .into_iter()
            .map(|image| MessagePart::Image {
                id: Some(image.id),
                content_type: image.content_type,
                data: image.data,
                name: image.name,
            }),
    );
    pending_message.parts.extend(
        pinned_skills
            .into_iter()
            .map(|skill| MessagePart::SkillRef { skill }),
    );
    if let Some(message_id) = replacing_message_id {
        pending_message.id = message_id;
        if let Some(created_at) = replaced_message_created_at {
            pending_message.created_at = created_at;
        }
    }
    let replaced_message = if replacing_message_id.is_some() {
        send_trace.phase("message_replacement_started", thread_id, None);
        let message = state.store.replace_message(pending_message.clone())?;
        send_trace.phase("message_replaced", thread_id, None);
        Some(message)
    } else {
        None
    };
    if let Some(active) = steer_turn {
        send_trace.phase(
            "message_persistence_started",
            thread_id,
            Some(active.turn_id),
        );
        let user_message = state.store.append_message(pending_message)?;
        send_trace.phase("message_persisted", thread_id, Some(active.turn_id));
        let content = model_user_message_with_attachment_manifest(&user_message, &prompt);
        state.turn_inbox.push(
            active.turn_id,
            TurnInboxItem::Steer {
                message_id: user_message.id,
                content,
            },
        );
        let mut headers = HeaderMap::new();
        headers.insert(
            TURN_ID_HEADER,
            HeaderValue::from_str(&active.turn_id.to_string())
                .expect("turn IDs are valid header values"),
        );
        headers.insert("x-opentopia-steered", HeaderValue::from_static("true"));
        send_trace.phase("response_ready", thread_id, Some(active.turn_id));
        send_trace.apply_response_headers(&mut headers);
        return Ok((headers, Json(user_message)));
    }
    send_trace.phase("turn_reservation_started", thread_id, None);
    let turn = state
        .turns
        .begin(thread_id, pending_message.id)
        .map_err(ApiError::from)?;
    let turn = match turn {
        Ok(turn) => turn,
        Err(_) => {
            send_trace.phase("message_persistence_started", thread_id, None);
            let user_message = match replaced_message.clone() {
                Some(message) => message,
                None => state.store.append_message(pending_message)?,
            };
            send_trace.phase("message_persisted", thread_id, None);
            state
                .store
                .enqueue_turn_message(thread_id, user_message.id)?;
            let _ = state.turn_queue.send(thread_id);
            let mut headers = HeaderMap::new();
            headers.insert("x-opentopia-queued", HeaderValue::from_static("true"));
            send_trace.phase("response_ready", thread_id, None);
            send_trace.apply_response_headers(&mut headers);
            return Ok((headers, Json(user_message)));
        }
    };
    let turn_id = turn.turn_id;
    send_trace.phase("turn_reserved", thread_id, Some(turn_id));
    send_trace.phase("message_persistence_started", thread_id, Some(turn_id));
    let user_message_result = match replaced_message {
        Some(message) => Ok(message),
        None => state.store.append_message(pending_message),
    };
    let user_message = match user_message_result {
        Ok(message) => message,
        Err(err) => {
            finish_turn(
                &state,
                thread_id,
                turn.turn_id,
                TurnStatus::Failed,
                Some(err.to_string()),
            );
            return Err(err.into());
        }
    };
    send_trace.phase("message_persisted", thread_id, Some(turn_id));
    let run_state = state.clone();
    let run_message = user_message.clone();
    let model_content = model_user_message_with_attachment_manifest(&user_message, &prompt);
    let model_user_content = Vec::new();
    tokio::spawn(async move {
        run_new_agent_turn(
            run_state,
            thread,
            run_message,
            model_content,
            model_user_content,
            loaded_skills,
            turn,
            collaboration_mode,
            goal_snapshot.map(|snapshot| snapshot.goal),
            library_provider,
            Some(send_trace),
        )
        .await;
    });
    send_trace.phase("agent_scheduled", thread_id, Some(turn_id));

    let mut headers = HeaderMap::new();
    headers.insert(
        TURN_ID_HEADER,
        HeaderValue::from_str(&turn_id.to_string()).expect("turn IDs are valid header values"),
    );
    send_trace.phase("response_ready", thread_id, Some(turn_id));
    send_trace.apply_response_headers(&mut headers);
    Ok((headers, Json(user_message)))
}

fn resolve_message_goal(
    state: &AppState,
    thread_id: Uuid,
    mode: CollaborationMode,
    requested_goal_id: Option<Uuid>,
    objective: &str,
) -> Result<Option<GoalSnapshot>, ApiError> {
    if mode != CollaborationMode::Goal {
        if requested_goal_id.is_some() {
            return Err(ApiError::bad_request("goalId is only valid in goal mode"));
        }
        return Ok(None);
    }

    let mut snapshot = match requested_goal_id {
        Some(goal_id) => state
            .store
            .get_goal(goal_id)?
            .ok_or_else(|| ApiError::not_found(format!("goal not found: {goal_id}")))?,
        None => state
            .store
            .create_goal(thread_id, objective.trim().to_string(), None)?,
    };
    if snapshot.goal.thread_id != thread_id {
        return Err(ApiError::bad_request(format!(
            "goal {} does not belong to thread {thread_id}",
            snapshot.goal.id
        )));
    }
    if snapshot.status().is_terminal() {
        return Err(ApiError::conflict(format!(
            "goal {} is already {}",
            snapshot.goal.id,
            snapshot.status().as_str()
        )));
    }
    if snapshot.status() != GoalStatus::Active {
        snapshot = state
            .store
            .update_goal_status(thread_id, snapshot.goal.id, GoalStatus::Active)?
            .context("goal disappeared while activating it")?;
    }
    Ok(Some(snapshot))
}

pub(super) fn legacy_direct_tool_command(content: &str) -> Option<&'static str> {
    match content.trim().split_whitespace().next()? {
        command if command.eq_ignore_ascii_case("/run") => Some("/run"),
        command if command.eq_ignore_ascii_case("/read") => Some("/read"),
        _ => None,
    }
}

pub(super) fn message_library_provider(
    message: &Message,
) -> Option<library_api::LibraryProviderId> {
    message.parts.iter().find_map(|part| match part {
        MessagePart::TurnContext {
            library_provider: Some(provider),
            ..
        } => library_api::LibraryProviderId::parse(provider).ok(),
        _ => None,
    })
}

pub(super) fn launch_next_queued_turn(state: &AppState, thread_id: Uuid) {
    if state.shutdown.is_preparing() {
        return;
    }
    match state
        .store
        .list_approvals(thread_id, Some(ApprovalStatus::Pending))
    {
        Ok(approvals) if !approvals.is_empty() => return,
        Ok(_) => {}
        Err(error) => {
            error!(?error, %thread_id, "failed to inspect approvals before queued turn");
            return;
        }
    }
    match state
        .store
        .list_user_input_requests(thread_id, Some(UserInputStatus::Pending))
    {
        Ok(requests) if !requests.is_empty() => return,
        Ok(_) => {}
        Err(error) => {
            error!(?error, %thread_id, "failed to inspect pending user input before queued turn");
            return;
        }
    }
    let queued = match state.store.list_queued_turn_messages(thread_id) {
        Ok(queued) => queued,
        Err(error) => {
            error!(?error, %thread_id, "failed to inspect queued turn messages");
            return;
        }
    };
    let Some(message_id) = queued.first().copied() else {
        return;
    };
    let thread = match state.store.get_thread(thread_id) {
        Ok(Some(thread)) => thread,
        Ok(None) => {
            let _ = state
                .store
                .remove_queued_turn_message(thread_id, message_id);
            return;
        }
        Err(error) => {
            error!(?error, %thread_id, "failed to load queued turn thread");
            return;
        }
    };
    let user_message = match state.store.list_messages(thread_id).and_then(|messages| {
        messages
            .into_iter()
            .find(|message| message.id == message_id)
            .ok_or_else(|| anyhow::anyhow!("queued message no longer exists: {message_id}"))
    }) {
        Ok(message) => message,
        Err(error) => {
            error!(?error, %thread_id, %message_id, "failed to load queued turn message");
            let _ = state
                .store
                .remove_queued_turn_message(thread_id, message_id);
            let _ = state.turn_queue.send(thread_id);
            return;
        }
    };
    let turn = match state.turns.begin(thread_id, message_id) {
        Ok(Ok(turn)) => turn,
        Ok(Err(_)) => return,
        Err(error) => {
            error!(?error, %thread_id, %message_id, "failed to start queued turn");
            return;
        }
    };
    let claim_error = match state
        .store
        .remove_queued_turn_message(thread_id, message_id)
    {
        Ok(true) => None,
        Ok(false) => Some("queued message was already claimed".to_string()),
        Err(error) => Some(format!("failed to claim queued turn: {error}")),
    };
    if let Some(message) = claim_error {
        publish_payload(
            state,
            thread_id,
            Some(turn.turn_id),
            AgentEventPayload::Error {
                message: message.clone(),
            },
        );
        finish_turn(
            state,
            thread_id,
            turn.turn_id,
            TurnStatus::Failed,
            Some(message),
        );
        return;
    }

    let content = user_message
        .parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let skill_ids = user_message
        .parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::SkillRef { skill } => Some(skill.id.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let (collaboration_mode, goal_id, library_provider) = user_message
        .parts
        .iter()
        .find_map(|part| match part {
            MessagePart::TurnContext {
                collaboration_mode,
                goal_id,
                library_provider,
            } => Some((
                *collaboration_mode,
                *goal_id,
                library_provider
                    .as_deref()
                    .and_then(|value| library_api::LibraryProviderId::parse(value).ok()),
            )),
            _ => None,
        })
        .unwrap_or((CollaborationMode::Default, None, None));
    let goal = match goal_id {
        Some(goal_id) => match state.store.get_goal(goal_id) {
            Ok(Some(snapshot)) => Some(snapshot.goal),
            Ok(None) => {
                fail_queued_turn(
                    state,
                    thread_id,
                    turn.turn_id,
                    format!("queued goal no longer exists: {goal_id}"),
                );
                return;
            }
            Err(error) => {
                fail_queued_turn(state, thread_id, turn.turn_id, error.to_string());
                return;
            }
        },
        None if collaboration_mode == CollaborationMode::Goal => {
            fail_queued_turn(
                state,
                thread_id,
                turn.turn_id,
                "queued collaboration mode is missing its goal id".to_string(),
            );
            return;
        }
        None => None,
    };
    let selected_skills = match load_selected_skills(Some(&thread.workspace_root), &skill_ids) {
        Ok(skills) => match ensure_mode_skills_visible(thread.experience_mode, &skills)
            .and_then(|()| ensure_plugin_skills_enabled(&state.store, &thread, &skills))
            .and_then(|()| ensure_bound_agent_skills_visible(&state, &thread, &skills))
        {
            Ok(()) => skills,
            Err(error) => {
                fail_queued_turn(state, thread_id, turn.turn_id, error.to_string());
                return;
            }
        },
        Err(error) => {
            fail_queued_turn(state, thread_id, turn.turn_id, error.to_string());
            return;
        }
    };
    let model_content = model_user_message_with_attachment_manifest(&user_message, &content);
    let user_content = Vec::new();
    let run_state = state.clone();
    tokio::spawn(async move {
        run_new_agent_turn(
            run_state,
            thread,
            user_message,
            model_content,
            user_content,
            selected_skills,
            turn,
            collaboration_mode,
            goal,
            library_provider,
            None,
        )
        .await;
    });
}

fn fail_queued_turn(state: &AppState, thread_id: Uuid, turn_id: Uuid, message: String) {
    publish_payload(
        state,
        thread_id,
        Some(turn_id),
        AgentEventPayload::Error {
            message: message.clone(),
        },
    );
    finish_turn(state, thread_id, turn_id, TurnStatus::Failed, Some(message));
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendMessageRequest {
    content: String,
    #[serde(default)]
    replace_message_id: Option<Uuid>,
    #[serde(default)]
    delivery: MessageDelivery,
    #[serde(default)]
    source_paths: Vec<PathBuf>,
    #[serde(default)]
    skill_ids: Vec<String>,
    #[serde(default)]
    collaboration_mode: CollaborationMode,
    #[serde(default)]
    goal_id: Option<Uuid>,
    #[serde(default)]
    library_provider: Option<library_api::LibraryProviderId>,
    #[serde(default)]
    image_attachments: Vec<InlineImageAttachmentRequest>,
    #[serde(default)]
    content_parts: Vec<InlineMessageContentPartRequest>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum MessageDelivery {
    #[default]
    QueueNext,
    SteerCurrent,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct InlineImageAttachmentRequest {
    pub(super) id: Uuid,
    pub(super) content_type: String,
    pub(super) data: Vec<u8>,
    #[serde(default)]
    pub(super) name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum InlineMessageContentPartRequest {
    Text {
        text: String,
    },
    ImageRef {
        #[serde(rename = "imageId")]
        image_id: Uuid,
    },
    AttachmentRef {
        path: PathBuf,
    },
}

const MAX_INLINE_IMAGE_ATTACHMENTS: usize = 10;
const MAX_INLINE_IMAGE_BYTES: usize = 25 * 1024 * 1024;
const MAX_INLINE_IMAGE_REFERENCES: usize = 100;

pub(super) fn validate_inline_image_attachments(
    attachments: &[InlineImageAttachmentRequest],
    content_parts: &[InlineMessageContentPartRequest],
) -> Result<(), ApiError> {
    if attachments.len() > MAX_INLINE_IMAGE_ATTACHMENTS {
        return Err(ApiError::bad_request(format!(
            "too many image attachments; maximum is {MAX_INLINE_IMAGE_ATTACHMENTS}"
        )));
    }
    if content_parts.len() > MAX_INLINE_IMAGE_REFERENCES {
        return Err(ApiError::bad_request(format!(
            "too many inline message parts; maximum is {MAX_INLINE_IMAGE_REFERENCES}"
        )));
    }
    let mut attachment_ids = HashSet::new();
    let mut total_bytes = 0usize;
    for attachment in attachments {
        if !attachment_ids.insert(attachment.id) {
            return Err(ApiError::bad_request("image attachment IDs must be unique"));
        }
        if !attachment.content_type.starts_with("image/") {
            return Err(ApiError::bad_request(
                "image attachments must use an image content type",
            ));
        }
        if attachment.data.is_empty() || attachment.data.len() > MAX_INLINE_IMAGE_BYTES {
            return Err(ApiError::bad_request(format!(
                "image attachments must be between 1 byte and {MAX_INLINE_IMAGE_BYTES} bytes"
            )));
        }
        total_bytes = total_bytes.saturating_add(attachment.data.len());
        if total_bytes > MAX_INLINE_IMAGE_BYTES {
            return Err(ApiError::bad_request(format!(
                "combined image attachments exceed {MAX_INLINE_IMAGE_BYTES} bytes"
            )));
        }
    }
    if !content_parts.is_empty() {
        let referenced_ids = content_parts
            .iter()
            .filter_map(|part| match part {
                InlineMessageContentPartRequest::ImageRef { image_id } => Some(*image_id),
                InlineMessageContentPartRequest::Text { .. }
                | InlineMessageContentPartRequest::AttachmentRef { .. } => None,
            })
            .collect::<HashSet<_>>();
        if referenced_ids.iter().any(|id| !attachment_ids.contains(id)) {
            return Err(ApiError::bad_request(
                "inline image references must point to an attached image",
            ));
        }
        if attachment_ids.iter().any(|id| !referenced_ids.contains(id)) {
            return Err(ApiError::bad_request(
                "every attached image must be referenced by the message content",
            ));
        }
    }
    Ok(())
}

pub(super) fn resolve_inline_message_parts(
    content_parts: Vec<InlineMessageContentPartRequest>,
    sources: &[ContextSourceRef],
) -> Result<(Vec<MessagePart>, HashSet<Uuid>), ApiError> {
    let mut referenced_source_ids = HashSet::new();
    let parts = content_parts
        .into_iter()
        .map(|part| match part {
            InlineMessageContentPartRequest::Text { text } => Ok(MessagePart::Text { text }),
            InlineMessageContentPartRequest::ImageRef { image_id } => {
                Ok(MessagePart::ImageRef { image_id })
            }
            InlineMessageContentPartRequest::AttachmentRef { path, .. } => {
                let canonical_path = path.canonicalize().map_err(|_| {
                    ApiError::bad_request(format!(
                        "inline attachment reference was not found: {}",
                        path.display()
                    ))
                })?;
                let source = sources
                    .iter()
                    .find(|source| source.path == canonical_path)
                    .ok_or_else(|| {
                        ApiError::bad_request(format!(
                            "inline attachment reference is not a selected source: {}",
                            path.display()
                        ))
                    })?;
                referenced_source_ids.insert(source.id);
                Ok(MessagePart::SourceRef {
                    source: source.clone(),
                    inline: Some(true),
                })
            }
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    Ok((parts, referenced_source_ids))
}
