use crate::{cancel_thread_turn, ApiError, AppState};
use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use opentopia_core::{
    retry_managed_office_runtime_install, retry_managed_powershell_install, OfficeRuntimeStatus,
    SessionStore, ShellRuntimeStatus,
};
use serde::Serialize;
use std::collections::HashSet;
use std::time::Duration;
use tokio::time::{sleep, Instant};
use tracing::warn;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/runtime/powershell/retry",
            post(retry_managed_powershell),
        )
        .route(
            "/api/runtime/office/retry",
            post(retry_managed_office_runtime),
        )
        .route("/api/runtime/prepare-shutdown", post(prepare_shutdown))
}

async fn retry_managed_powershell() -> Json<ShellRuntimeStatus> {
    Json(retry_managed_powershell_install())
}

async fn retry_managed_office_runtime() -> Json<OfficeRuntimeStatus> {
    Json(retry_managed_office_runtime_install())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ShutdownPreparationResult {
    requested: usize,
    completed: usize,
    remaining: usize,
    cancellation_errors: usize,
}

async fn prepare_shutdown(
    State(state): State<AppState>,
) -> Result<Json<ShutdownPreparationResult>, ApiError> {
    state.shutdown.begin();
    let mut requested_turn_ids = HashSet::new();
    let mut cancellation_errors = 0;
    let deadline = Instant::now() + Duration::from_secs(6);
    let mut empty_since = None;
    let remaining = loop {
        let active = state.store.list_active_turns()?;
        for turn in &active {
            if !requested_turn_ids.insert(turn.turn_id) {
                continue;
            }
            if let Err(error) = cancel_thread_turn(&state, turn.thread_id, Some(turn.turn_id)).await
            {
                cancellation_errors += 1;
                warn!(
                    ?error,
                    thread_id = %turn.thread_id,
                    turn_id = %turn.turn_id,
                    "failed to cancel active Turn during shutdown preparation"
                );
            }
        }
        let now = Instant::now();
        if active.is_empty() {
            let quiet_since = empty_since.get_or_insert(now);
            if now.duration_since(*quiet_since) >= Duration::from_millis(100) {
                break 0;
            }
        } else {
            empty_since = None;
        }
        if now >= deadline {
            break active.len();
        }
        sleep(Duration::from_millis(50)).await;
    };

    Ok(Json(ShutdownPreparationResult {
        requested: requested_turn_ids.len(),
        completed: requested_turn_ids.len().saturating_sub(remaining),
        remaining,
        cancellation_errors,
    }))
}
