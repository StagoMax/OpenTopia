use crate::{
    browser_handoff_for_node, current_settings, ensure_thread, ApiError, AppState, BrowserAction,
    BrowserActionReceipt, BrowserContent, BrowserDownloadRequest, BrowserNavigateRequest,
    BrowserNodeRef, BrowserObservation, BrowserObservationId, BrowserObserveOptions, BrowserOutput,
    BrowserRuntimeRoute, BrowserSelector, BrowserSessionId, BrowserSessionSpec, BrowserTargetRef,
    BrowserWaitCondition, BrowserWaitRequest, ComputerSessionId, ObserveOptions, SandboxDescriptor,
};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/threads/:thread_id/sandbox", get(get_sandbox))
        .route("/api/threads/:thread_id/browser", post(run_browser_command))
        .route(
            "/api/threads/:thread_id/browser/runtime",
            get(get_browser_runtime).post(bind_browser_runtime),
        )
        .route(
            "/api/threads/:thread_id/computer/windows",
            get(list_computer_windows),
        )
        .route(
            "/api/threads/:thread_id/computer/observe",
            post(observe_computer_window),
        )
        .route(
            "/api/threads/:thread_id/computer/session",
            post(close_computer_session),
        )
}

async fn get_sandbox(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<SandboxDescriptor>, ApiError> {
    let thread = ensure_thread(&state, thread_id)?;
    Ok(Json(SandboxDescriptor::local(
        thread_id,
        thread.workspace_root,
        &current_settings(&state).sandbox.to_local_sandbox_config(),
    )))
}

/// This surface is deliberately read-only. It lets the desktop panel make an explicit user
/// selection and view that one window, while all input injection remains inside AgentCore's
/// approval flow through the `computer` tool.
async fn list_computer_windows(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<Vec<opentopia_core::WindowTarget>>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let session = ComputerSessionId::from_thread(thread_id);
    let windows = state
        .computer
        .list_windows(session)
        .await
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    Ok(Json(windows))
}

async fn observe_computer_window(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(request): Json<ComputerObserveRequest>,
) -> Result<Json<opentopia_core::ComputerObservation>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let session = ComputerSessionId::from_thread(thread_id);
    let target = state
        .computer
        .list_windows(session)
        .await
        .map_err(|error| ApiError::bad_request(error.to_string()))?
        .into_iter()
        .find(|target| target.window_id == request.window_id)
        .ok_or_else(|| {
            ApiError::bad_request("windowId is not a visible controllable desktop window")
        })?;
    let observation = state
        .computer
        .observe(session, target, ObserveOptions::default())
        .await
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    Ok(Json(observation))
}

