use super::{
    background_scope, decode_typed_tool_input, derived_tool_schema, enforce_policy_decision,
    required_typed_string, Tool, ToolExecutionPolicy, ToolInvocationContext, TypedTool,
    DEFAULT_BACKGROUND_TIMEOUT_SECONDS, DEFAULT_FOREGROUND_YIELD_MILLISECONDS,
    MAX_BACKGROUND_TIMEOUT_SECONDS, MAX_FOREGROUND_YIELD_MILLISECONDS,
};
use crate::browser::{
    BrowserAction, BrowserActionReceipt, BrowserContent, BrowserDownloadRequest,
    BrowserNavigateRequest, BrowserNetworkGrant, BrowserNodeRef, BrowserObservation,
    BrowserObservationId, BrowserObserveOptions, BrowserRuntime, BrowserSelector, BrowserSessionId,
    BrowserWaitCondition, BrowserWaitRequest,
};
use crate::execution_authorization::ToolExecutionIntent;
use crate::model::{ModelContentPart, ToolCall, ToolResult};
use crate::policy::PolicyDecision;
use anyhow::Context;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Copy)]
enum BrowserActionInput {
    Navigate,
    Observe,
    Screenshot,
    Click,
    Type,
    Select,
    Hover,
    Scroll,
    SwitchTarget,
    Wait,
    Download,
    Close,
}

