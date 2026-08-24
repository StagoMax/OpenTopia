use super::{ensure_thread, ApiError, AppState};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::stream::{self, StreamExt};
use opentopia_core::collaboration::{
    AgentActivitySource, AgentAvailability, AgentListItem, AgentThreadId, CollaborationRegistry,
};
use opentopia_core::{
    AgentActivityNotification, AgentEvent, AgentEventPayload, DesktopStreamEnvelope,
    DesktopStreamKind, SessionStore, TurnRecord,
};
use serde::Deserialize;
use serde_json::Value;
use std::convert::Infallible;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use uuid::Uuid;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/threads/:thread_id/events", get(list_events))
        .route("/api/threads/:thread_id/events/stream", get(stream_events))
        .route("/api/activity/events/stream", get(stream_activity_events))
        .route("/api/activity/statuses", get(list_activity_statuses))
        .route("/api/threads/:thread_id/agents", get(list_agent_threads))
        .route(
            "/api/threads/:thread_id/agents/:agent_thread_id/interrupt",
            post(interrupt_agent_thread),
        )
        .route(
            "/api/threads/:thread_id/agents/events/stream",
            get(stream_agent_events),
        )
}

async fn list_activity_statuses(
    State(state): State<AppState>,
) -> Result<Json<Vec<TurnRecord>>, ApiError> {
    let store = state.store.clone();
    let statuses = tokio::task::spawn_blocking(move || store.list_live_turn_statuses())
        .await
        .map_err(|error| ApiError::internal(format!("activity status task failed: {error}")))??;
    Ok(Json(statuses))
}

async fn stream_activity_events(
    State(state): State<AppState>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.events.subscribe_activity();
    let event_stream = BroadcastStream::new(rx)
        .filter_map(|result| async move { result.ok() })
        .map(|agent_event| {
            let event_name = sse_event_name(agent_event.kind());
            let seq = agent_event.seq;
            let envelope =
                DesktopStreamEnvelope::new(DesktopStreamKind::AgentEvent, seq, agent_event);
            let sse = Event::default()
                .id(seq.to_string())
                .event(event_name)
                .json_data(envelope)
                .expect("thread activity event should serialize");
            Ok(sse)
        });

    Sse::new(event_stream).keep_alive(KeepAlive::default())
}

async fn list_events(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Query(query): Query<EventQuery>,
) -> Result<Json<Vec<AgentEvent>>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let store = state.store.clone();
    let events = tokio::task::spawn_blocking(move || match query.view {
        Some(EventView::Conversation) if query.limit.is_some() || query.before.is_some() => store
            .list_conversation_event_page(
                thread_id,
                query.since,
                query.before,
                query.limit.unwrap_or(250),
            ),
        Some(EventView::Conversation) => store.list_conversation_events(thread_id, query.since),
        None => store.list_events(thread_id, query.since),
    })
    .await
    .map_err(|error| ApiError::internal(format!("event history task failed: {error}")))??;
    Ok(Json(events))
}

async fn stream_events(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Query(query): Query<EventQuery>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let rx = state.events.subscribe(thread_id);
    let conversation_view = matches!(query.view, Some(EventView::Conversation));
    let history = if conversation_view {
        state
            .store
            .list_conversation_events(thread_id, query.since)?
    } else {
        state.store.list_events(thread_id, query.since)?
    };
    let event_stream = replay_then_live_events(history, rx, query.since)
        .filter_map(move |agent_event| async move {
            if conversation_view {
                project_conversation_event(agent_event)
            } else {
                Some(agent_event)
            }
        })
        .map(|agent_event| {
            let event_name = sse_event_name(agent_event.kind());
            let seq = agent_event.seq;
            let envelope =
                DesktopStreamEnvelope::new(DesktopStreamKind::AgentEvent, seq, agent_event);
            let sse = Event::default()
                .id(seq.to_string())
                .event(event_name)
                .json_data(envelope)
                .expect("agent event should serialize");
            Ok(sse)
        });

    Ok(Sse::new(event_stream).keep_alive(KeepAlive::default()))
}

async fn list_agent_threads(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<Vec<AgentListItem>>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let Some(session) = state
        .collaboration_repository
        .find_session_by_user_task_id(thread_id)?
    else {
        return Ok(Json(Vec::new()));
    };
    let mut result = Vec::new();
    for agent in state
        .collaboration_repository
        .list_threads(session.id)
        .await?
    {
        let latest_turn = state.collaboration_repository.latest_turn(agent.id).await?;
        let activity = match latest_turn.as_ref() {
            Some(turn) => Some(
                state
                    .agent_activity
                    .read_activity(
                        agent.id,
                        turn.id,
                        turn.status,
                        opentopia_core::collaboration::ActivityQuery::default(),
                    )
                    .await
                    .map_err(|error| ApiError::internal(error.to_string()))?,
            ),
            None => None,
        };
        result.push(AgentListItem {
            availability: AgentAvailability::derive(&agent, latest_turn.as_ref()),
            agent,
            latest_turn,
            activity,
        });
    }
    Ok(Json(result))
}