async fn close_computer_session(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    ensure_thread(&state, thread_id)?;
    state
        .computer
        .close_session(ComputerSessionId::from_thread(thread_id))
        .await
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn run_browser_command(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(request): Json<BrowserCommandRequest>,
) -> Result<Json<BrowserOutput>, ApiError> {
    let thread = ensure_thread(&state, thread_id)?;
    let _workspace_root = thread.workspace_root;
    let session = BrowserSessionId::from_thread(thread_id);
    let timeout = request
        .timeout_ms
        .map(|milliseconds| Duration::from_millis(milliseconds.clamp(1, 120_000)));
    let result = match request.action.as_str() {
        "navigate" => {
            let url = browser_required(&request.url, "url")?;
            let mut command = BrowserNavigateRequest::new(url);
            if let Some(timeout) = timeout {
                command.wait = Some(BrowserWaitRequest {
                    condition: BrowserWaitCondition::DocumentComplete,
                    timeout: Some(timeout),
                    poll_interval: Duration::from_millis(100),
                });
            }
            state.browser.navigate(session, command).await
        }
        "observe" => {
            let observation = state
                .browser
                .observe(
                    session,
                    BrowserObserveOptions {
                        include_screenshot: request.include_screenshot.unwrap_or(false),
                    },
                )
                .await
                .map_err(|error| ApiError::bad_request(error.to_string()))?;
            return Ok(Json(browser_observation_output(observation, None)));
        }
        "screenshot" => state.browser.screenshot(session).await,
        "click" => {
            let observation_id = browser_observation_required(request.observation_id)?;
            let node_ref = browser_node_required(request.node_ref)?;
            let target = state
                .browser
                .observation_node(session, observation_id, node_ref)
                .await
                .map_err(|error| ApiError::bad_request(error.to_string()))?;
            if let Some(handoff) = browser_handoff_for_node("click", &target, target.href.clone()) {
                return Err(ApiError::conflict(handoff.reason));
            }
            let receipt = state
                .browser
                .perform(session, observation_id, node_ref, BrowserAction::Click)
                .await
                .map_err(|error| ApiError::bad_request(error.to_string()))?;
            let observation = state
                .browser
                .observe(session, BrowserObserveOptions::default())
                .await
                .map_err(|error| ApiError::bad_request(error.to_string()))?;
            return Ok(Json(browser_observation_output(observation, Some(receipt))));
        }
        "type" => {
            let observation_id = browser_observation_required(request.observation_id)?;
            let node_ref = browser_node_required(request.node_ref)?;
            let target = state
                .browser
                .observation_node(session, observation_id, node_ref)
                .await
                .map_err(|error| ApiError::bad_request(error.to_string()))?;
            if let Some(handoff) = browser_handoff_for_node("type", &target, None) {
                return Err(ApiError::conflict(handoff.reason));
            }
            let receipt = state
                .browser
                .perform(
                    session,
                    observation_id,
                    node_ref,
                    BrowserAction::Type {
                        text: browser_required(&request.text, "text")?.to_string(),
                        clear_first: request.clear_first.unwrap_or(true),
                    },
                )
                .await
                .map_err(|error| ApiError::bad_request(error.to_string()))?;
            let observation = state
                .browser
                .observe(session, BrowserObserveOptions::default())
                .await
                .map_err(|error| ApiError::bad_request(error.to_string()))?;
            return Ok(Json(browser_observation_output(observation, Some(receipt))));
        }
        "select" => {
            let observation_id = browser_observation_required(request.observation_id)?;
            let node_ref = browser_node_required(request.node_ref)?;
            let target = state
                .browser
                .observation_node(session, observation_id, node_ref)
                .await
                .map_err(|error| ApiError::bad_request(error.to_string()))?;
            if let Some(handoff) = browser_handoff_for_node("select", &target, None) {
                return Err(ApiError::conflict(handoff.reason));
            }
            let receipt = state
                .browser
                .perform(
                    session,
                    observation_id,
                    node_ref,
                    BrowserAction::Select {
                        value: browser_required(&request.value, "value")?.to_string(),
                    },
                )
                .await
                .map_err(|error| ApiError::bad_request(error.to_string()))?;
            let observation = state
                .browser
                .observe(session, BrowserObserveOptions::default())
                .await
                .map_err(|error| ApiError::bad_request(error.to_string()))?;
            return Ok(Json(browser_observation_output(observation, Some(receipt))));
        }
        "hover" | "scroll" => {
            let observation_id = browser_observation_required(request.observation_id)?;
            let node_ref = browser_node_required(request.node_ref)?;
            let delta_x = request.delta_x.unwrap_or(0.0);
            let delta_y = request.delta_y.unwrap_or(0.0);
            if !delta_x.is_finite()
                || !delta_y.is_finite()
                || delta_x.abs() > 10_000.0
                || delta_y.abs() > 10_000.0
            {
                return Err(ApiError::bad_request(
                    "browser scroll deltas must be finite values between -10000 and 10000",
                ));
            }
            let action = if request.action == "hover" {
                BrowserAction::Hover
            } else {
                BrowserAction::Scroll { delta_x, delta_y }
            };
            let receipt = state
                .browser
                .perform(session, observation_id, node_ref, action)
                .await
                .map_err(|error| ApiError::bad_request(error.to_string()))?;
            let observation = state
                .browser
                .observe(session, BrowserObserveOptions::default())
                .await
                .map_err(|error| ApiError::bad_request(error.to_string()))?;
            return Ok(Json(browser_observation_output(observation, Some(receipt))));
        }
        "switch_target" => {
            let target_ref = request
                .target_ref
                .ok_or_else(|| ApiError::bad_request("browser targetRef is required"))?;
            state
                .browser
                .switch_target(session, target_ref)
                .await
                .map_err(|error| ApiError::bad_request(error.to_string()))?;
            let observation = state
                .browser
                .observe(session, BrowserObserveOptions::default())
                .await
                .map_err(|error| ApiError::bad_request(error.to_string()))?;
            return Ok(Json(browser_observation_output(observation, None)));
        }
        "wait" => {
            let condition = match request.condition.as_deref().unwrap_or("document_complete") {
                "document_complete" => BrowserWaitCondition::DocumentComplete,
                "selector" => BrowserWaitCondition::Selector(
                    BrowserSelector::new(browser_required(&request.selector, "selector")?)
                        .map_err(|error| ApiError::bad_request(error.to_string()))?,
                ),
                "text" => {
                    BrowserWaitCondition::Text(browser_required(&request.text, "text")?.to_string())
                }
                other => {
                    return Err(ApiError::bad_request(format!(
                        "unsupported browser wait condition: {other}"
                    )))
                }
            };
            state
                .browser
                .wait(
                    session,
                    BrowserWaitRequest {
                        condition,
                        timeout,
                        poll_interval: Duration::from_millis(100),
                    },
                )
                .await
        }
        "download" => {
            let url = browser_required(&request.url, "url")?;
            state
                .browser
                .download(
                    session,
                    BrowserDownloadRequest {
                        url: url.to_string(),
                        expected_filename: request.expected_filename,
                        timeout,
                    },
                )
                .await
        }
        "close" => {
            state
                .browser
                .close_session(session)
                .await
                .map_err(|error| ApiError::bad_request(error.to_string()))?;
            return Ok(Json(BrowserOutput {
                url: None,
                contents: Vec::new(),
                metadata: json!({ "action": "close" }),
            }));
        }
        other => {
            return Err(ApiError::bad_request(format!(
                "unsupported browser action: {other}"
            )))
        }
    }
    .map_err(|error| ApiError::bad_request(error.to_string()))?;
    Ok(Json(result))
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrowserRuntimeStatus {
    pub(crate) route: BrowserRuntimeRoute,
    pub(crate) chrome_available: bool,
}

async fn get_browser_runtime(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<BrowserRuntimeStatus>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let session = BrowserSessionId::from_thread(thread_id);
    Ok(Json(BrowserRuntimeStatus {
        route: state.browser_router.route_for(session).await,
        chrome_available: state.browser_router.chrome_available(),
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BindBrowserRuntimeRequest {
    route: BrowserRuntimeRoute,
}

async fn bind_browser_runtime(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(request): Json<BindBrowserRuntimeRequest>,
) -> Result<Json<BrowserRuntimeStatus>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let session = BrowserSessionId::from_thread(thread_id);
    state
        .browser_router
        .bind(BrowserSessionSpec::from(session), request.route)
        .await
        .map_err(|error| ApiError::conflict(error.to_string()))?;
    Ok(Json(BrowserRuntimeStatus {
        route: request.route,
        chrome_available: state.browser_router.chrome_available(),
    }))
}

fn browser_required<'a>(value: &'a Option<String>, field: &str) -> Result<&'a str, ApiError> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::bad_request(format!("browser {field} is required")))
}

fn browser_observation_required(
    value: Option<BrowserObservationId>,
) -> Result<BrowserObservationId, ApiError> {
    value.ok_or_else(|| ApiError::bad_request("browser observationId is required"))
}

fn browser_node_required(value: Option<BrowserNodeRef>) -> Result<BrowserNodeRef, ApiError> {
    value.ok_or_else(|| ApiError::bad_request("browser nodeRef is required"))
}

fn browser_observation_output(
    observation: BrowserObservation,
    receipt: Option<BrowserActionReceipt>,
) -> BrowserOutput {
    // Transport the screenshot only as an image content block. Keeping it out of the
    // structured observation prevents the direct browser API from duplicating PNG bytes.
    let mut response_observation = observation;
    let screenshot = response_observation.screenshot.take();
    let mut contents = vec![
        BrowserContent::Text {
            text: response_observation.text.clone(),
            truncated: response_observation.text_truncated,
        },
        BrowserContent::Json {
            value: serde_json::to_value(&response_observation).unwrap_or(Value::Null),
        },
    ];
    if let Some(screenshot) = screenshot {
        contents.push(BrowserContent::Image {
            mime_type: screenshot.mime_type,
            bytes: screenshot.bytes,
        });
    }
    if let Some(receipt) = &receipt {
        contents.push(BrowserContent::Json {
            value: serde_json::to_value(receipt).unwrap_or(Value::Null),
        });
    }
    BrowserOutput {
        url: Some(response_observation.url.clone()),
        contents,
        metadata: json!({
            "action": receipt.as_ref().map(|value| value.action.as_str()).unwrap_or("observe"),
            "observation": response_observation,
            "receipt": receipt,
        }),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserCommandRequest {
    action: String,
    url: Option<String>,
    selector: Option<String>,
    observation_id: Option<BrowserObservationId>,
    node_ref: Option<BrowserNodeRef>,
    text: Option<String>,
    value: Option<String>,
    clear_first: Option<bool>,
    delta_x: Option<f64>,
    delta_y: Option<f64>,
    target_ref: Option<BrowserTargetRef>,
    include_screenshot: Option<bool>,
    condition: Option<String>,
    timeout_ms: Option<u64>,
    expected_filename: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ComputerObserveRequest {
    window_id: String,
}
