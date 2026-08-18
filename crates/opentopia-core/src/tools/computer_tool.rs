use super::{
    decode_typed_tool_input, derived_tool_schema, enforce_policy_decision, required_typed_string,
    Tool, ToolExecutionPolicy, ToolInvocationContext, TypedTool,
};
use crate::computer::{
    ComputerAccessPolicy, ComputerAction, ComputerMouseButton, ComputerPolicyContext,
    ComputerRuntime, ComputerSessionId, ObserveOptions, WindowTarget,
};
use crate::execution_authorization::ToolExecutionIntent;
use crate::model::{ModelContentPart, ToolCall, ToolResult};
use crate::policy::PolicyDecision;
use anyhow::Context;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, Clone, Copy)]
enum ComputerActionInput {
    ListWindows,
    Observe,
    Click,
    Type,
    Keypress,
    Scroll,
    Drag,
    Wait,
    Close,
}

impl ComputerActionInput {
    fn as_str(self) -> &'static str {
        match self {
            Self::ListWindows => "list_windows",
            Self::Observe => "observe",
            Self::Click => "click",
            Self::Type => "type",
            Self::Keypress => "keypress",
            Self::Scroll => "scroll",
            Self::Drag => "drag",
            Self::Wait => "wait",
            Self::Close => "close",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum ComputerMouseButtonInput {
    #[default]
    Left,
    Right,
}

impl From<ComputerMouseButtonInput> for ComputerMouseButton {
    fn from(value: ComputerMouseButtonInput) -> Self {
        match value {
            ComputerMouseButtonInput::Left => Self::Left,
            ComputerMouseButtonInput::Right => Self::Right,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
pub(super) enum ComputerKeyInput {
    #[serde(rename = "ENTER")]
    Enter,
    #[serde(rename = "TAB")]
    Tab,
    #[serde(rename = "ESCAPE")]
    Escape,
    #[serde(rename = "BACKSPACE")]
    Backspace,
    #[serde(rename = "LEFT")]
    Left,
    #[serde(rename = "RIGHT")]
    Right,
    #[serde(rename = "UP")]
    Up,
    #[serde(rename = "DOWN")]
    Down,
}

impl ComputerKeyInput {
    fn as_str(self) -> &'static str {
        match self {
            Self::Enter => "ENTER",
            Self::Tab => "TAB",
            Self::Escape => "ESCAPE",
            Self::Backspace => "BACKSPACE",
            Self::Left => "LEFT",
            Self::Right => "RIGHT",
            Self::Up => "UP",
            Self::Down => "DOWN",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(
    tag = "action",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(super) enum ComputerInput {
    #[schemars(rename_all = "camelCase")]
    ListWindows {},
    #[schemars(rename_all = "camelCase")]
    Observe { window_id: String },
    #[schemars(rename_all = "camelCase")]
    Click {
        observation_id: String,
        x: u32,
        y: u32,
        #[serde(default)]
        button: ComputerMouseButtonInput,
    },
    #[schemars(rename_all = "camelCase")]
    Type {
        observation_id: String,
        #[schemars(length(max = 4096))]
        text: String,
    },
    #[schemars(rename_all = "camelCase")]
    Keypress {
        observation_id: String,
        key: ComputerKeyInput,
    },
    #[schemars(rename_all = "camelCase")]
    Scroll {
        observation_id: String,
        #[schemars(range(min = -12000, max = 12000))]
        delta_y: i64,
    },
    #[schemars(rename_all = "camelCase")]
    Drag {
        observation_id: String,
        x: u32,
        y: u32,
        end_x: u32,
        end_y: u32,
    },
    #[schemars(rename_all = "camelCase")]
    Wait {
        observation_id: String,
        #[serde(default)]
        #[schemars(range(min = 1, max = 30000))]
        duration_ms: Option<u64>,
    },
    #[schemars(rename_all = "camelCase")]
    Close {},
}

impl ComputerInput {
    fn action(&self) -> ComputerActionInput {
        match self {
            Self::ListWindows {} => ComputerActionInput::ListWindows,
            Self::Observe { .. } => ComputerActionInput::Observe,
            Self::Click { .. } => ComputerActionInput::Click,
            Self::Type { .. } => ComputerActionInput::Type,
            Self::Keypress { .. } => ComputerActionInput::Keypress,
            Self::Scroll { .. } => ComputerActionInput::Scroll,
            Self::Drag { .. } => ComputerActionInput::Drag,
            Self::Wait { .. } => ComputerActionInput::Wait,
            Self::Close {} => ComputerActionInput::Close,
        }
    }
}

struct ComputerExecutionInput {
    action: ComputerActionInput,
    window_id: Option<String>,
    observation_id: Option<String>,
    x: Option<u64>,
    y: Option<u64>,
    end_x: Option<u64>,
    end_y: Option<u64>,
    button: ComputerMouseButtonInput,
    text: Option<String>,
    key: Option<ComputerKeyInput>,
    delta_y: Option<i64>,
    duration_ms: Option<u64>,
}

impl From<ComputerInput> for ComputerExecutionInput {
    fn from(input: ComputerInput) -> Self {
        let mut execution = Self {
            action: input.action(),
            window_id: None,
            observation_id: None,
            x: None,
            y: None,
            end_x: None,
            end_y: None,
            button: ComputerMouseButtonInput::Left,
            text: None,
            key: None,
            delta_y: None,
            duration_ms: None,
        };
        match input {
            ComputerInput::ListWindows {} | ComputerInput::Close {} => {}
            ComputerInput::Observe { window_id } => execution.window_id = Some(window_id),
            ComputerInput::Click {
                observation_id,
                x,
                y,
                button,
            } => {
                execution.observation_id = Some(observation_id);
                execution.x = Some(u64::from(x));
                execution.y = Some(u64::from(y));
                execution.button = button;
            }
            ComputerInput::Type {
                observation_id,
                text,
            } => {
                execution.observation_id = Some(observation_id);
                execution.text = Some(text);
            }
            ComputerInput::Keypress {
                observation_id,
                key,
            } => {
                execution.observation_id = Some(observation_id);
                execution.key = Some(key);
            }
            ComputerInput::Scroll {
                observation_id,
                delta_y,
            } => {
                execution.observation_id = Some(observation_id);
                execution.delta_y = Some(delta_y);
            }
            ComputerInput::Drag {
                observation_id,
                x,
                y,
                end_x,
                end_y,
            } => {
                execution.observation_id = Some(observation_id);
                execution.x = Some(u64::from(x));
                execution.y = Some(u64::from(y));
                execution.end_x = Some(u64::from(end_x));
                execution.end_y = Some(u64::from(end_y));
            }
            ComputerInput::Wait {
                observation_id,
                duration_ms,
            } => {
                execution.observation_id = Some(observation_id);
                execution.duration_ms = duration_ms;
            }
        }
        execution
    }
}

pub struct ComputerTool;

#[async_trait]
impl TypedTool for ComputerTool {
    type Input = ComputerInput;

    fn name(&self) -> &str {
        "computer"
    }

    fn description(&self) -> &str {
        "Observe and operate an application window from the user's executable allowlist. After implementing or changing visible UI, use read-only observation when visual inspection would materially verify layout, overflow, overlap, focus visibility, loading or error states, or relevant viewport sizes. First list windows, then observe one window. Read-only listing and observation do not grant input control. Every input action must use the latest observationId and requires explicit approval. Never use this tool for passwords, secrets, payments, publishing, deletion, UAC, or the entire desktop."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        match input.action() {
            ComputerActionInput::ListWindows | ComputerActionInput::Observe => {
                ToolExecutionPolicy::read_only(vec!["computer:windows".to_string()])
            }
            _ => ToolExecutionPolicy::conservative(),
        }
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let input = ComputerExecutionInput::from(input);
        let runtime = ctx
            .computer
            .as_ref()
            .context("computer runtime is unavailable")?
            .clone();
        let thread_id = ctx
            .thread_id
            .context("computer requires a thread context")?;
        let session = ComputerSessionId::from_thread(thread_id);
        let action_name = input.action.as_str().to_string();

        match input.action {
            ComputerActionInput::ListWindows => {
                let windows = allowed_computer_windows(
                    runtime.as_ref(),
                    session,
                    &ctx.computer_access_policy,
                )
                .await?;
                let value = json!({
                    "sessionId": session,
                    "windows": windows,
                    "truncated": false,
                    "allowlistConfigured": !ctx.computer_access_policy.is_empty(),
                });
                return Ok(computer_tool_result(
                    call_id,
                    action_name,
                    value,
                    None,
                    None,
                ));
            }
            ComputerActionInput::Observe => {
                let window_id = required_typed_string(input.window_id.as_deref(), "windowId")?;
                let target = allowed_computer_windows(
                    runtime.as_ref(),
                    session,
                    &ctx.computer_access_policy,
                )
                .await?
                .into_iter()
                .find(|target| target.window_id == window_id)
                .context("windowId is not an allowlisted visible desktop window")?;
                let observation = runtime
                    .observe(session, target, ObserveOptions::default())
                    .await?;
                let value = computer_observation_summary(&observation);
                return Ok(computer_tool_result(
                    call_id,
                    action_name,
                    value,
                    Some(observation),
                    None,
                ));
            }
            ComputerActionInput::Close => {
                runtime.close_session(session).await?;
                return Ok(ToolResult::text(
                    call_id,
                    "Closed the desktop computer session for this thread.",
                    json!({ "toolName": "computer", "sessionId": session, "success": true }),
                ));
            }
            _ => {}
        }

        let action = parse_computer_action(input)?;
        if action.contains_sensitive_text() {
            anyhow::bail!("refused: input appears to contain a password, token, or other secret");
        }
        let target = runtime
            .target_for_observation(session, action.observation_id())
            .await?;
        ensure_computer_target_allowed(&ctx.computer_access_policy, &target)?;
        enforce_policy_decision(
            ctx.policy.inspect_computer_action(
                &target,
                &action,
                &ComputerPolicyContext {
                    session_id: session,
                    thread_id: Some(thread_id),
                },
            ),
            ctx.approval_granted,
        )?;
        let receipt = runtime.perform(session, action).await?;
        let observation = runtime
            .observe(session, receipt.target.clone(), ObserveOptions::default())
            .await?;
        let value = json!({
            "receipt": receipt,
            "observation": computer_observation_summary(&observation),
        });
        Ok(computer_tool_result(
            call_id,
            action_name,
            value,
            Some(observation),
            None,
        ))
    }
}

impl_typed_tool!(ComputerTool);

async fn allowed_computer_windows(
    runtime: &dyn ComputerRuntime,
    session: ComputerSessionId,
    policy: &ComputerAccessPolicy,
) -> anyhow::Result<Vec<WindowTarget>> {
    Ok(runtime
        .list_windows(session)
        .await?
        .into_iter()
        .filter(|target| policy.allows(target))
        .collect())
}

fn ensure_computer_target_allowed(
    policy: &ComputerAccessPolicy,
    target: &WindowTarget,
) -> anyhow::Result<()> {
    if policy.allows(target) {
        Ok(())
    } else {
        anyhow::bail!(
            "desktop application `{}` is not in the Computer Use allowlist",
            target.executable.as_deref().unwrap_or("unknown")
        )
    }
}

fn parse_computer_action(input: ComputerExecutionInput) -> anyhow::Result<ComputerAction> {
    let observation_id = || required_typed_string(input.observation_id.as_deref(), "observationId");
    match input.action {
        ComputerActionInput::Click => Ok(ComputerAction::Click {
            observation_id: observation_id()?,
            x: computer_coordinate(input.x, "x")?,
            y: computer_coordinate(input.y, "y")?,
            button: input.button.into(),
        }),
        ComputerActionInput::Type => Ok(ComputerAction::Type {
            observation_id: observation_id()?,
            text: required_typed_string(input.text.as_deref(), "text")?,
        }),
        ComputerActionInput::Keypress => Ok(ComputerAction::Keypress {
            observation_id: observation_id()?,
            key: input
                .key
                .context("key is required for keypress")?
                .as_str()
                .to_string(),
        }),
        ComputerActionInput::Scroll => Ok(ComputerAction::Scroll {
            observation_id: observation_id()?,
            delta_y: input
                .delta_y
                .context("deltaY must be an integer")?
                .clamp(-12_000, 12_000) as i32,
        }),
        ComputerActionInput::Drag => Ok(ComputerAction::Drag {
            observation_id: observation_id()?,
            start_x: computer_coordinate(input.x, "x")?,
            start_y: computer_coordinate(input.y, "y")?,
            end_x: computer_coordinate(input.end_x, "endX")?,
            end_y: computer_coordinate(input.end_y, "endY")?,
        }),
        ComputerActionInput::Wait => Ok(ComputerAction::Wait {
            observation_id: observation_id()?,
            duration_ms: input.duration_ms.unwrap_or(1_000).clamp(1, 30_000),
        }),
        other => anyhow::bail!(
            "unsupported computer action for an observed window: {}",
            other.as_str()
        ),
    }
}

fn computer_coordinate(value: Option<u64>, field: &str) -> anyhow::Result<u32> {
    value
        .and_then(|value| u32::try_from(value).ok())
        .with_context(|| format!("{field} must be a non-negative integer"))
}

fn computer_observation_summary(observation: &crate::computer::ComputerObservation) -> Value {
    json!({
        "observationId": observation.observation_id,
        "sessionId": observation.session_id,
        "target": observation.target,
        "captureRect": observation.capture_rect,
        "imageWidth": observation.image_width,
        "imageHeight": observation.image_height,
        "unstable": observation.unstable,
        "capturedAt": observation.captured_at,
        "screenshotBytes": observation.screenshot.as_ref().map(|image| image.bytes.len()),
        "accessibilityTreeAvailable": observation.accessibility_tree.is_some(),
    })
}

fn computer_tool_result(
    call_id: Uuid,
    action: String,
    value: Value,
    observation: Option<crate::computer::ComputerObservation>,
    error: Option<String>,
) -> ToolResult {
    let mut content = vec![ModelContentPart::json(value.clone())];
    if let Some(image) = observation.and_then(|observation| observation.screenshot) {
        content.push(ModelContentPart::image(image.mime_type, image.bytes));
    }
    let success = error.is_none();
    let output = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
    ToolResult {
        call_id,
        output,
        content,
        metadata: json!({
            "toolName": "computer",
            "action": action,
            "computer": value,
            "success": success,
            "error": error,
        }),
    }
}