impl BrowserActionInput {
    fn as_str(self) -> &'static str {
        match self {
            Self::Navigate => "navigate",
            Self::Observe => "observe",
            Self::Screenshot => "screenshot",
            Self::Click => "click",
            Self::Type => "type",
            Self::Select => "select",
            Self::Hover => "hover",
            Self::Scroll => "scroll",
            Self::SwitchTarget => "switch_target",
            Self::Wait => "wait",
            Self::Download => "download",
            Self::Close => "close",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum BrowserWaitConditionInput {
    #[default]
    DocumentComplete,
    Selector,
    Text,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(
    tag = "action",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(super) enum BrowserInput {
    #[schemars(rename_all = "camelCase")]
    Navigate {
        url: String,
        #[serde(default)]
        #[schemars(range(min = 1, max = 120000))]
        timeout_ms: Option<u64>,
    },
    #[schemars(rename_all = "camelCase")]
    Observe {
        #[serde(default)]
        include_screenshot: bool,
    },
    #[schemars(rename_all = "camelCase")]
    Screenshot {},
    #[schemars(rename_all = "camelCase")]
    Click {
        observation_id: String,
        node_ref: String,
    },
    #[schemars(rename_all = "camelCase")]
    Type {
        observation_id: String,
        node_ref: String,
        text: String,
        #[serde(default = "default_true")]
        clear_first: bool,
    },
    #[schemars(rename_all = "camelCase")]
    Select {
        observation_id: String,
        node_ref: String,
        value: String,
    },
    #[schemars(rename_all = "camelCase")]
    Hover {
        observation_id: String,
        node_ref: String,
    },
    #[schemars(rename_all = "camelCase")]
    Scroll {
        observation_id: String,
        node_ref: String,
        #[serde(default)]
        #[schemars(range(min = -10000.0, max = 10000.0))]
        delta_x: f64,
        #[serde(default)]
        #[schemars(range(min = -10000.0, max = 10000.0))]
        delta_y: f64,
    },
    #[schemars(rename_all = "camelCase")]
    SwitchTarget { target_ref: String },
    #[schemars(rename_all = "camelCase")]
    Wait {
        #[serde(default)]
        condition: BrowserWaitConditionInput,
        #[serde(default)]
        selector: Option<String>,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        #[schemars(range(min = 1, max = 120000))]
        timeout_ms: Option<u64>,
    },
    #[schemars(rename_all = "camelCase")]
    Download {
        url: String,
        #[serde(default)]
        #[schemars(range(min = 1, max = 21600000))]
        timeout_ms: Option<u64>,
        #[serde(default)]
        #[schemars(range(min = 1, max = 120000))]
        yield_time_ms: Option<u64>,
        #[serde(default)]
        expected_filename: Option<String>,
    },
    #[schemars(rename_all = "camelCase")]
    Close {},
}

struct BrowserExecutionInput {
    action: BrowserActionInput,
    url: Option<String>,
    selector: Option<String>,
    observation_id: Option<String>,
    node_ref: Option<String>,
    include_screenshot: bool,
    text: Option<String>,
    value: Option<String>,
    clear_first: bool,
    delta_x: f64,
    delta_y: f64,
    target_ref: Option<String>,
    condition: BrowserWaitConditionInput,
    timeout_ms: Option<u64>,
    yield_time_ms: Option<u64>,
    expected_filename: Option<String>,
}

impl From<BrowserInput> for BrowserExecutionInput {
    fn from(input: BrowserInput) -> Self {
        let mut execution = Self {
            action: BrowserActionInput::Close,
            url: None,
            selector: None,
            observation_id: None,
            node_ref: None,
            include_screenshot: false,
            text: None,
            value: None,
            clear_first: true,
            delta_x: 0.0,
            delta_y: 0.0,
            target_ref: None,
            condition: BrowserWaitConditionInput::DocumentComplete,
            timeout_ms: None,
            yield_time_ms: None,
            expected_filename: None,
        };
        match input {
            BrowserInput::Navigate { url, timeout_ms } => {
                execution.action = BrowserActionInput::Navigate;
                execution.url = Some(url);
                execution.timeout_ms = timeout_ms;
            }
            BrowserInput::Observe { include_screenshot } => {
                execution.action = BrowserActionInput::Observe;
                execution.include_screenshot = include_screenshot;
            }
            BrowserInput::Screenshot {} => execution.action = BrowserActionInput::Screenshot,
            BrowserInput::Click {
                observation_id,
                node_ref,
            } => {
                execution.action = BrowserActionInput::Click;
                execution.observation_id = Some(observation_id);
                execution.node_ref = Some(node_ref);
            }
            BrowserInput::Type {
                observation_id,
                node_ref,
                text,
                clear_first,
            } => {
                execution.action = BrowserActionInput::Type;
                execution.observation_id = Some(observation_id);
                execution.node_ref = Some(node_ref);
                execution.text = Some(text);
                execution.clear_first = clear_first;
            }
            BrowserInput::Select {
                observation_id,
                node_ref,
                value,
            } => {
                execution.action = BrowserActionInput::Select;
                execution.observation_id = Some(observation_id);
                execution.node_ref = Some(node_ref);
                execution.value = Some(value);
            }
            BrowserInput::Hover {
                observation_id,
                node_ref,
            } => {
                execution.action = BrowserActionInput::Hover;
                execution.observation_id = Some(observation_id);
                execution.node_ref = Some(node_ref);
            }
            BrowserInput::Scroll {
                observation_id,
                node_ref,
                delta_x,
                delta_y,
            } => {
                execution.action = BrowserActionInput::Scroll;
                execution.observation_id = Some(observation_id);
                execution.node_ref = Some(node_ref);
                execution.delta_x = delta_x;
                execution.delta_y = delta_y;
            }
            BrowserInput::SwitchTarget { target_ref } => {
                execution.action = BrowserActionInput::SwitchTarget;
                execution.target_ref = Some(target_ref);
            }
            BrowserInput::Wait {
                condition,
                selector,
                text,
                timeout_ms,
            } => {
                execution.action = BrowserActionInput::Wait;
                execution.condition = condition;
                execution.selector = selector;
                execution.text = text;
                execution.timeout_ms = timeout_ms;
            }
            BrowserInput::Download {
                url,
                timeout_ms,
                yield_time_ms,
                expected_filename,
            } => {
                execution.action = BrowserActionInput::Download;
                execution.url = Some(url);
                execution.timeout_ms = timeout_ms;
                execution.yield_time_ms = yield_time_ms;
                execution.expected_filename = expected_filename;
            }
            BrowserInput::Close {} => {}
        }
        execution
    }
}

pub struct BrowserTool;

/// Signals that a browser interaction must be completed by the user in the visible page.
/// This is distinct from an approval: the agent must stop controlling the page rather than retry
/// the same operation after a yes/no decision.
#[derive(Debug, Clone, Error)]
#[error("{reason}")]
pub struct BrowserHandoffRequired {
    pub action: String,
    pub reason: String,
    pub url: Option<String>,
}

pub fn browser_handoff_required(error: &anyhow::Error) -> Option<&BrowserHandoffRequired> {
    error.downcast_ref::<BrowserHandoffRequired>()
}

pub fn browser_handoff_for_node(
    action: &str,
    node: &crate::browser::BrowserNode,
    url: Option<String>,
) -> Option<BrowserHandoffRequired> {
    if !node.requires_user_action {
        return None;
    }
    Some(BrowserHandoffRequired {
        action: action.to_string(),
        reason: node.user_action_reason.clone().unwrap_or_else(|| {
            "This page requires you to complete the action yourself before I continue.".to_string()
        }),
        url,
    })
}

#[async_trait]
impl TypedTool for BrowserTool {
    type Input = BrowserInput;

    fn name(&self) -> &str {
        "browser"
    }

    fn description(&self) -> &str {
        "Use the shared local browser. Observe before every click, type, select, hover, or scroll, then use the returned observationId and nodeRef. Observations include owned tabs/popups, frames, and a bounded accessibility tree. Use switch_target with a returned targetRef to change tabs. The runtime rejects stale observations; if it reports stale_observation, discard the old node reference and observe again. When a page requires a login, verification, upload, payment, publication, or irreversible submission, stop controlling the page and tell the user to complete it in the visible browser."
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let input = BrowserExecutionInput::from(input);
        let runtime = ctx
            .browser
            .as_ref()
            .context("browser runtime is unavailable")?
            .clone();
        let thread_id = ctx.thread_id.context("browser requires a thread context")?;
        let session = BrowserSessionId::from_thread(thread_id);
        let action = input.action.as_str().to_string();
        let timeout = input.timeout_ms.map(|milliseconds| {
            let maximum = if matches!(input.action, BrowserActionInput::Download) {
                MAX_BACKGROUND_TIMEOUT_SECONDS * 1_000
            } else {
                120_000
            };
            Duration::from_millis(milliseconds.clamp(1, maximum))
        });
        let output = match input.action {
            BrowserActionInput::Navigate => {
                let url = required_typed_string(input.url.as_deref(), "url")?;
                let host = inspect_browser_destination(&ctx, &url)?;
                grant_browser_network_access(&ctx, &runtime, session, [host]).await?;
                let mut request = BrowserNavigateRequest::new(url);
                if let Some(wait) = request.wait.as_mut() {
                    wait.timeout = timeout;
                }
                runtime.navigate(session, request).await?
            }
            BrowserActionInput::Observe => {
                grant_browser_network_access(&ctx, &runtime, session, std::iter::empty::<String>())
                    .await?;
                let observation = runtime
                    .observe(
                        session,
                        BrowserObserveOptions {
                            include_screenshot: input.include_screenshot,
                        },
                    )
                    .await?;
                return Ok(browser_observation_to_tool_result(
                    call_id,
                    action,
                    observation,
                    None,
                ));
            }
            BrowserActionInput::Screenshot => {
                grant_browser_network_access(&ctx, &runtime, session, std::iter::empty::<String>())
                    .await?;
                runtime.screenshot(session).await?
            }
            BrowserActionInput::Click => {
                let observation_id = browser_observation_id(input.observation_id.as_deref())?;
                let node_ref = browser_node_ref(input.node_ref.as_deref())?;
                let target = runtime
                    .observation_node(session, observation_id, node_ref)
                    .await?;
                if let Some(handoff) =
                    browser_handoff_for_node(&action, &target, target.href.clone())
                {
                    return Err(handoff.into());
                }
                let hosts = inspect_browser_node_destinations(&ctx, &target)?;
                grant_browser_network_access(&ctx, &runtime, session, hosts).await?;
                let receipt = runtime
                    .perform(session, observation_id, node_ref, BrowserAction::Click)
                    .await?;
                let observation = runtime
                    .observe(session, BrowserObserveOptions::default())
                    .await?;
                return Ok(browser_observation_to_tool_result(
                    call_id,
                    action,
                    observation,
                    Some(receipt),
                ));
            }
            BrowserActionInput::Type => {
                let observation_id = browser_observation_id(input.observation_id.as_deref())?;
                let node_ref = browser_node_ref(input.node_ref.as_deref())?;
                let target = runtime
                    .observation_node(session, observation_id, node_ref)
                    .await?;
                if let Some(handoff) = browser_handoff_for_node(&action, &target, None) {
                    return Err(handoff.into());
                }
                grant_browser_network_access(&ctx, &runtime, session, std::iter::empty::<String>())
                    .await?;
                let receipt = runtime
                    .perform(
                        session,
                        observation_id,
                        node_ref,
                        BrowserAction::Type {
                            text: required_typed_string(input.text.as_deref(), "text")?,
                            clear_first: input.clear_first,
                        },
                    )
                    .await?;
                let observation = runtime
                    .observe(session, BrowserObserveOptions::default())
                    .await?;
                return Ok(browser_observation_to_tool_result(
                    call_id,
                    action,
                    observation,
                    Some(receipt),
                ));
            }
            BrowserActionInput::Select => {
                let observation_id = browser_observation_id(input.observation_id.as_deref())?;
                let node_ref = browser_node_ref(input.node_ref.as_deref())?;
                let target = runtime
                    .observation_node(session, observation_id, node_ref)
                    .await?;
                if let Some(handoff) = browser_handoff_for_node(&action, &target, None) {
                    return Err(handoff.into());
                }
                grant_browser_network_access(&ctx, &runtime, session, std::iter::empty::<String>())
                    .await?;
                let receipt = runtime
                    .perform(
                        session,
                        observation_id,
                        node_ref,
                        BrowserAction::Select {
                            value: required_typed_string(input.value.as_deref(), "value")?,
                        },
                    )
                    .await?;
                let observation = runtime
                    .observe(session, BrowserObserveOptions::default())
                    .await?;
                return Ok(browser_observation_to_tool_result(
                    call_id,
                    action,
                    observation,
                    Some(receipt),
                ));
            }
            BrowserActionInput::Hover | BrowserActionInput::Scroll => {
                let observation_id = browser_observation_id(input.observation_id.as_deref())?;
                let node_ref = browser_node_ref(input.node_ref.as_deref())?;
                grant_browser_network_access(&ctx, &runtime, session, std::iter::empty::<String>())
                    .await?;
                if !input.delta_x.is_finite()
                    || !input.delta_y.is_finite()
                    || input.delta_x.abs() > 10_000.0
                    || input.delta_y.abs() > 10_000.0
                {
                    anyhow::bail!("scroll deltas must be finite values between -10000 and 10000");
                }
                let browser_action = if matches!(input.action, BrowserActionInput::Hover) {
                    BrowserAction::Hover
                } else {
                    BrowserAction::Scroll {
                        delta_x: input.delta_x,
                        delta_y: input.delta_y,
                    }
                };
                let receipt = runtime
                    .perform(session, observation_id, node_ref, browser_action)
                    .await?;
                let observation = runtime
                    .observe(session, BrowserObserveOptions::default())
                    .await?;
                return Ok(browser_observation_to_tool_result(
                    call_id,
                    action,
                    observation,
                    Some(receipt),
                ));
            }
            BrowserActionInput::SwitchTarget => {
                let target_ref = serde_json::from_value(Value::String(required_typed_string(
                    input.target_ref.as_deref(),
                    "targetRef",
                )?))
                .context("targetRef must be a browser target reference")?;
                grant_browser_network_access(&ctx, &runtime, session, std::iter::empty::<String>())
                    .await?;
                runtime.switch_target(session, target_ref).await?;
                let observation = runtime
                    .observe(session, BrowserObserveOptions::default())
                    .await?;
                return Ok(browser_observation_to_tool_result(
                    call_id,
                    action,
                    observation,
                    None,
                ));
            }
            BrowserActionInput::Wait => {
                grant_browser_network_access(&ctx, &runtime, session, std::iter::empty::<String>())
                    .await?;
                let condition = match input.condition {
                    BrowserWaitConditionInput::DocumentComplete => {
                        BrowserWaitCondition::DocumentComplete
                    }
                    BrowserWaitConditionInput::Selector => {
                        BrowserWaitCondition::Selector(BrowserSelector::new(
                            required_typed_string(input.selector.as_deref(), "selector")?,
                        )?)
                    }
                    BrowserWaitConditionInput::Text => BrowserWaitCondition::Text(
                        required_typed_string(input.text.as_deref(), "text")?,
                    ),
                };
                runtime
                    .wait(
                        session,
                        BrowserWaitRequest {
                            condition,
                            timeout,
                            poll_interval: Duration::from_millis(100),
                        },
                    )
                    .await?
            }
            BrowserActionInput::Download => {
                let url = required_typed_string(input.url.as_deref(), "url")?;
                let host = inspect_browser_destination(&ctx, &url)?;
                grant_browser_network_access(&ctx, &runtime, session, [host]).await?;
                let request = BrowserDownloadRequest {
                    url: url.clone(),
                    expected_filename: input.expected_filename,
                    timeout: Some(
                        timeout.unwrap_or(Duration::from_secs(DEFAULT_BACKGROUND_TIMEOUT_SECONDS)),
                    ),
                };
                if let (Some(registry), Some(_)) = (ctx.background.as_ref(), ctx.thread_id) {
                    let scope = background_scope(&ctx)?;
                    let task_runtime = runtime.clone();
                    let job = registry.spawn_task(
                        scope.clone(),
                        format!("browser download {url}"),
                        ctx.cancel.clone(),
                        async move {
                            let output = task_runtime.download(session, request).await?;
                            serde_json::to_string(&output)
                                .context("failed to serialize browser download result")
                        },
                    )?;
                    let yield_time_ms = input
                        .yield_time_ms
                        .unwrap_or(DEFAULT_FOREGROUND_YIELD_MILLISECONDS)
                        .clamp(1, MAX_FOREGROUND_YIELD_MILLISECONDS);
                    if let Some(chunk) = registry
                        .wait_for_output(&scope, job.job_id, Duration::from_millis(yield_time_ms))
                        .await?
                    {
                        anyhow::ensure!(
                            chunk.job.success,
                            "browser download failed: {}",
                            chunk
                                .job
                                .error
                                .as_deref()
                                .unwrap_or("unknown background error")
                        );
                        serde_json::from_str(&chunk.stdout)
                            .context("invalid browser download result from background registry")?
                    } else {
                        let value = json!({
                            "jobId": job.job_id,
                            "status": job.status.as_str(),
                            "action": action,
                            "url": url,
                            "startedAt": job.started_at,
                            "autoDetached": true,
                            "yieldTimeMs": yield_time_ms,
                            "note": "The download is still running. Carry on with independent work; completion is delivered automatically. Use background_output only to stop it or to wait when no independent work remains."
                        });
                        return Ok(ToolResult {
                            call_id,
                            output: serde_json::to_string_pretty(&value)?,
                            content: vec![ModelContentPart::json(value)],
                            metadata: json!({
                                "toolName": "browser",
                                "action": action,
                                "background": true,
                                "autoDetached": true,
                                "yieldTimeMs": yield_time_ms,
                                "jobId": job.job_id,
                                "url": url,
                                "success": true
                            }),
                        });
                    }
                } else {
                    runtime.download(session, request).await?
                }
            }
            BrowserActionInput::Close => {
                runtime.close_session(session).await?;
                return Ok(ToolResult::text(
                    call_id,
                    "Closed this browser tab.",
                    json!({ "sessionId": session.to_string(), "action": action }),
                ));
            }
        };
        Ok(browser_output_to_tool_result(call_id, action, output))
    }
}

impl_typed_tool!(BrowserTool);

fn inspect_browser_destination(
    ctx: &ToolInvocationContext,
    raw_url: &str,
) -> anyhow::Result<String> {
    let host = browser_destination_host(raw_url)?;
    enforce_policy_decision(ctx.policy.inspect_network(&host), ctx)?;
    Ok(host)
}

fn inspect_browser_node_destinations(
    ctx: &ToolInvocationContext,
    node: &crate::browser::BrowserNode,
) -> anyhow::Result<Vec<String>> {
    let mut inspected = HashSet::new();
    for destination in [node.href.as_deref(), node.form_action.as_deref()]
        .into_iter()
        .flatten()
    {
        let host = browser_destination_host(destination)?;
        if inspected.insert(host.clone()) {
            enforce_policy_decision(ctx.policy.inspect_network(&host), ctx)?;
        }
    }
    Ok(inspected.into_iter().collect())
}

async fn grant_browser_network_access<I>(
    ctx: &ToolInvocationContext,
    runtime: &Arc<dyn BrowserRuntime>,
    session: BrowserSessionId,
    explicit_hosts: I,
) -> anyhow::Result<()>
where
    I: IntoIterator<Item = String>,
{
    let mut hosts = configured_browser_hosts(ctx)?;
    hosts.extend(explicit_hosts);
    runtime
        .grant_network_access(session, BrowserNetworkGrant::new(hosts)?)
        .await?;
    Ok(())
}

pub(super) fn configured_browser_hosts(
    ctx: &ToolInvocationContext,
) -> anyhow::Result<HashSet<String>> {
    let (Some(store), Some(thread_id)) = (ctx.state.as_ref(), ctx.thread_id) else {
        return Ok(HashSet::new());
    };
    let settings =
        store.effective_plugin_settings("browser-automation", &ctx.workspace_root, thread_id)?;
    let Some(domains) = settings.get("allowedDomains") else {
        return Ok(HashSet::new());
    };
    let domains = domains
        .as_array()
        .context("browser-automation allowedDomains must be an array")?;
    let mut hosts = HashSet::new();
    for domain in domains {
        let domain = domain
            .as_str()
            .context("browser-automation allowedDomains entries must be strings")?;
        let grant = BrowserNetworkGrant::new([domain]).with_context(|| {
            format!("invalid browser-automation allowedDomains entry `{domain}`")
        })?;
        for host in grant.allowed_hosts {
            if !matches!(
                ctx.policy.inspect_network(&host),
                PolicyDecision::Deny { .. }
            ) {
                hosts.insert(host);
            }
        }
    }
    Ok(hosts)
}

pub(super) fn browser_destination_host(raw_url: &str) -> anyhow::Result<String> {
    let url =
        reqwest::Url::parse(raw_url).context("browser destination must be an absolute URL")?;
    if !matches!(url.scheme(), "http" | "https") {
        anyhow::bail!(
            "browser destination uses a blocked URL scheme: {}",
            url.scheme()
        );
    }
    if !url.username().is_empty() || url.password().is_some() {
        anyhow::bail!("browser destination must not contain embedded credentials");
    }
    url.host_str()
        .map(str::to_ascii_lowercase)
        .context("browser destination must contain a host")
}

fn browser_output_to_tool_result(
    call_id: Uuid,
    action: String,
    output: crate::browser::BrowserOutput,
) -> ToolResult {
    let mut rendered = Vec::new();
    let mut content = Vec::new();
    for item in output.contents {
        match item {
            BrowserContent::Text { text, truncated } => {
                if truncated {
                    rendered.push(format!("{text}\n\n[Browser text truncated]"));
                } else {
                    rendered.push(text.clone());
                }
                content.push(ModelContentPart::text(text));
            }
            BrowserContent::Json { value } => {
                rendered.push(value.to_string());
                content.push(ModelContentPart::json(value));
            }
            BrowserContent::Image { mime_type, bytes } => {
                rendered.push(format!("[Browser screenshot: {} bytes]", bytes.len()));
                content.push(ModelContentPart::image(mime_type, bytes));
            }
            BrowserContent::File {
                path,
                mime_type,
                bytes,
            } => {
                rendered.push(format!(
                    "[Browser download: {} ({} bytes)]",
                    path.display(),
                    bytes
                ));
                content.push(ModelContentPart::resource(
                    path.to_string_lossy(),
                    mime_type,
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .map(str::to_string),
                ));
            }
        }
    }
    ToolResult {
        call_id,
        output: rendered.join("\n\n"),
        content,
        metadata: json!({ "toolName": "browser", "action": action, "url": output.url, "browser": output.metadata }),
    }
}

fn browser_observation_id(input: Option<&str>) -> anyhow::Result<BrowserObservationId> {
    serde_json::from_value(Value::String(required_typed_string(
        input,
        "observationId",
    )?))
    .context("observationId must be a browser observation ID")
}

fn browser_node_ref(input: Option<&str>) -> anyhow::Result<BrowserNodeRef> {
    serde_json::from_value(Value::String(required_typed_string(input, "nodeRef")?))
        .context("nodeRef must be a browser node reference")
}

fn browser_observation_to_tool_result(
    call_id: Uuid,
    action: String,
    observation: BrowserObservation,
    receipt: Option<BrowserActionReceipt>,
) -> ToolResult {
    let mut rendered = vec![observation.text.clone()];
    let mut content = vec![ModelContentPart::text(observation.text.clone())];
    if let Some(receipt) = &receipt {
        rendered.push(serde_json::to_string(receipt).unwrap_or_default());
        content.push(ModelContentPart::json(
            serde_json::to_value(receipt).unwrap_or(Value::Null),
        ));
    }
    let mut structured_observation = observation.clone();
    if let Some(screenshot) = structured_observation.screenshot.take() {
        rendered.push(format!(
            "[Browser screenshot: {} bytes]",
            screenshot.bytes.len()
        ));
        content.push(ModelContentPart::image(
            screenshot.mime_type,
            screenshot.bytes,
        ));
    }
    rendered.push(serde_json::to_string(&structured_observation).unwrap_or_default());
    content.push(ModelContentPart::json(
        serde_json::to_value(&structured_observation).unwrap_or(Value::Null),
    ));
    ToolResult {
        call_id,
        output: rendered.join("\n\n"),
        content,
        metadata: json!({
            "toolName": "browser",
            "action": action,
            "url": observation.url,
            "browser": {
                "observation": structured_observation,
                "receipt": receipt,
            },
        }),
    }
}
