use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use futures_util::stream::{self, StreamExt};
use opentopia_core::{
    AgentCore, AgentEvent, AgentEventPayload, AgentTurnInput, Message, MessageRole, PermissionMode,
    SessionStore, SqliteSessionStore,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::{error, info};
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(name = "opentopia-server")]
struct Args {
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value_t = 8787)]
    port: u16,
    #[arg(long, env = "OPENTOPIA_DB", default_value = ".opentopia/opentopia.db")]
    db: PathBuf,
    #[arg(long, env = "OPENTOPIA_PERMISSION", default_value = "auto")]
    permission: PermissionMode,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "opentopia_server=info,tower_http=info".into()),
        )
        .init();

    let args = Args::parse();
    let store = Arc::new(SqliteSessionStore::open(&args.db)?);
    let state = AppState {
        store,
        agent: Arc::new(AgentCore::from_env()),
        events: EventBus::default(),
        permission_mode: args.permission,
        approvals: PendingApprovals::default(),
    };

    let app = build_router(state);
    let addr: SocketAddr = format!("{}:{}", args.host, args.port).parse()?;
    let listener = TcpListener::bind(addr).await?;
    info!(%addr, db = %args.db.display(), "OpenTopia server listening");
    axum::serve(listener, app).await?;
    Ok(())
}

fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/threads", get(list_threads).post(create_thread))
        .route("/api/threads/:thread_id/messages", get(list_messages).post(send_message))
        .route("/api/threads/:thread_id/events", get(list_events))
        .route("/api/threads/:thread_id/events/stream", get(stream_events))
        .route(
            "/api/threads/:thread_id/approvals/:approval_id/decision",
            post(decide_approval),
        )
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[derive(Clone)]
struct AppState {
    store: Arc<SqliteSessionStore>,
    agent: Arc<AgentCore>,
    events: EventBus,
    permission_mode: PermissionMode,
    approvals: PendingApprovals,
}

#[derive(Clone, Default)]
struct PendingApprovals {
    items: Arc<RwLock<HashMap<Uuid, PendingApproval>>>,
}

#[derive(Debug, Clone)]
struct PendingApproval {
    thread_id: Uuid,
    action: String,
}

impl PendingApprovals {
    fn insert(&self, approval_id: Uuid, approval: PendingApproval) {
        self.items
            .write()
            .expect("approval store poisoned")
            .insert(approval_id, approval);
    }

    fn remove(&self, approval_id: Uuid) -> Option<PendingApproval> {
        self.items
            .write()
            .expect("approval store poisoned")
            .remove(&approval_id)
    }
}

#[derive(Clone, Default)]
struct EventBus {
    channels: Arc<RwLock<HashMap<Uuid, broadcast::Sender<AgentEvent>>>>,
}

impl EventBus {
    fn subscribe(&self, thread_id: Uuid) -> broadcast::Receiver<AgentEvent> {
        let mut channels = self.channels.write().expect("event bus poisoned");
        channels
            .entry(thread_id)
            .or_insert_with(|| {
                let (tx, _rx) = broadcast::channel(256);
                tx
            })
            .subscribe()
    }

    fn publish(&self, event: AgentEvent) {
        let sender = {
            let mut channels = self.channels.write().expect("event bus poisoned");
            channels
                .entry(event.thread_id)
                .or_insert_with(|| {
                    let (tx, _rx) = broadcast::channel(256);
                    tx
                })
                .clone()
        };
        let _ = sender.send(event);
    }
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        service: "opentopia-server",
    })
}

async fn list_threads(State(state): State<AppState>) -> Result<Json<Vec<opentopia_core::Thread>>, ApiError> {
    Ok(Json(state.store.list_threads()?))
}

async fn create_thread(
    State(state): State<AppState>,
    Json(request): Json<CreateThreadRequest>,
) -> Result<Json<opentopia_core::Thread>, ApiError> {
    let workspace_root = request
        .workspace_root
        .unwrap_or(std::env::current_dir().map_err(anyhow::Error::from)?);
    let workspace_root = workspace_root.canonicalize().unwrap_or(workspace_root);
    let thread = state.store.create_thread(request.title, workspace_root)?;
    Ok(Json(thread))
}