async fn interrupt_agent_thread(
    State(state): State<AppState>,
    Path((thread_id, agent_thread_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    ensure_thread(&state, thread_id)?;
    let session = state
        .collaboration_repository
        .find_session_by_user_task_id(thread_id)?
        .ok_or_else(|| ApiError::not_found("collaboration session has not started"))?;
    let agent = state
        .collaboration_repository
        .get_thread(AgentThreadId::from_uuid(agent_thread_id))
        .await?;
    if agent.session_id != session.id {
        return Err(ApiError::not_found(
            "agent thread was not found in this task",
        ));
    }
    state.collaboration_runtime.interrupt_agent(&agent).await?;
    Ok(StatusCode::ACCEPTED)
}

async fn stream_agent_events(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Query(query): Query<EventQuery>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let session = state
        .collaboration_repository
        .find_session_by_user_task_id(thread_id)?
        .ok_or_else(|| ApiError::not_found("collaboration session has not started"))?;
    let cursor = query.since.unwrap_or_default();
    let stream_state = (state, session.id, cursor);
    let event_stream = stream::unfold(stream_state, |(state, session_id, mut cursor)| async move {
        loop {
            let changed = state
                .agent_activity
                .wait_for_change(session_id, None, Some(cursor), Duration::from_secs(30))
                .await;
            match changed {
                Ok(Some(agent_thread_id)) => {
                    let next_cursor = match state
                        .collaboration_repository
                        .latest_session_activity_cursor(session_id)
                    {
                        Ok(next_cursor) if next_cursor > cursor => next_cursor,
                        Ok(_) => continue,
                        Err(error) => {
                            let sse = Event::default().event("error").data(error.to_string());
                            return Some((Ok(sse), (state, session_id, cursor)));
                        }
                    };
                    cursor = next_cursor;
                    let notification = AgentActivityNotification {
                        seq: cursor,
                        agent_thread_id,
                    };
                    let envelope = DesktopStreamEnvelope::new(
                        DesktopStreamKind::AgentActivity,
                        cursor,
                        notification,
                    );
                    let sse = Event::default()
                        .id(cursor.to_string())
                        .event("activity")
                        .json_data(envelope)
                        .expect("Agent activity notification should serialize");
                    return Some((Ok(sse), (state, session_id, cursor)));
                }
                Ok(None) => continue,
                Err(error) => {
                    let sse = Event::default().event("error").data(error.to_string());
                    return Some((Ok(sse), (state, session_id, cursor)));
                }
            }
        }
    });
    Ok(Sse::new(event_stream).keep_alive(KeepAlive::default()))
}

pub(super) fn project_conversation_event(mut event: AgentEvent) -> Option<AgentEvent> {
    event.payload = project_conversation_payload(event.payload)?;
    Some(event)
}

pub(super) fn project_conversation_payload(
    mut payload: AgentEventPayload,
) -> Option<AgentEventPayload> {
    match &mut payload {
        AgentEventPayload::ReasoningDelta { .. } => return None,
        AgentEventPayload::ModelContextBuilt { items, .. } => items.clear(),
        AgentEventPayload::ModelRequest { request, .. } => *request = Value::Null,
        AgentEventPayload::ProviderRequestSent { body, .. }
        | AgentEventPayload::ProviderRequestRetried { body, .. }
        | AgentEventPayload::ProviderResponseReceived { body, .. } => *body = Value::Null,
        AgentEventPayload::ToolCallFinished { result } => result.content.clear(),
        _ => {}
    }
    Some(payload)
}

pub(super) fn replay_then_live_events(
    history: Vec<AgentEvent>,
    rx: broadcast::Receiver<AgentEvent>,
    after_seq: Option<i64>,
) -> impl futures_util::Stream<Item = AgentEvent> {
    let mut last_seq = history
        .last()
        .map(|event| event.seq)
        .unwrap_or_else(|| after_seq.unwrap_or_default());
    let history_stream = stream::iter(history);
    let live_stream = BroadcastStream::new(rx).filter_map(move |result| {
        let event = match result {
            Ok(event) if event.seq > last_seq => {
                last_seq = event.seq;
                Some(event)
            }
            _ => None,
        };
        async move { event }
    });
    history_stream.chain(live_stream)
}

fn sse_event_name(kind: &str) -> &str {
    if kind == "error" {
        "agent_error"
    } else {
        kind
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EventView {
    Conversation,
}

#[derive(Debug, Deserialize)]
struct EventQuery {
    since: Option<i64>,
    before: Option<i64>,
    limit: Option<usize>,
    view: Option<EventView>,
}
