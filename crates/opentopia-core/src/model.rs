use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub id: Uuid,
    pub title: String,
    pub workspace_root: PathBuf,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Thread {
    pub fn new(title: impl Into<String>, workspace_root: PathBuf) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            title: title.into(),
            workspace_root,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

impl MessageRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }

    pub fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "system" => Ok(Self::System),
            "user" => Ok(Self::User),
            "assistant" => Ok(Self::Assistant),
            "tool" => Ok(Self::Tool),
            other => anyhow::bail!("unknown message role: {other}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: Uuid,
    pub thread_id: Uuid,
    pub role: MessageRole,
    pub parts: Vec<MessagePart>,
    pub created_at: DateTime<Utc>,
}

impl Message {
    pub fn text(thread_id: Uuid, role: MessageRole, text: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            thread_id,
            role,
            parts: vec![MessagePart::Text { text: text.into() }],
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessagePart {
    Text { text: String },
    ToolCall { call: ToolCall },
    ToolResult { result: ToolResult },
    FileRef { path: PathBuf },
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub id: Uuid,
    pub name: String,
    pub input: Value,
}

impl ToolCall {
    pub fn new(name: impl Into<String>, input: Value) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            input,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResult {
    pub call_id: Uuid,
    pub output: String,
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEvent {
    pub id: Uuid,
    pub thread_id: Uuid,
    pub turn_id: Option<Uuid>,
    pub seq: i64,
    pub created_at: DateTime<Utc>,
    pub payload: AgentEventPayload,
}

impl AgentEvent {
    pub fn new(
        thread_id: Uuid,
        turn_id: Option<Uuid>,
        seq: i64,
        payload: AgentEventPayload,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            thread_id,
            turn_id,
            seq,
            created_at: Utc::now(),
            payload,
        }
    }

    pub fn kind(&self) -> &'static str {
        self.payload.kind()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEventPayload {
    TurnStarted { user_message_id: Uuid },
    ModelDelta { text: String },
    ToolCallStarted { call: ToolCall },
    ToolCallFinished { result: ToolResult },
    AssistantMessage { message: Message },
    FileChanged { path: PathBuf, summary: String },
    ApprovalRequested {
        approval_id: Uuid,
        reason: String,
        action: String,
    },
    TurnFinished { summary: String },
    Error { message: String },
}

impl AgentEventPayload {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::TurnStarted { .. } => "turn_started",
            Self::ModelDelta { .. } => "model_delta",
            Self::ToolCallStarted { .. } => "tool_call_started",
            Self::ToolCallFinished { .. } => "tool_call_finished",
            Self::AssistantMessage { .. } => "assistant_message",
            Self::FileChanged { .. } => "file_changed",
            Self::ApprovalRequested { .. } => "approval_requested",
            Self::TurnFinished { .. } => "turn_finished",
            Self::Error { .. } => "error",
        }
    }
}