async fn list_messages(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<Vec<Message>>, ApiError> {
    ensure_thread(&state, thread_id)?;
    Ok(Json(state.store.list_messages(thread_id)?))
}

async fn send_message(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(request): Json<SendMessageRequest>,
) -> Result<Json<Message>, ApiError> {
    let thread = ensure_thread(&state, thread_id)?;
    if request.content.trim().is_empty() {
        return Err(ApiError::bad_request("message content cannot be empty"));
    }

    let user_message = state.store.append_message(Message::text(
        thread_id,
        MessageRole::User,
        request.content.clone(),
    ))?;

    let run_state = state.clone();
    let run_message = user_message.clone();
    tokio::spawn(async move {
        let turn_id = Uuid::new_v4();
        let input = AgentTurnInput {
            thread_id,
            user_message_id: run_message.id,
            workspace_root: thread.workspace_root,
            content: request.content,
            permission_mode: run_state.permission_mode,
        };

        match run_state.agent.run_turn(input).await {
            Ok(payloads) => {
                for payload in payloads {
                    if let AgentEventPayload::AssistantMessage { message } = &payload {
                        if let Err(err) = run_state.store.append_message(message.clone()) {
                            error!(?err, "failed to persist assistant message");
                        }
                    }
                    if let AgentEventPayload::ApprovalRequested {
                        approval_id,
                        action,
                        ..
                    } = &payload
                    {
                        run_state.approvals.insert(
                            *approval_id,
                            PendingApproval {
                                thread_id,
                                action: action.clone(),
                            },
                        );
                    }
                    publish_payload(&run_state, thread_id, Some(turn_id), payload);
                }
            }
            Err(err) => {
                publish_payload(
                    &run_state,
                    thread_id,
                    Some(turn_id),
                    AgentEventPayload::Error {
                        message: err.to_string(),
                    },
                );
            }
        }
    });

    Ok(Json(user_message))
}

async fn decide_approval(
    State(state): State<AppState>,
    Path((thread_id, approval_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<ApprovalDecisionRequest>,
) -> Result<Json<ApprovalDecisionResponse>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let pending = state
        .approvals
        .remove(approval_id)
        .ok_or_else(|| ApiError::not_found(format!("approval not found: {approval_id}")))?;
    if pending.thread_id != thread_id {
        return Err(ApiError::bad_request("approval does not belong to this thread"));
    }

    if !request.approved {
        let message = state.store.append_message(Message::text(
            thread_id,
            MessageRole::Assistant,
            "Approval denied. The requested action was not executed.",
        ))?;
        publish_payload(
            &state,
            thread_id,
            Some(Uuid::new_v4()),
            AgentEventPayload::AssistantMessage { message },
        );
        return Ok(Json(ApprovalDecisionResponse {
            accepted: true,
            executed: false,
        }));
    }

    let thread = ensure_thread(&state, thread_id)?;
    let run_state = state.clone();
    tokio::spawn(async move {
        let turn_id = Uuid::new_v4();
        let input = AgentTurnInput {
            thread_id,
            user_message_id: approval_id,
            workspace_root: thread.workspace_root,
            content: pending.action,
            permission_mode: PermissionMode::FullAccess,
        };
        match run_state.agent.run_turn(input).await {
            Ok(payloads) => {
                for payload in payloads {
                    if let AgentEventPayload::AssistantMessage { message } = &payload {
                        if let Err(err) = run_state.store.append_message(message.clone()) {
                            error!(?err, "failed to persist approved assistant message");
                        }
                    }
                    publish_payload(&run_state, thread_id, Some(turn_id), payload);
                }
            }
            Err(err) => publish_payload(
                &run_state,
                thread_id,
                Some(turn_id),
                AgentEventPayload::Error {
                    message: err.to_string(),
                },
            ),
        }
    });

    Ok(Json(ApprovalDecisionResponse {
        accepted: true,
        executed: true,
    }))
}

async fn list_events(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Query(query): Query<EventQuery>,
) -> Result<Json<Vec<AgentEvent>>, ApiError> {
    ensure_thread(&state, thread_id)?;
    Ok(Json(state.store.list_events(thread_id, query.since)?))
}

async fn stream_events(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Query(query): Query<EventQuery>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let history = state.store.list_events(thread_id, query.since)?;
    let rx = state.events.subscribe(thread_id);
    let history_stream = stream::iter(history);
    let live_stream = BroadcastStream::new(rx).filter_map(|event| async move { event.ok() });
    let event_stream = history_stream.chain(live_stream).map(|agent_event| {
        let event_name = sse_event_name(agent_event.kind());
        let sse = Event::default()
            .event(event_name)
            .json_data(agent_event)
            .expect("agent event should serialize");
        Ok(sse)
    });

    Ok(Sse::new(event_stream).keep_alive(KeepAlive::default()))
}

fn sse_event_name(kind: &str) -> &str {
    if kind == "error" {
        "agent_error"
    } else {
        kind
    }
}

fn ensure_thread(state: &AppState, thread_id: Uuid) -> Result<opentopia_core::Thread, ApiError> {
    state
        .store
        .get_thread(thread_id)?
        .ok_or_else(|| ApiError::not_found(format!("thread not found: {thread_id}")))
}

fn publish_payload(
    state: &AppState,
    thread_id: Uuid,
    turn_id: Option<Uuid>,
    payload: AgentEventPayload,
) {
    let event = AgentEvent::new(thread_id, turn_id, 0, payload);
    match state.store.append_event(event) {
        Ok(event) => state.events.publish(event),
        Err(err) => error!(?err, "failed to persist event"),
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HealthResponse {
    ok: bool,
    service: &'static str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateThreadRequest {
    title: Option<String>,
    workspace_root: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendMessageRequest {
    content: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApprovalDecisionRequest {
    approved: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ApprovalDecisionResponse {
    accepted: bool,
    executed: bool,
}

#[derive(Debug, Deserialize)]
struct EventQuery {
    since: Option<i64>,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(value: anyhow::Error) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: value.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(serde_json::json!({
            "error": self.message,
        }));
        (self.status, body).into_response()
    }
}
